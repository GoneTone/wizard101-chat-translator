//! 熱鍵與遊戲輸入框分派中需要斷言 log 的測試。

#[allow(dead_code)]
mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use common::log_capture::{capture_logs, log_lines_with, logged};
use egui_kittest::Harness;
use serde_json::json;
use wizard101_chat_translator::input::dispatch::{
    ChatInputTracker, Foreground, GameInputEvents, InputBoxControl, foreground_game_hwnd,
    on_hotkey, region_hotkey_to_register,
};
use wizard101_chat_translator::ui::app::{App, channel};
use wizard101_chat_translator::ui::events::{UiEvent, UiPoster};
use wizard101_chat_translator::ui::fonts_egui;
use wizard101_chat_translator::ui::geometry::Rect;
use wizard101_chat_translator::ui::overlay::frame::Frame;

fn fg(exe: fn() -> Option<String>) -> Foreground {
    Foreground { hwnd: || 0xB, exe }
}

#[test]
fn region_hotkey_to_register_returns_none_on_collision() {
    capture_logs();
    let cfg = json!({"hotkey": "ctrl+shift+space", "region_hotkey": "ctrl+shift+space"});
    assert_eq!(region_hotkey_to_register(cfg.as_object().unwrap()), None);
    assert!(
        !log_lines_with("[app] region_hotkey 'ctrl+shift+space' collides with hotkey;").is_empty()
    );
}

#[test]
fn on_hotkey_does_not_log_ignored_when_box_open_and_foreground_is_not_the_game() {
    capture_logs();
    let (ui, _rx) = UiPoster::for_test();
    let f = fg(|| Some(r"C:\Tools\open_box_notepad.exe".into()));
    assert!(on_hotkey(&f, &AtomicBool::new(true), &ui));
    assert!(log_lines_with("open_box_notepad").is_empty());
}

#[test]
fn ignored_hotkey_logs_the_foreground_exe() {
    capture_logs();
    let (ui, _rx) = UiPoster::for_test();
    assert!(!on_hotkey(&fg(|| None), &AtomicBool::new(false), &ui));
    assert!(
        !log_lines_with("[app] hotkey ignored: foreground is not the game window (exe=None)")
            .is_empty()
    );
}

#[test]
fn region_hotkey_logs_until_phase_3() {
    capture_logs();
    let (poster, inbox) = channel();
    poster.post(UiEvent::RegionHotkey { hwnd: 0x5E61 });
    let mut harness = Harness::new_eframe(|cc| {
        fonts_egui::install(&cc.egui_ctx, "en-US");
        App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
    });
    harness.step();
    assert_eq!(
        log_lines_with("[ui] region hotkey pressed (hwnd=0x5e61)")
            .iter()
            .map(|l| l.split_once(' ').unwrap().1)
            .collect::<Vec<_>>(),
        ["[ui] region hotkey pressed (hwnd=0x5e61); region capture arrives in phase 3"]
    );
}

#[test]
fn region_guard_logs_the_foreground_exe_with_its_prefix() {
    capture_logs();
    let f = fg(|| Some("region_guard_probe.exe".into()));
    assert_eq!(foreground_game_hwnd(&f, "region"), None);
    assert!(logged(
        "[region] hotkey ignored: foreground is not the game window (exe='region_guard_probe.exe')"
    ));
}

struct NoBox;

impl InputBoxControl for NoBox {
    fn set_anchor(&mut self, _: Rect) {}
    fn clear_anchor(&mut self) {}
    fn retarget(&mut self, _: isize) {}
    fn show(&mut self) {}
    fn hide(&mut self) {}
}

#[test]
fn background_client_chat_open_is_logged_and_ignored() {
    capture_logs();
    let mut events = GameInputEvents::new(Arc::new(ChatInputTracker::default()), || 0xB);
    events.opened(0x7AC1, None, true, &mut NoBox);
    assert!(logged(
        "[app] game chat opened in a background client (hwnd=0x7ac1); ignoring"
    ));
}
