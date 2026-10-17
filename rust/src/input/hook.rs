//! 低階鍵盤 hook：專屬執行緒掛 `WH_KEYBOARD_LL`，把每個按鍵事件交給分派器決定吞不吞。

use std::cell::Cell;
use std::sync::{OnceLock, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, PM_NOREMOVE, PeekMessageW,
    PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN,
    WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::input::dispatcher::{KeyEvent, SharedDispatcher};
use crate::input::keys;
use crate::logs::log;

const VK_PACKET: u32 = 0xE7;
const UNINSTALL_TIMEOUT: Duration = Duration::from_secs(1);

static DISPATCHER: OnceLock<SharedDispatcher> = OnceLock::new();

thread_local! {
    static INSIDE_PROCESS: Cell<bool> = const { Cell::new(false) };
}

/// 已安裝的鍵盤 hook；`uninstall` 卸除並收掉執行緒。
pub struct KeyboardHook {
    thread: JoinHandle<()>,
    thread_id: u32,
}

/// 起 `keyboard-hook` 執行緒掛上 hook，每個按鍵事件交給 `d`；一個程序只能裝一次。
pub fn install(d: SharedDispatcher) -> Result<KeyboardHook, String> {
    // 先建好 lazy 表，hook 回呼裡就不會跑初始化
    keys::is_modifier(0);
    let _ = keys::parse_hotkey("ctrl");
    DISPATCHER
        .set(d)
        .map_err(|_| "keyboard hook already installed".to_owned())?;
    let (ready, started) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("keyboard-hook".into())
        .spawn(move || hook_thread(&ready))
        .map_err(|e| format!("cannot spawn keyboard-hook: {e}"))?;
    let thread_id = started
        .recv()
        .map_err(|_| "keyboard-hook exited before reporting".to_owned())??;
    Ok(KeyboardHook { thread, thread_id })
}

impl KeyboardHook {
    /// 結束 hook 執行緒的訊息迴圈並等它卸除，最多等 1 秒。
    pub fn uninstall(self) {
        // SAFETY: 純 Win32 呼叫，thread_id 由 hook 執行緒自己回報。
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        let deadline = Instant::now() + UNINSTALL_TIMEOUT;
        while !self.thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if self.thread.is_finished() {
            let _ = self.thread.join();
        } else {
            log("[hotkey] keyboard hook thread still running after 1.0s; continuing shutdown");
        }
    }
}

fn hook_thread(ready: &mpsc::Sender<Result<u32, String>>) {
    // SAFETY: 以下皆為本執行緒上的 Win32 呼叫，MSG 為本地緩衝。
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        // 先建好訊息佇列，uninstall 的 PostThreadMessageW 才不會丟失
        PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(hook_proc),
            GetModuleHandleW(std::ptr::null()),
            0,
        );
        if hook.is_null() {
            let err = std::io::Error::last_os_error();
            let _ = ready.send(Err(format!("SetWindowsHookExW failed: {err}")));
            return;
        }
        let _ = ready.send(Ok(GetCurrentThreadId()));
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
        UnhookWindowsHookEx(hook);
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: nCode >= 0 時 lParam 指向系統提供的 KBDLLHOOKSTRUCT。
        let k = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if let (Some(ev), Some(d)) = (
            to_event(k.vkCode, k.scanCode, k.flags, wparam as u32),
            DISPATCHER.get(),
        ) {
            let pass = INSIDE_PROCESS.with(|inside| {
                reentrant_guard(inside, || {
                    d.lock().unwrap_or_else(|e| e.into_inner()).process(ev)
                })
            });
            if !pass {
                return 1;
            }
        }
    }
    // SAFETY: 原樣轉交給下一個 hook。
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
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
    Some(KeyEvent { down, code })
}

/// handler 經 `SendInput` 注入的事件可能同步重入 hook；此時分派器的鎖還握著，直接放行以免自鎖。
fn reentrant_guard(inside: &Cell<bool>, process: impl FnOnce() -> bool) -> bool {
    if inside.replace(true) {
        return true;
    }
    let pass = process();
    inside.set(false);
    pass
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

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
                code: 30
            })
        );
    }

    #[test]
    fn zero_scan_code_becomes_negative_vk() {
        assert_eq!(
            to_event(0xE8, 0, 0x10, WM_KEYUP),
            Some(KeyEvent {
                down: false,
                code: -0xE8
            })
        );
    }

    #[test]
    fn syskey_messages_map_like_key_messages() {
        assert_eq!(
            to_event(0x12, 56, 0x20, WM_SYSKEYDOWN),
            Some(KeyEvent {
                down: true,
                code: 56
            })
        );
        assert_eq!(
            to_event(0x12, 56, 0xA0, WM_SYSKEYUP),
            Some(KeyEvent {
                down: false,
                code: 56
            })
        );
        assert_eq!(to_event(0x41, 30, 0, 0x0100 + 7), None);
    }

    #[test]
    fn reentrant_calls_pass_without_processing() {
        let inside = Cell::new(false);
        let swallowed = reentrant_guard(&inside, || {
            assert!(reentrant_guard(&inside, || unreachable!()));
            false
        });
        assert!(!swallowed);
        assert!(!inside.get(), "the flag is cleared after processing");
        assert!(!reentrant_guard(&inside, || false));
    }
}
