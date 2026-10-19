//! 全域熱鍵與遊戲聊天輸入框事件的分派：決定要不要接手，再交給輸入框、框選或貼上。
//!
//! 回呼跑在鍵盤 hook 裡，只能投遞 `UiEvent`、讀共享狀態，不可阻塞。

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::config::Config;
use crate::config::py::truthy;
use crate::logs::log;
use crate::py::pyrepr::repr_str;
use crate::reader::process::{foreground_window, is_game_process_path, window_exe};
use crate::ui::events::{UiEvent, UiPoster};
use crate::ui::geometry::Rect;
use crate::util::lock;

/// 前景視窗的查詢入口；測試換成假的。
pub struct Foreground {
    /// 前景視窗 HWND。
    pub hwnd: fn() -> isize,
    /// 前景視窗的 exe 路徑；讀不到回 `None`。
    pub exe: fn() -> Option<String>,
}

impl Foreground {
    /// 接 Win32 的真實查詢。
    pub fn real() -> Self {
        Self {
            hwnd: foreground_window,
            exe: || window_exe(foreground_window()),
        }
    }
}

fn is_game(exe: &Option<String>) -> bool {
    exe.as_deref().is_some_and(is_game_process_path)
}

fn exe_repr(exe: &Option<String>) -> String {
    exe.as_deref().map_or("unavailable".into(), repr_str)
}

/// 框選熱鍵要不要註冊：與輸入框熱鍵相同就不註冊（回 `None`），否則回熱鍵字串。
pub fn region_hotkey_to_register(cfg: &Config) -> Option<String> {
    let region = cfg.get("region_hotkey")?.as_str().unwrap_or_default();
    if cfg.get("region_hotkey") == cfg.get("hotkey") {
        log(format!(
            "[app] region_hotkey {} collides with hotkey; \
             region translation disabled until a different hotkey is saved in settings",
            repr_str(region)
        ));
        return None;
    }
    Some(region.to_owned())
}

/// 兩把熱鍵共用的前景守衛：前景是遊戲就回它的 HWND，否則記一行 log 回 `None`。
pub fn foreground_game_hwnd(fg: &Foreground, prefix: &str) -> Option<isize> {
    let exe = (fg.exe)();
    if is_game(&exe) {
        return Some((fg.hwnd)());
    }
    log(format!(
        "[{prefix}] hotkey ignored: foreground is not the game window (exe={})",
        exe_repr(&exe)
    ));
    None
}

/// 全域熱鍵回呼，回傳有沒有接手：遊戲在前景就呼出並改綁，輸入框已開著則只重新對焦。
///
/// 不走 `foreground_game_hwnd`：框自己是前景時那顆的 ignored log 會誤報。
pub fn on_hotkey(fg: &Foreground, box_open: &AtomicBool, ui: &UiPoster) -> bool {
    let exe = (fg.exe)();
    if is_game(&exe) {
        ui.post(UiEvent::ShowInputBox {
            retarget: Some((fg.hwnd)()),
        });
        return true;
    }
    if box_open.load(Ordering::SeqCst) {
        ui.post(UiEvent::ShowInputBox { retarget: None });
        return true;
    }
    log(format!(
        "[app] hotkey ignored: foreground is not the game window (exe={})",
        exe_repr(&exe)
    ));
    false
}

/// 框選熱鍵回呼，回傳有沒有接手：選取層開著就取消，遊戲在前景才開始框選。
pub fn on_region_hotkey(fg: &Foreground, selecting: &AtomicBool, ui: &UiPoster) -> bool {
    if selecting.load(Ordering::SeqCst) {
        ui.post(UiEvent::RegionHotkey { hwnd: 0 });
        return true;
    }
    match foreground_game_hwnd(fg, "region") {
        Some(hwnd) => {
            ui.post(UiEvent::RegionHotkey { hwnd });
            true
        }
        None => false,
    }
}

/// Ctrl+V 要不要由本程式接手：設定開啟且遊戲在前景。
pub fn should_intercept_paste(cfg: &Config, fg: &Foreground) -> bool {
    truthy(cfg.get("paste_hotkey")) && is_game(&(fg.exe)())
}

/// 哪些遊戲視窗的聊天輸入框正開著；雙開時各客戶端各自開關。
#[derive(Default)]
pub struct ChatInputTracker {
    open: Mutex<HashSet<isize>>,
}

impl ChatInputTracker {
    /// 記下該視窗的聊天框開了。
    pub fn opened(&self, hwnd: isize) {
        lock(&self.open).insert(hwnd);
    }

    /// 記下該視窗的聊天框關了。
    pub fn closed(&self, hwnd: isize) {
        lock(&self.open).remove(&hwnd);
    }

    /// 該視窗的聊天框是否開著。
    pub fn is_open(&self, hwnd: isize) -> bool {
        lock(&self.open).contains(&hwnd)
    }
}

/// 翻譯輸入框對 `GameInputEvents` 開放的操作。
pub trait InputBoxControl {
    /// 設定貼齊遊戲聊天框的錨點。
    fn set_anchor(&mut self, r: Rect);
    /// 清除錨點。
    fn clear_anchor(&mut self);
    /// 改綁譯文要打回的客戶端。
    fn retarget(&mut self, hwnd: isize);
    /// 呼出輸入框。
    fn show(&mut self);
    /// 收起輸入框。
    fn hide(&mut self);
}

/// 遊戲聊天輸入框開／關事件的分派（UI 執行緒），只認前景客戶端。
pub struct GameInputEvents {
    tracker: Arc<ChatInputTracker>,
    foreground: fn() -> isize,
    shown_for: Option<isize>,
}

impl GameInputEvents {
    /// `foreground` 回目前前景視窗的 HWND。
    pub fn new(tracker: Arc<ChatInputTracker>, foreground: fn() -> isize) -> Self {
        Self {
            tracker,
            foreground,
            shown_for: None,
        }
    }

    /// 聊天框開了：記下來；是前景客戶端才設錨點、改綁翻譯目標、依設定自動呼出。
    pub fn opened(
        &mut self,
        hwnd: isize,
        anchor: Option<Rect>,
        auto_show: bool,
        b: &mut impl InputBoxControl,
    ) {
        self.tracker.opened(hwnd);
        if (self.foreground)() != hwnd {
            log(format!(
                "[app] game chat opened in a background client (hwnd={hwnd:#x}); ignoring"
            ));
            return;
        }
        self.shown_for = Some(hwnd);
        if let Some(r) = anchor {
            b.set_anchor(r);
        }
        b.retarget(hwnd);
        if auto_show {
            b.show();
        }
    }

    /// 聊天框關了：只有當初為它設錨點／呼出的客戶端才收起。
    pub fn closed(&mut self, hwnd: isize, auto_show: bool, b: &mut impl InputBoxControl) {
        self.tracker.closed(hwnd);
        if self.shown_for != Some(hwnd) {
            return;
        }
        self.shown_for = None;
        b.clear_anchor();
        if auto_show {
            b.hide();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::Receiver;

    use serde_json::json;

    use super::*;

    const GAME: &str = r"C:\Wizard101\Bin\WizardGraphicalClient.exe";

    #[derive(Debug, PartialEq)]
    enum Call {
        Anchor(Rect),
        ClearAnchor,
        Retarget(isize),
        Show,
        Hide,
    }

    #[derive(Default)]
    struct FakeBox(Vec<Call>);

    impl InputBoxControl for FakeBox {
        fn set_anchor(&mut self, r: Rect) {
            self.0.push(Call::Anchor(r));
        }
        fn clear_anchor(&mut self) {
            self.0.push(Call::ClearAnchor);
        }
        fn retarget(&mut self, hwnd: isize) {
            self.0.push(Call::Retarget(hwnd));
        }
        fn show(&mut self) {
            self.0.push(Call::Show);
        }
        fn hide(&mut self) {
            self.0.push(Call::Hide);
        }
    }

    fn fg_a() -> isize {
        0xA
    }
    fn fg_b() -> isize {
        0xB
    }

    fn events(foreground: fn() -> isize) -> (GameInputEvents, Arc<ChatInputTracker>) {
        let tracker = Arc::new(ChatInputTracker::default());
        (
            GameInputEvents::new(Arc::clone(&tracker), foreground),
            tracker,
        )
    }

    #[test]
    fn open_in_the_foreground_window_anchors_and_shows() {
        let (mut ev, tracker) = events(fg_a);
        let mut b = FakeBox::default();
        ev.opened(0xA, Some((1, 2, 3, 4)), true, &mut b);
        assert_eq!(
            b.0,
            [Call::Anchor((1, 2, 3, 4)), Call::Retarget(0xA), Call::Show]
        );
        assert!(tracker.is_open(0xA));
    }

    #[test]
    fn open_in_the_foreground_window_retargets_before_showing() {
        let (mut ev, _) = events(fg_b);
        let mut b = FakeBox::default();
        ev.opened(0xB, Some((1, 2, 3, 4)), true, &mut b);
        let pos = |c: Call| b.0.iter().position(|x| *x == c).unwrap();
        assert!(pos(Call::Retarget(0xB)) < pos(Call::Show));
    }

    #[test]
    fn open_in_a_background_window_is_tracked_but_not_shown() {
        let (mut ev, tracker) = events(fg_b);
        let mut b = FakeBox::default();
        ev.opened(0xA, Some((1, 2, 3, 4)), true, &mut b);
        assert!(b.0.is_empty());
        assert!(tracker.is_open(0xA));
    }

    #[test]
    fn close_from_another_window_does_not_hide_the_box() {
        let (mut ev, tracker) = events(fg_b);
        let mut b = FakeBox::default();
        ev.opened(0xB, None, true, &mut b);
        ev.opened(0xA, None, true, &mut b);
        ev.closed(0xA, true, &mut b);
        assert!(!b.0.contains(&Call::Hide));
        assert!(!tracker.is_open(0xA) && tracker.is_open(0xB));
        ev.closed(0xB, true, &mut b);
        assert_eq!(b.0[b.0.len() - 2..], [Call::ClearAnchor, Call::Hide]);
    }

    #[test]
    fn auto_show_off_still_tracks_and_anchors() {
        let (mut ev, _) = events(fg_a);
        let mut b = FakeBox::default();
        ev.opened(0xA, Some((1, 2, 3, 4)), false, &mut b);
        ev.closed(0xA, false, &mut b);
        assert_eq!(
            b.0,
            [
                Call::Anchor((1, 2, 3, 4)),
                Call::Retarget(0xA),
                Call::ClearAnchor
            ]
        );
    }

    #[test]
    fn paste_single_line_follows_the_foreground_window() {
        let tracker = ChatInputTracker::default();
        tracker.opened(0xB);
        assert!(tracker.is_open(0xB));
        tracker.closed(0xB);
        tracker.opened(0xA);
        assert!(!tracker.is_open(0xB));
        assert!(tracker.is_open(0xA));
    }

    fn game_fg() -> Foreground {
        Foreground {
            hwnd: fg_b,
            exe: || Some(GAME.into()),
        }
    }
    fn other_fg() -> Foreground {
        Foreground {
            hwnd: fg_b,
            exe: || Some(r"C:\Tools\notepad.exe".into()),
        }
    }
    fn unknown_fg() -> Foreground {
        Foreground {
            hwnd: fg_b,
            exe: || None,
        }
    }

    fn drain(rx: &Receiver<UiEvent>) -> Vec<UiEvent> {
        rx.try_iter().collect()
    }

    fn show(retarget: Option<isize>) -> UiEvent {
        UiEvent::ShowInputBox { retarget }
    }

    #[test]
    fn on_hotkey_retargets_before_showing_when_game_is_foreground() {
        let (ui, rx) = UiPoster::for_test();
        assert!(on_hotkey(&game_fg(), &AtomicBool::new(true), &ui));
        assert_eq!(drain(&rx), [show(Some(0xB))]);
    }

    #[test]
    fn on_hotkey_only_refocuses_when_the_box_itself_is_foreground() {
        let (ui, rx) = UiPoster::for_test();
        assert!(on_hotkey(&unknown_fg(), &AtomicBool::new(true), &ui));
        assert_eq!(drain(&rx), [show(None)]);
    }

    #[test]
    fn on_hotkey_ignored_when_box_closed_and_game_not_foreground() {
        let (ui, rx) = UiPoster::for_test();
        assert!(!on_hotkey(&unknown_fg(), &AtomicBool::new(false), &ui));
        assert!(drain(&rx).is_empty());
    }

    #[test]
    fn hotkey_opens_input_box_when_game_is_foreground() {
        let (ui, rx) = UiPoster::for_test();
        assert!(on_hotkey(&game_fg(), &AtomicBool::new(false), &ui));
        assert_eq!(drain(&rx), [show(Some(0xB))]);
    }

    #[test]
    fn hotkey_ignored_when_other_window_is_foreground() {
        let (ui, rx) = UiPoster::for_test();
        assert!(!on_hotkey(&other_fg(), &AtomicBool::new(false), &ui));
        assert!(drain(&rx).is_empty());
    }

    #[test]
    fn hotkey_ignored_when_foreground_unknown() {
        let (ui, rx) = UiPoster::for_test();
        assert!(!on_hotkey(&unknown_fg(), &AtomicBool::new(false), &ui));
        assert!(drain(&rx).is_empty());
    }

    #[test]
    fn hotkey_refocuses_open_input_box_regardless_of_foreground() {
        let (ui, rx) = UiPoster::for_test();
        assert!(on_hotkey(&other_fg(), &AtomicBool::new(true), &ui));
        assert_eq!(drain(&rx), [show(None)]);
    }

    fn cfg(v: serde_json::Value) -> Config {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn region_hotkey_to_register_returns_the_hotkey_when_different() {
        let c = cfg(json!({"hotkey": "f8", "region_hotkey": "ctrl+shift+space"}));
        assert_eq!(
            region_hotkey_to_register(&c).as_deref(),
            Some("ctrl+shift+space")
        );
    }

    #[test]
    fn a_config_without_the_hotkeys_does_not_panic() {
        assert_eq!(region_hotkey_to_register(&cfg(json!({}))), None);
        let c = cfg(json!({"region_hotkey": "f9"}));
        assert_eq!(region_hotkey_to_register(&c).as_deref(), Some("f9"));
    }

    #[test]
    fn region_hotkey_cancels_when_the_selector_is_already_open() {
        let (ui, rx) = UiPoster::for_test();
        assert!(on_region_hotkey(&other_fg(), &AtomicBool::new(true), &ui));
        assert_eq!(drain(&rx), [UiEvent::RegionHotkey { hwnd: 0 }]);
    }

    #[test]
    fn region_hotkey_starts_a_selection_when_game_is_foreground() {
        let (ui, rx) = UiPoster::for_test();
        assert!(on_region_hotkey(&game_fg(), &AtomicBool::new(false), &ui));
        assert_eq!(drain(&rx), [UiEvent::RegionHotkey { hwnd: 0xB }]);
    }

    #[test]
    fn region_hotkey_ignored_when_other_window_is_foreground() {
        let (ui, rx) = UiPoster::for_test();
        assert!(!on_region_hotkey(&other_fg(), &AtomicBool::new(false), &ui));
        assert!(drain(&rx).is_empty());
    }

    #[test]
    fn paste_intercepted_only_when_enabled_and_game_is_foreground() {
        let on = cfg(json!({"paste_hotkey": true}));
        let off = cfg(json!({"paste_hotkey": false}));
        assert!(should_intercept_paste(&on, &game_fg()));
        assert!(!should_intercept_paste(&off, &game_fg()));
        assert!(!should_intercept_paste(&on, &other_fg()));
        assert!(!should_intercept_paste(&on, &unknown_fg()));
    }
}
