//! 按鍵分派與熱鍵註冊中需要斷言 log 的測試。

#[allow(dead_code)]
mod common;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use common::log_capture::{capture_logs, log_lines_with};
use wizard101_chat_translator::input::dispatcher::{
    Blocking, Dispatcher, KeyEvent, Pressed, SharedDispatcher,
};
use wizard101_chat_translator::input::hotkeys::register_hotkey;
use wizard101_chat_translator::input::keys::{Code, key_to_scan_codes};

fn code(name: &str) -> Code {
    key_to_scan_codes(name).unwrap()[0]
}

fn down(code: Code) -> KeyEvent {
    KeyEvent { down: true, code }
}

fn up(code: Code) -> KeyEvent {
    KeyEvent { down: false, code }
}

struct Panics;

impl Blocking for Panics {
    fn handle(&mut self, ev: KeyEvent, _: &Pressed) -> bool {
        if ev.down {
            panic!("handler boom");
        }
        true
    }
}

#[test]
fn panicking_handler_passes_the_event_through() {
    capture_logs();
    let mut d = Dispatcher::new();
    d.add_blocking(Box::new(Panics));
    let a = code("a");
    assert!(d.process(down(a)));
    assert!(d.pressed().contains(&a));
    assert!(d.process(up(a)));
    assert!(d.pressed().is_empty());
    assert!(!log_lines_with("[hotkey] handler panicked").is_empty());
}

struct Setup {
    dispatcher: SharedDispatcher,
    triggers: Arc<AtomicU32>,
    masks: Arc<AtomicU32>,
}

fn register(
    hotkey: &str,
    fallback: &str,
) -> (
    Setup,
    wizard101_chat_translator::input::hotkeys::Registration,
    String,
) {
    capture_logs();
    let dispatcher: SharedDispatcher = Arc::new(Mutex::new(Dispatcher::new()));
    let triggers = Arc::new(AtomicU32::new(0));
    let masks = Arc::new(AtomicU32::new(0));
    let (t, m) = (triggers.clone(), masks.clone());
    let (registration, used) = register_hotkey(
        &dispatcher,
        hotkey,
        Arc::new(move || {
            t.fetch_add(1, Ordering::SeqCst);
            true
        }),
        fallback,
        Arc::new(move || {
            m.fetch_add(1, Ordering::SeqCst);
        }),
    );
    (
        Setup {
            dispatcher,
            triggers,
            masks,
        },
        registration,
        used,
    )
}

impl Setup {
    fn process(&self, ev: KeyEvent) -> bool {
        self.dispatcher.lock().unwrap().process(ev)
    }
}

fn logged(line: &str) -> bool {
    log_lines_with(line).iter().any(|l| l.ends_with(line))
}

#[test]
fn register_hotkey_installs_a_swallowing_handler() {
    // 用其他測試沒用到的熱鍵，log 才分得出是這個測試寫的
    let (s, registration, used) = register("ctrl+f9", "ctrl+space");
    assert_eq!(used, "ctrl+f9");
    assert!(logged(
        "[hotkey] 'ctrl+f9' registered; its trigger key is swallowed when handled"
    ));
    let (ctrl, f9) = (code("ctrl"), code("f9"));
    assert!(s.process(down(ctrl)));
    assert!(!s.process(down(f9)));
    assert!(!s.process(up(f9)));
    assert_eq!(s.triggers.load(Ordering::SeqCst), 1);
    assert_eq!(s.masks.load(Ordering::SeqCst), 1);
    assert!(logged(
        "[hotkey] 'ctrl+f9' swallowed; injected mask key vk=0xe8"
    ));
    registration.unregister();
    assert!(s.process(down(f9)));
    assert_eq!(s.triggers.load(Ordering::SeqCst), 1);
}

#[test]
fn register_hotkey_falls_back_to_passive_when_it_cannot_swallow() {
    for (hotkey, presses, why) in [
        (
            "ctrl+shift",
            vec!["ctrl", "shift"],
            "has only modifier keys",
        ),
        ("ctrl+a, s", vec!["ctrl", "a"], "is a multi-step hotkey"),
    ] {
        let (s, registration, used) = register(hotkey, "ctrl+space");
        assert_eq!(used, hotkey);
        let codes: Vec<Code> = presses.iter().map(|n| code(n)).collect();
        for &c in &codes {
            assert!(s.process(down(c)));
        }
        for &c in &codes {
            assert!(s.process(up(c)));
        }
        let s_code = code("s");
        assert!(s.process(down(s_code)));
        assert!(s.process(up(s_code)));
        assert_eq!(s.triggers.load(Ordering::SeqCst), 1, "{hotkey}");
        assert!(
            logged(&format!(
                "[hotkey] '{hotkey}' {why}; registered without swallowing, its keys still reach the game"
            )),
            "{hotkey}"
        );
        registration.unregister();
        for &c in &codes {
            s.process(down(c));
        }
        s.process(down(s_code));
        assert_eq!(s.triggers.load(Ordering::SeqCst), 1, "{hotkey}");
    }
}

#[test]
fn register_hotkey_falls_back_to_the_default_on_an_unknown_key() {
    let (s, _registration, used) = register("ctrl+nope", "ctrl+space");
    assert_eq!(used, "ctrl+space");
    assert!(s.process(down(code("ctrl"))));
    assert!(!s.process(down(code("space"))));
    assert!(
        log_lines_with("'ctrl+nope' is not a valid key combination")
            .iter()
            .any(|l| l.ends_with(
                "[hotkey] 'ctrl+nope' is not a valid key combination \
                 (Key 'nope' is not mapped to any known key.); using 'ctrl+space'"
            ))
    );
}

#[test]
fn register_hotkey_falls_back_to_the_given_fallback_on_an_unknown_key() {
    let (_, _registration, used) = register("ctrl+nope", "ctrl+shift+space");
    assert_eq!(used, "ctrl+shift+space");
}
