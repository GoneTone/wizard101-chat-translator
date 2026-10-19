//! 把文字送進遊戲：「逐字鍵入」（模擬打字）。遊戲不支援剪貼簿貼上，故改用自動輸入。
//!
//! 兩條路徑共用同一個鍵入迴圈但規矩不同：譯文（`type_into_window`）絕不模擬 Enter，由使用者
//! 自己確認後送出；攔截到的 Ctrl+V（`paste_clipboard`）行為等同原生貼上，剪貼簿內容原樣送。

use std::sync::{Arc, LazyLock, Mutex, TryLockError};
use std::time::{Duration, Instant};

use regex::Regex;

use crate::config::py::py_bool;
use crate::input::dispatcher::{Pressed, is_pressed};
use crate::input::hotkeys::HotkeyInterceptor;
use crate::input::inject::{Injector, SendInputInjector};
use crate::input::keys::{Code, key_codes_or_empty};
use crate::logs::log;
use crate::reader::process::{foreground_window, window_exists};
use crate::ui::{clipboard, win32};
use crate::util::lock;

/// 切回遊戲視窗後、開始打字前的緩衝。
pub const FOCUS_DELAY: Duration = Duration::from_millis(150);
/// 與 Ctrl 合按時被攔截的鍵。
pub const PASTE_KEY: &str = "v";
/// 貼上前等使用者放開 Ctrl／V 的上限。
pub const MODIFIER_RELEASE_TIMEOUT: Duration = Duration::from_secs(3);
/// 等待放開時的輪詢間隔。
pub const KEY_RELEASE_POLL: Duration = Duration::from_millis(10);

/// 譯文與貼上兩條路共用：鍵入進行中就持有，也讓兩者不交錯。
static TYPING: Mutex<()> = Mutex::new(());
static PASTE_IN_PROGRESS: Mutex<()> = Mutex::new(());

static LINE_BREAKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\r\n]+").unwrap());

/// 目前是否正在逐字鍵入（譯文或貼上）。
pub fn typing_in_progress() -> bool {
    matches!(TYPING.try_lock(), Err(TryLockError::WouldBlock))
}

/// 鍵入用到的所有 Win32 動作；測試換成假的，不碰桌面。
pub struct TypingEnv {
    pub inject: Arc<dyn Injector>,
    pub foreground: fn() -> isize,
    pub is_window: fn(isize) -> bool,
    pub force_foreground: fn(isize) -> bool,
    pub sleep: fn(Duration),
    pub now: fn() -> Instant,
    pub pressed: Arc<dyn Fn() -> Pressed + Send + Sync>,
    pub clipboard: fn() -> Result<Option<String>, String>,
}

impl TypingEnv {
    /// 接真的 Win32；`pressed` 回報目前按著的鍵。
    pub fn real(pressed: Arc<dyn Fn() -> Pressed + Send + Sync>) -> Self {
        Self {
            inject: Arc::new(SendInputInjector),
            foreground: foreground_window,
            is_window: window_exists,
            force_foreground: win32::force_foreground,
            sleep: std::thread::sleep,
            now: Instant::now,
            pressed,
            clipboard: clipboard::get_text,
        }
    }
}

/// 逐字鍵入 `text`，`delay` 為每字間隔。
///
/// `hwnd` 存在時每個字送出前都確認它仍是前景：打到一半切窗（或搶焦點失敗）就停，
/// 剩下的字不會打進別的視窗；`hwnd` 為 0 則打到底。
fn type_chars(env: &TypingEnv, hwnd: isize, text: &str, delay: Duration) {
    let guarded = hwnd != 0 && (env.is_window)(hwnd);
    let (enter, backspace) = (key_code("enter"), key_code("backspace"));
    let total = text.chars().count();
    let _typing = lock(&TYPING);
    for (index, ch) in text.chars().enumerate() {
        if guarded && (env.foreground)() != hwnd {
            log(format!(
                "[composer] typing aborted: target lost foreground after {index}/{total} chars (hwnd={hwnd:#x})"
            ));
            return;
        }
        // 不還原修飾鍵：還原會讓之後每個字都帶著使用者按著的 Ctrl 送出（Enter 變 Ctrl+Enter）
        for code in (env.pressed)() {
            env.inject.release(code);
        }
        match ch {
            '\n' => tap(env, enter),
            '\u{8}' => tap(env, backspace),
            _ => env.inject.unicode(ch),
        }
        if !delay.is_zero() {
            (env.sleep)(delay);
        }
    }
}

fn key_code(name: &str) -> Code {
    key_codes_or_empty(name)[0]
}

fn tap(env: &TypingEnv, code: Code) {
    env.inject.press(code);
    env.inject.release(code);
}

/// 把譯文送進遊戲：還原前景視窗（`hwnd`）後逐字鍵入 `text`；遊戲漏字就把 `delay` 調大。
pub fn type_into_window(env: &TypingEnv, hwnd: isize, text: &str, delay: Duration) {
    // 絕不送 Enter 的最後防線：模型偶發的多行輸出一律壓成空格
    let text = LINE_BREAKS.replace_all(text, " ");
    let text = text.trim();
    let target_alive = hwnd != 0 && (env.is_window)(hwnd);
    log(format!(
        "[composer] typing chars={} delay={:?} hwnd={hwnd:#x} target_alive={}",
        text.chars().count(),
        delay.as_secs_f64(),
        py_bool(target_alive)
    ));
    if target_alive {
        if !(env.force_foreground)(hwnd) {
            log(format!(
                "[composer] force_foreground failed (hwnd={hwnd:#x})"
            ));
        }
        (env.sleep)(FOCUS_DELAY);
    }
    type_chars(env, hwnd, text, delay);
}

/// Ctrl+V 的攔截器：`should_intercept()` 成立（遊戲在前景、設定開啟）才吞掉並呼叫 `on_paste`。
///
/// 否則原樣放行、不留 log —— 其他視窗的 Ctrl+V 很頻繁。`on_paste` 跑在低階 hook 裡，
/// 只能把工作丟給別的執行緒。
pub fn paste_interceptor(
    should_intercept: Box<dyn Fn() -> bool + Send>,
    mut on_paste: Box<dyn FnMut() + Send>,
    mask: Box<dyn Fn() + Send>,
) -> HotkeyInterceptor {
    let trigger = Box::new(move || {
        if !should_intercept() {
            return false;
        }
        on_paste();
        true
    });
    HotkeyInterceptor::new(&format!("ctrl+{PASTE_KEY}"), trigger, mask)
        .expect("paste hotkey names are valid")
        .expect("ctrl+v can be swallowed")
        // 鍵入進行中每個字都會先把使用者按著的 Ctrl 放開，此時再按 Ctrl+V 查不到 Ctrl；
        // 一律吞掉，否則 v 會插進打到一半的訊息
        .swallow_override(Box::new(typing_in_progress))
}

/// 等使用者把 Ctrl 與 V 都放開再開始打字，一直按著不放就等到上限照打。
///
/// Ctrl 還按著時送出的 Enter 是 Ctrl+Enter，聊天框不會送出。
fn wait_for_paste_keys_released(env: &TypingEnv) {
    let (ctrl, paste) = (key_codes_or_empty("ctrl"), key_codes_or_empty(PASTE_KEY));
    let deadline = (env.now)() + MODIFIER_RELEASE_TIMEOUT;
    loop {
        let pressed = (env.pressed)();
        if !is_pressed(&pressed, &ctrl) && !is_pressed(&pressed, &paste) {
            return;
        }
        if (env.now)() >= deadline {
            log(format!(
                "[composer] ctrl/{PASTE_KEY} still held after {:?}s; typing anyway",
                MODIFIER_RELEASE_TIMEOUT.as_secs_f64()
            ));
            return;
        }
        (env.sleep)(KEY_RELEASE_POLL);
    }
}

/// 攔到 Ctrl+V：把剪貼簿文字鍵入 `hwnd`（遊戲已在前景，不搶焦點），回傳有沒有真的開始打。
///
/// 行為要跟原生貼上一樣 —— 空白照送、內容不 strip；CRLF 折成一個換行。`single_line`
/// （聊天輸入框開著）時每個換行改成一個空格，因為那裡的 Enter 會把打到一半的內容送出。
/// 空剪貼簿不打；上一次還在打的時候再呼叫一律忽略，免得兩串字交錯。
pub fn paste_clipboard(env: &TypingEnv, hwnd: isize, delay: Duration, single_line: bool) -> bool {
    let _paste = match PASTE_IN_PROGRESS.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(e)) => e.into_inner(),
        Err(TryLockError::WouldBlock) => {
            log("[composer] paste ignored: previous paste still typing");
            return false;
        }
    };
    let text = match (env.clipboard)() {
        Ok(Some(text)) if !text.is_empty() => text,
        result => {
            match result {
                Err(err) => log(format!("[composer] clipboard open failed: {err}")),
                Ok(None) => log("[composer] clipboard has no text: no CF_UNICODETEXT data"),
                Ok(Some(_)) => {}
            }
            log("[composer] paste skipped: clipboard is empty or not text");
            return false;
        }
    };
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = if single_line {
        text.replace('\n', " ")
    } else {
        text
    };
    log(format!(
        "[composer] pasting clipboard chars={} delay={:?} single_line={} hwnd={hwnd:#x}",
        text.chars().count(),
        delay.as_secs_f64(),
        py_bool(single_line)
    ));
    wait_for_paste_keys_released(env);
    type_chars(env, hwnd, &text, delay);
    true
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::sync::MutexGuard;

    use super::*;
    use crate::input::dispatcher::{Blocking, KeyEvent};
    use crate::input::inject::encode_units;
    use crate::input::keys::key_to_scan_codes;

    #[derive(Debug, PartialEq, Clone)]
    enum Call {
        Press(Code),
        Release(Code),
        Unicode(char),
    }

    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<Call>>,
        typing_seen: Mutex<Vec<bool>>,
        paste_locked: Mutex<Vec<bool>>,
    }

    impl Injector for Fake {
        fn press(&self, code: Code) {
            self.calls.lock().unwrap().push(Call::Press(code));
        }
        fn release(&self, code: Code) {
            self.calls.lock().unwrap().push(Call::Release(code));
        }
        fn unicode(&self, ch: char) {
            self.typing_seen.lock().unwrap().push(typing_in_progress());
            self.paste_locked
                .lock()
                .unwrap()
                .push(PASTE_IN_PROGRESS.try_lock().is_err());
            self.calls.lock().unwrap().push(Call::Unicode(ch));
        }
        fn mask(&self) {}
    }

    thread_local! {
        static FG: RefCell<VecDeque<isize>> = const { RefCell::new(VecDeque::new()) };
        static SLEPT: RefCell<Vec<Duration>> = const { RefCell::new(Vec::new()) };
        static FORCED: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
        static CLOCK: Cell<Duration> = const { Cell::new(Duration::ZERO) };
        static CLIP: RefCell<Result<Option<String>, String>> = const { RefCell::new(Ok(None)) };
        static BASE: Instant = Instant::now();
    }

    /// 前景序列：用完後固定回最後一個。
    fn set_foreground(seq: &[isize]) {
        FG.with(|fg| *fg.borrow_mut() = seq.iter().copied().collect());
    }

    fn set_clipboard(clip: Result<Option<&str>, &str>) {
        CLIP.with(|c| {
            *c.borrow_mut() = clip.map(|t| t.map(String::from)).map_err(String::from);
        });
    }

    fn slept() -> Vec<Duration> {
        SLEPT.with(|s| s.borrow().clone())
    }

    fn fake_env(fake: &Arc<Fake>) -> TypingEnv {
        TypingEnv {
            inject: fake.clone(),
            foreground: || {
                FG.with(|fg| {
                    let mut fg = fg.borrow_mut();
                    if fg.len() > 1 {
                        fg.pop_front().unwrap()
                    } else {
                        fg.front().copied().unwrap_or(0)
                    }
                })
            },
            is_window: |_| true,
            force_foreground: |hwnd| {
                FORCED.with(|f| f.borrow_mut().push(hwnd));
                true
            },
            sleep: |d| {
                SLEPT.with(|s| s.borrow_mut().push(d));
                CLOCK.with(|c| c.set(c.get() + d));
            },
            now: || BASE.with(|b| *b) + CLOCK.with(Cell::get),
            pressed: Arc::new(Pressed::new),
            clipboard: || CLIP.with(|c| c.borrow().clone()),
        }
    }

    /// TYPING／PASTE_IN_PROGRESS 是全域的，碰到它們的測試一律排隊跑。
    fn serial() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        lock(&LOCK)
    }

    fn typed(fake: &Fake) -> String {
        let (enter, backspace) = (code("enter"), code("backspace"));
        fake.calls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|c| match c {
                Call::Unicode(ch) => Some(*ch),
                Call::Press(k) if *k == enter => Some('\n'),
                Call::Press(k) if *k == backspace => Some('\u{8}'),
                _ => None,
            })
            .collect()
    }

    fn code(name: &str) -> Code {
        key_to_scan_codes(name).unwrap()[0]
    }

    fn setup() -> (Arc<Fake>, TypingEnv) {
        set_foreground(&[0]);
        set_clipboard(Ok(None));
        SLEPT.with(|s| s.borrow_mut().clear());
        FORCED.with(|f| f.borrow_mut().clear());
        CLOCK.with(|c| c.set(Duration::ZERO));
        let fake = Arc::new(Fake::default());
        let env = fake_env(&fake);
        (fake, env)
    }

    #[test]
    fn typing_never_re_presses_the_modifiers_the_user_held() {
        let _g = serial();
        let (fake, mut env) = setup();
        let held: Pressed = [code("ctrl"), code("shift")].into();
        env.pressed = Arc::new(move || held.clone());
        type_into_window(&env, 0, "ab", Duration::ZERO);
        let (a, b) = (
            code("ctrl").min(code("shift")),
            code("ctrl").max(code("shift")),
        );
        let released = [Call::Release(a), Call::Release(b)];
        let expected: Vec<Call> = released
            .iter()
            .cloned()
            .chain([Call::Unicode('a')])
            .chain(released.iter().cloned())
            .chain([Call::Unicode('b')])
            .collect();
        assert_eq!(*fake.calls.lock().unwrap(), expected);
    }

    #[test]
    fn type_into_window_types_text_one_char_at_a_time() {
        let _g = serial();
        let (fake, env) = setup();
        let delay = Duration::from_millis(30);
        type_into_window(&env, 0, "wanna team up?", delay);
        assert_eq!(typed(&fake), "wanna team up?");
        assert_eq!(slept(), vec![delay; 14]);
    }

    #[test]
    fn type_into_window_flattens_newlines() {
        let _g = serial();
        let (fake, env) = setup();
        type_into_window(
            &env,
            0,
            "line one\nline two\r\nline three\n",
            Duration::ZERO,
        );
        assert_eq!(typed(&fake), "line one line two line three");
    }

    #[test]
    fn type_into_window_stops_when_foreground_leaves_target() {
        let _g = serial();
        let (fake, env) = setup();
        let target = 0x1234;
        set_foreground(&[target, target, target, 0x9999, target]);
        type_into_window(&env, target, "hello", Duration::ZERO);
        assert_eq!(typed(&fake), "hel");
        assert_eq!(FORCED.with(|f| f.borrow().clone()), [target]);
        assert_eq!(slept(), [FOCUS_DELAY]);
    }

    #[test]
    fn type_into_window_never_sends_enter() {
        let _g = serial();
        let (fake, env) = setup();
        type_into_window(&env, 0, "hello\nworld", Duration::ZERO);
        assert!(
            !fake
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| matches!(c, Call::Press(_)))
        );
    }

    fn down(code: Code) -> KeyEvent {
        KeyEvent {
            down: true,
            code,
            raw: None,
        }
    }

    fn up(code: Code) -> KeyEvent {
        KeyEvent {
            down: false,
            code,
            raw: None,
        }
    }

    fn interceptor(intercept: bool) -> (HotkeyInterceptor, Arc<Mutex<u32>>) {
        let pastes = Arc::new(Mutex::new(0));
        let count = pastes.clone();
        let it = paste_interceptor(
            Box::new(move || intercept),
            Box::new(move || *count.lock().unwrap() += 1),
            Box::new(|| {}),
        );
        (it, pastes)
    }

    fn held(names: &[&str]) -> Pressed {
        names.iter().map(|n| code(n)).collect()
    }

    #[test]
    fn interceptor_passes_plain_v_through() {
        let _g = serial();
        let (mut it, pastes) = interceptor(true);
        assert!(it.handle(down(code("v")), &held(&[])));
        assert!(it.handle(up(code("v")), &held(&[])));
        assert_eq!(*pastes.lock().unwrap(), 0);
    }

    #[test]
    fn interceptor_passes_ctrl_v_through_outside_the_game() {
        let _g = serial();
        let (mut it, pastes) = interceptor(false);
        assert!(it.handle(down(code("v")), &held(&["ctrl"])));
        assert!(it.handle(up(code("v")), &held(&["ctrl"])));
        assert_eq!(*pastes.lock().unwrap(), 0);
    }

    #[test]
    fn interceptor_swallows_ctrl_v_in_the_game_and_pastes_once() {
        let _g = serial();
        let (mut it, pastes) = interceptor(true);
        assert!(!it.handle(down(code("v")), &held(&["ctrl"])));
        assert!(!it.handle(up(code("v")), &held(&["ctrl"])));
        assert_eq!(*pastes.lock().unwrap(), 1);
    }

    #[test]
    fn interceptor_ignores_other_keys() {
        let _g = serial();
        let (mut it, pastes) = interceptor(true);
        assert!(it.handle(down(code("shift")), &held(&["ctrl"])));
        assert_eq!(*pastes.lock().unwrap(), 0);
    }

    #[test]
    fn interceptor_ignores_key_repeat_until_released() {
        let _g = serial();
        let (mut it, pastes) = interceptor(true);
        for _ in 0..3 {
            assert!(!it.handle(down(code("v")), &held(&["ctrl"])));
        }
        assert_eq!(*pastes.lock().unwrap(), 1);
        it.handle(up(code("v")), &held(&["ctrl"]));
        it.handle(down(code("v")), &held(&["ctrl"]));
        assert_eq!(*pastes.lock().unwrap(), 2);
    }

    #[test]
    fn interceptor_keeps_swallowing_v_until_released_even_after_ctrl_is_up() {
        let _g = serial();
        let (mut it, pastes) = interceptor(true);
        assert!(!it.handle(down(code("v")), &held(&["ctrl"])));
        assert!(!it.handle(down(code("v")), &held(&[])));
        assert!(!it.handle(up(code("v")), &held(&[])));
        assert_eq!(*pastes.lock().unwrap(), 1);
        assert!(it.handle(down(code("v")), &held(&[])));
    }

    #[test]
    fn interceptor_swallows_v_while_typing_is_in_progress() {
        let _g = serial();
        let (mut it, pastes) = interceptor(true);
        let _typing = TYPING.lock().unwrap();
        assert!(!it.handle(down(code("v")), &held(&[])));
        assert!(!it.handle(up(code("v")), &held(&[])));
        assert_eq!(*pastes.lock().unwrap(), 0);
    }

    #[test]
    fn paste_clipboard_reads_unicode_text() {
        let _g = serial();
        let (fake, env) = setup();
        set_clipboard(Ok(Some("來組隊嗎？")));
        assert!(paste_clipboard(&env, 0, Duration::ZERO, false));
        assert_eq!(typed(&fake), "來組隊嗎？");
    }

    #[test]
    fn paste_clipboard_skips_unreadable_clipboard() {
        let _g = serial();
        let (fake, env) = setup();
        set_clipboard(Ok(None));
        assert!(!paste_clipboard(&env, 0, Duration::ZERO, false));
        set_clipboard(Err("error=5"));
        let text = crate::logs::capture(|| {
            assert!(!paste_clipboard(&env, 0, Duration::ZERO, false));
        });
        assert!(
            text.contains("[composer] clipboard open failed: error=5"),
            "{text}"
        );
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn paste_clipboard_skips_empty_clipboard() {
        let _g = serial();
        let (fake, env) = setup();
        set_clipboard(Ok(Some("")));
        assert!(!paste_clipboard(&env, 0, Duration::ZERO, false));
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn paste_clipboard_types_the_clipboard_with_the_configured_delay() {
        let _g = serial();
        let (fake, env) = setup();
        set_foreground(&[0x42]);
        set_clipboard(Ok(Some("hello there")));
        let delay = Duration::from_millis(50);
        assert!(paste_clipboard(&env, 0x42, delay, false));
        assert_eq!(typed(&fake), "hello there");
        assert_eq!(slept(), vec![delay; 11]);
    }

    #[test]
    fn paste_clipboard_types_the_text_verbatim_like_native_paste() {
        let _g = serial();
        let (fake, env) = setup();
        set_foreground(&[0x42]);
        set_clipboard(Ok(Some("  line one\r\nline two\n  ")));
        assert!(paste_clipboard(&env, 0x42, Duration::ZERO, false));
        assert_eq!(typed(&fake), "  line one\nline two\n  ");
        let calls = fake.calls.lock().unwrap();
        assert!(
            calls
                .windows(2)
                .any(|w| w == [Call::Press(28), Call::Release(28)])
        );
    }

    #[test]
    fn paste_clipboard_turns_line_breaks_into_spaces_for_the_chat_box() {
        let _g = serial();
        let (fake, env) = setup();
        set_foreground(&[0x42]);
        set_clipboard(Ok(Some("測試\r\nTest\n\n\n123 ")));
        assert!(paste_clipboard(&env, 0x42, Duration::ZERO, true));
        assert_eq!(typed(&fake), "測試 Test   123 ");
    }

    #[test]
    fn paste_clipboard_waits_for_ctrl_and_v_to_be_released_before_typing() {
        let _g = serial();
        let (fake, mut env) = setup();
        set_clipboard(Ok(Some("x")));
        let polls = Arc::new(Mutex::new(0));
        let (ctrl, v) = (code("ctrl"), code("v"));
        env.pressed = Arc::new(move || {
            let mut n = polls.lock().unwrap();
            *n += 1;
            if *n <= 2 {
                [ctrl, v].into()
            } else if *n == 3 {
                [v].into()
            } else {
                Pressed::new()
            }
        });
        assert!(paste_clipboard(&env, 0, Duration::ZERO, false));
        assert_eq!(typed(&fake), "x");
        assert_eq!(slept(), vec![KEY_RELEASE_POLL; 3]);
    }

    #[test]
    fn paste_clipboard_gives_up_waiting_after_the_timeout() {
        let _g = serial();
        let (fake, mut env) = setup();
        set_clipboard(Ok(Some("x")));
        let held = held(&["ctrl"]);
        env.pressed = Arc::new(move || held.clone());
        assert!(paste_clipboard(&env, 0, Duration::ZERO, false));
        assert_eq!(typed(&fake), "x");
        assert!(slept().iter().sum::<Duration>() >= MODIFIER_RELEASE_TIMEOUT);
    }

    #[test]
    fn paste_clipboard_stops_when_foreground_leaves_target() {
        let _g = serial();
        let (fake, env) = setup();
        set_foreground(&[0x42, 0x42, 0x9999, 0x42, 0x42]);
        set_clipboard(Ok(Some("hello")));
        paste_clipboard(&env, 0x42, Duration::ZERO, false);
        assert_eq!(typed(&fake), "he");
    }

    #[test]
    fn paste_clipboard_ignores_a_second_paste_while_typing() {
        let _g = serial();
        let (fake, env) = setup();
        set_clipboard(Ok(Some("slow")));
        let running = PASTE_IN_PROGRESS.lock().unwrap();
        assert!(!paste_clipboard(&env, 0, Duration::ZERO, false));
        drop(running);
        assert!(fake.calls.lock().unwrap().is_empty());
        assert!(paste_clipboard(&env, 0, Duration::ZERO, false));
        assert_eq!(*fake.paste_locked.lock().unwrap(), [true; 4]);
    }

    #[test]
    fn typing_in_progress_is_true_only_while_chars_are_being_typed() {
        let _g = serial();
        let (fake, env) = setup();
        assert!(!typing_in_progress());
        type_into_window(&env, 0, "ab", Duration::ZERO);
        assert_eq!(*fake.typing_seen.lock().unwrap(), [true, true]);
        assert!(!typing_in_progress());
    }

    #[test]
    fn backspace_in_clipboard_is_sent_as_the_backspace_key() {
        let _g = serial();
        let (fake, env) = setup();
        set_clipboard(Ok(Some("a\u{8}")));
        assert!(paste_clipboard(&env, 0, Duration::ZERO, false));
        let bs = code("backspace");
        assert_eq!(
            *fake.calls.lock().unwrap(),
            [Call::Unicode('a'), Call::Press(bs), Call::Release(bs)]
        );
    }

    #[test]
    fn astral_char_is_sent_as_one_surrogate_pair() {
        let _g = serial();
        let (fake, env) = setup();
        type_into_window(&env, 0, "a😀", Duration::ZERO);
        assert_eq!(
            *fake.calls.lock().unwrap(),
            [Call::Unicode('a'), Call::Unicode('😀')]
        );
        assert_eq!(
            encode_units('😀'),
            [
                (0xD83D, false),
                (0xDE00, false),
                (0xD83D, true),
                (0xDE00, true)
            ]
        );
    }
}
