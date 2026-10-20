//! 低階鍵盤 hook：專屬執行緒掛 `WH_KEYBOARD_LL`，把每個按鍵事件交給分派器決定吞不吞。

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{OnceLock, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, MSG,
    PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_LCONTROL, VK_RMENU};

use crate::input::dispatcher::{KeyEvent, RawKey, SharedDispatcher};
use crate::input::keys;
use crate::logs::log;
use crate::util::{Clock, join_until, lock, panic_message};

const VK_PACKET: u32 = 0xE7;
const UNINSTALL_TIMEOUT: Duration = Duration::from_secs(1);
/// 掛 hook 的嘗試次數與失敗後的間隔：開機自動啟動時桌面可能還沒就緒。
const INSTALL_ATTEMPTS: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(2);
/// 回呼超過這個時間就重掛 hook：Windows 會默默移除超過 `LowLevelHooksTimeout` 的 hook。
const SLOW_CALLBACK: Duration = Duration::from_millis(200);
/// 回呼太慢時寄給 hook 執行緒自己、要它重掛 hook 的訊息。
const REINSTALL_HOOK: u32 = WM_APP;

static DISPATCHER: OnceLock<SharedDispatcher> = OnceLock::new();
static FIRST_EVENT_SEEN: AtomicBool = AtomicBool::new(false);
static HOOK_CALLS: AtomicU64 = AtomicU64::new(0);

/// hook 回呼被系統呼叫的總次數（含 `code < 0`）；與分派器的計數比對，分得出事件是沒進 hook 還是進了沒分派。
pub fn hook_calls() -> u64 {
    HOOK_CALLS.load(Ordering::Relaxed)
}

thread_local! {
    static INSIDE_PROCESS: Cell<bool> = const { Cell::new(false) };
    static IGNORE_NEXT_RIGHT_ALT: Cell<bool> = const { Cell::new(false) };
    static DEFERRED: RefCell<Vec<Box<dyn FnOnce()>>> = const { RefCell::new(Vec::new()) };
}

/// 分派中（hook 執行緒握著分派器的鎖）就把 `f` 延到放鎖後才跑，否則立刻跑。
///
/// 給 handler 裡的 `SendInput` 用：注入的事件可能同步重入 hook，握著鎖時只能放行不處理，會漏記按鍵放開。
pub fn after_dispatch(f: impl FnOnce() + 'static) {
    if INSIDE_PROCESS.with(Cell::get) {
        DEFERRED.with(|queue| queue.borrow_mut().push(Box::new(f)));
    } else {
        f();
    }
}

/// 已安裝的鍵盤 hook；`uninstall` 卸除並收掉執行緒。
pub struct KeyboardHook {
    thread: JoinHandle<()>,
    thread_id: u32,
}

/// 起 `keyboard-hook` 執行緒掛上 hook，每個按鍵事件交給 `d`；一個程序只能裝一次。
///
/// 掛 hook 在該執行緒上進行、失敗會重試，不擋呼叫端；全部失敗時呼叫 `on_failure`。
pub fn install(
    d: SharedDispatcher,
    on_failure: impl FnOnce() + Send + 'static,
) -> Result<KeyboardHook, String> {
    // 先建好 lazy 表，hook 回呼裡就不會跑初始化
    keys::is_modifier(0);
    let _ = keys::parse_hotkey("ctrl");
    DISPATCHER
        .set(d)
        .map_err(|_| "keyboard hook already installed".to_owned())?;
    let (ready, started) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name(crate::logs::KEYBOARD_HOOK_THREAD.into())
        .spawn(move || hook_thread(&ready, on_failure))
        .map_err(|e| format!("cannot spawn keyboard-hook: {e}"))?;
    let thread_id = started
        .recv()
        .map_err(|_| "keyboard-hook exited before reporting".to_owned())?;
    Ok(KeyboardHook { thread, thread_id })
}

/// 呼叫 `attempt` 最多 `tries` 次，失敗之間以 `sleep` 等 `RETRY_DELAY`；回傳第一次成功或最後一次的錯誤。
fn retry<T>(
    tries: u32,
    sleep: impl Fn(Duration),
    mut attempt: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    let mut n = 1;
    loop {
        match attempt() {
            Ok(value) => return Ok(value),
            Err(e) if n >= tries => return Err(e),
            Err(e) => {
                log(format!(
                    "[hotkey] keyboard hook attempt {n}/{tries} failed: {e}; retrying in {}s",
                    RETRY_DELAY.as_secs()
                ));
                sleep(RETRY_DELAY);
                n += 1;
            }
        }
    }
}

impl KeyboardHook {
    /// 結束 hook 執行緒的訊息迴圈並等它卸除，最多等 1 秒。
    pub fn uninstall(self) {
        // SAFETY: 純 Win32 呼叫，thread_id 由 hook 執行緒自己回報。
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        let deadline = Instant::now() + UNINSTALL_TIMEOUT;
        if !join_until(self.thread, deadline, &Clock::default()) {
            log(format!(
                "[hotkey] keyboard hook thread still running after {:.1}s; continuing shutdown",
                UNINSTALL_TIMEOUT.as_secs_f64()
            ));
        }
    }
}

fn hook_thread(ready: &mpsc::Sender<u32>, on_failure: impl FnOnce()) {
    let mut on_failure = Some(on_failure);
    let mut install = || match retry(INSTALL_ATTEMPTS, std::thread::sleep, set_hook) {
        Ok(hook) => Some(hook),
        Err(e) => {
            log(format!(
                "[hotkey] cannot install keyboard hook after {INSTALL_ATTEMPTS} attempts: {e}"
            ));
            if let Some(f) = on_failure.take() {
                f();
            }
            None
        }
    };
    // SAFETY: 以下皆為本執行緒上的 Win32 呼叫，MSG 為本地緩衝。
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        // 先建好訊息佇列，uninstall 的 PostThreadMessageW 才不會丟失（重試期間送來的 WM_QUIT 也會留著）
        PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
        let thread_id = GetCurrentThreadId();
        let _ = ready.send(thread_id);
        let Some(mut hook) = install() else { return };
        log(format!(
            "[hotkey] keyboard hook installed (thread_id={thread_id})"
        ));
        let ended = loop {
            let got = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if got <= 0 {
                break got;
            }
            if msg.message == REINSTALL_HOOK {
                // 系統可能已經移除它，卸除失敗不用管
                UnhookWindowsHookEx(hook);
                let Some(new) = install() else { return };
                hook = new;
                log("[hotkey] keyboard hook reinstalled after a slow callback");
            }
        };
        log(format!(
            "[hotkey] keyboard hook message loop ended (GetMessageW returned {ended}); unhooking"
        ));
        UnhookWindowsHookEx(hook);
    }
}

fn set_hook() -> Result<HHOOK, String> {
    // SAFETY: 純 Win32 呼叫，hook_proc 符合 HOOKPROC 簽名。
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(hook_proc),
            GetModuleHandleW(std::ptr::null()),
            0,
        )
    };
    if hook.is_null() {
        let err = std::io::Error::last_os_error();
        return Err(format!("SetWindowsHookExW failed: {err}"));
    }
    Ok(hook)
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    HOOK_CALLS.fetch_add(1, Ordering::Relaxed);
    if code >= 0 {
        let started = Instant::now();
        if !FIRST_EVENT_SEEN.swap(true, Ordering::Relaxed) {
            log("[hotkey] keyboard hook active (first key event seen)");
        }
        // SAFETY: nCode >= 0 時 lParam 指向系統提供的 KBDLLHOOKSTRUCT。
        let k = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        // panic 不可穿過 extern "system"（會直接中止程序、跳過關閉流程），出錯就放行
        let pass = catch_unwind(AssertUnwindSafe(|| should_pass(k, wparam as u32))).unwrap_or_else(
            |payload| {
                log(format!(
                    "[hotkey] error in keyboard hook: {}",
                    panic_message(payload.as_ref())
                ));
                true
            },
        );
        let took = started.elapsed();
        if took >= SLOW_CALLBACK {
            log(format!(
                "[hotkey] slow keyboard hook callback: {} ms (vk={:#x}); reinstalling the hook in case Windows removed it for exceeding LowLevelHooksTimeout",
                took.as_millis(),
                k.vkCode
            ));
            // SAFETY: 純 Win32 呼叫；hook 回呼跑在 hook 執行緒上，訊息寄給它自己的佇列。
            unsafe { PostThreadMessageW(GetCurrentThreadId(), REINSTALL_HOOK, 0, 0) };
        }
        if !pass {
            return 1;
        }
    }
    // SAFETY: 原樣轉交給下一個 hook。
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

fn should_pass(k: &KBDLLHOOKSTRUCT, msg: u32) -> bool {
    let (Some(ev), Some(d)) = (
        to_event(k.vkCode, k.scanCode, k.flags, msg),
        DISPATCHER.get(),
    ) else {
        return true;
    };
    if IGNORE_NEXT_RIGHT_ALT.with(|ignore| is_alt_gr_echo(ignore, k.vkCode, k.scanCode)) {
        return true;
    }
    let pass = INSIDE_PROCESS.with(|inside| {
        reentrant_guard(inside, ev, || {
            let mut d = lock(d);
            if ev.down {
                d.drop_released_keys(ev.code, any_key_down);
            }
            d.process(ev)
        })
    });
    run_deferred();
    pass
}

/// 以非同步按鍵狀態查 `vks` 是否有任一按著。
fn any_key_down(vks: &[u16]) -> bool {
    // SAFETY: 純 Win32 查詢；最高位元為 1 表示按著。
    vks.iter()
        .any(|&vk| unsafe { GetAsyncKeyState(i32::from(vk)) } < 0)
}

fn run_deferred() {
    for f in DEFERRED.with(|queue| std::mem::take(&mut *queue.borrow_mut())) {
        f();
    }
}

/// 把 hook 收到的原始欄位轉成按鍵事件；Unicode 注入、假 alt 與未知訊息回 `None`（照常放行）。
pub fn to_event(vk: u32, scan: u32, flags: u32, msg: u32) -> Option<KeyEvent> {
    // 有時會多出一個帶 0x20 的注入 alt down（`_winkeyboard` 的 fake_alt）
    let fake_alt = LLKHF_INJECTED | 0x20;
    if vk == VK_PACKET || flags & fake_alt == fake_alt {
        return None;
    }
    let down = match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => true,
        WM_KEYUP | WM_SYSKEYUP => false,
        _ => return None,
    };
    let code = if scan == 0 { -(vk as i32) } else { scan as i32 };
    Some(KeyEvent {
        down,
        code,
        raw: Some(RawKey {
            vk,
            extended: flags & LLKHF_EXTENDED != 0,
        }),
    })
}

/// handler 經 `SendInput` 注入的事件可能同步重入 hook；此時分派器的鎖還握著，直接放行以免自鎖。
fn reentrant_guard(inside: &Cell<bool>, ev: KeyEvent, process: impl FnOnce() -> bool) -> bool {
    struct Reset<'a>(&'a Cell<bool>);
    impl Drop for Reset<'_> {
        fn drop(&mut self) {
            self.0.set(false);
        }
    }
    if inside.replace(true) {
        log(format!(
            "[hotkey] re-entrant key event passed through unprocessed (code={}, down={})",
            ev.code, ev.down
        ));
        return true;
    }
    let _reset = Reset(inside);
    process()
}

/// Alt Gr 送出 scan 541 的 left ctrl 後會多一個 right alt；那個 right alt 放行但不分派（`_winkeyboard.process_key`）。
fn is_alt_gr_echo(ignore_next_right_alt: &Cell<bool>, vk: u32, scan: u32) -> bool {
    if vk == u32::from(VK_RMENU) && ignore_next_right_alt.replace(false) {
        return true;
    }
    if scan == keys::ALT_GR as u32 && vk == u32::from(VK_LCONTROL) {
        ignore_next_right_alt.set(true);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vk_packet_events_are_ignored() {
        assert_eq!(to_event(0xE7, 0, 0, WM_KEYDOWN), None);
    }

    #[test]
    fn injected_alt_down_events_are_ignored() {
        assert_eq!(to_event(0x12, 56, 0x30, WM_SYSKEYDOWN), None);
        assert_eq!(
            to_event(0x41, 30, 0x10, WM_KEYDOWN),
            Some(KeyEvent {
                down: true,
                code: 30,
                raw: Some(RawKey {
                    vk: 0x41,
                    extended: false
                })
            })
        );
    }

    #[test]
    fn zero_scan_code_becomes_negative_vk() {
        assert_eq!(
            to_event(0xE8, 0, 0x10, WM_KEYUP),
            Some(KeyEvent {
                down: false,
                code: -0xE8,
                raw: Some(RawKey {
                    vk: 0xE8,
                    extended: false
                })
            })
        );
    }

    #[test]
    fn syskey_messages_map_like_key_messages() {
        assert_eq!(
            to_event(0x12, 56, 0x20, WM_SYSKEYDOWN),
            Some(KeyEvent {
                down: true,
                code: 56,
                raw: Some(RawKey {
                    vk: 0x12,
                    extended: false
                })
            })
        );
        assert_eq!(
            to_event(0x12, 56, 0xA0, WM_SYSKEYUP),
            Some(KeyEvent {
                down: false,
                code: 56,
                raw: Some(RawKey {
                    vk: 0x12,
                    extended: false
                })
            })
        );
        assert_eq!(to_event(0x41, 30, 0, 0x0100 + 7), None);
    }

    #[test]
    fn retry_stops_at_the_first_success_and_sleeps_between_attempts() {
        let slept = Cell::new(0);
        let mut calls = 0;
        let result = retry(
            3,
            |d| {
                assert_eq!(d, RETRY_DELAY);
                slept.set(slept.get() + 1);
            },
            || {
                calls += 1;
                if calls < 3 {
                    Err("busy".into())
                } else {
                    Ok(calls)
                }
            },
        );
        assert_eq!(result, Ok(3));
        assert_eq!(slept.get(), 2);
    }

    #[test]
    fn retry_returns_the_last_error_after_the_last_attempt() {
        let slept = Cell::new(0);
        let mut calls = 0;
        let result: Result<(), String> = retry(
            3,
            |_| slept.set(slept.get() + 1),
            || {
                calls += 1;
                Err(format!("fail {calls}"))
            },
        );
        assert_eq!(result, Err("fail 3".into()));
        assert_eq!(slept.get(), 2, "no wait after the last attempt");
    }

    const SPACE: KeyEvent = KeyEvent {
        down: false,
        code: 57,
        raw: None,
    };

    #[test]
    fn work_queued_during_dispatch_runs_after_the_lock_is_released() {
        use std::rc::Rc;
        let ran = Rc::new(Cell::new(0));
        let (during, after) = (Rc::clone(&ran), Rc::clone(&ran));
        INSIDE_PROCESS.with(|inside| {
            reentrant_guard(inside, SPACE, || {
                after_dispatch(move || during.set(during.get() + 1));
                assert_eq!(ran.get(), 0, "deferred while dispatching");
                true
            })
        });
        run_deferred();
        assert_eq!(ran.get(), 1);
        after_dispatch(move || after.set(after.get() + 1));
        assert_eq!(ran.get(), 2, "runs at once outside dispatch");
    }

    #[test]
    fn reentrant_calls_pass_without_processing() {
        let inside = Cell::new(false);
        let swallowed = reentrant_guard(&inside, SPACE, || {
            assert!(reentrant_guard(&inside, SPACE, || unreachable!()));
            false
        });
        assert!(!swallowed);
        assert!(!inside.get(), "the flag is cleared after processing");
        assert!(!reentrant_guard(&inside, SPACE, || false));
    }

    #[test]
    fn guard_flag_is_cleared_when_processing_panics() {
        let inside = Cell::new(false);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            reentrant_guard(&inside, SPACE, || std::panic::resume_unwind(Box::new(())))
        }));
        assert!(unwound.is_err());
        assert!(!inside.get());
    }

    #[test]
    fn right_alt_after_alt_gr_ctrl_is_passed_without_dispatching() {
        let ignore = Cell::new(false);
        assert!(
            !is_alt_gr_echo(&ignore, 0xA2, 541),
            "the alt gr ctrl itself is dispatched"
        );
        assert!(is_alt_gr_echo(&ignore, 0xA5, 56));
        assert!(
            !is_alt_gr_echo(&ignore, 0xA5, 56),
            "only the next right alt is skipped"
        );
        assert!(
            !is_alt_gr_echo(&ignore, 0xA2, 29),
            "a plain left ctrl does not arm it"
        );
        assert!(!is_alt_gr_echo(&ignore, 0xA5, 56));
    }
}
