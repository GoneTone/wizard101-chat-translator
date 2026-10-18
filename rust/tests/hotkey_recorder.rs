//! 熱鍵錄製器：以 `Dispatcher::process` 驅動，不裝真的鍵盤 hook。

mod common;

use std::sync::{Arc, Mutex};

use wizard101_chat_translator::input::dispatcher::{
    Dispatcher, KeyEvent, RawKey, SharedDispatcher,
};
use wizard101_chat_translator::input::recorder::record;

const CTRL: (i32, u32) = (29, 0xA2);
const SPACE: (i32, u32) = (57, 0x20);
const SHIFT: (i32, u32) = (42, 0xA0);
const ONE: (i32, u32) = (2, 0x31);

fn key(down: bool, (code, vk): (i32, u32)) -> KeyEvent {
    KeyEvent {
        down,
        code,
        raw: Some(RawKey {
            vk,
            extended: false,
        }),
    }
}

fn shared() -> SharedDispatcher {
    Arc::new(Mutex::new(Dispatcher::new()))
}

fn process(d: &SharedDispatcher, ev: KeyEvent) -> bool {
    d.lock().unwrap().process(ev)
}

type Done = Box<dyn FnOnce(String) + Send>;

fn collector() -> (Arc<Mutex<Vec<String>>>, Done) {
    let out = Arc::new(Mutex::new(Vec::new()));
    let sink = out.clone();
    (out, Box::new(move |name| sink.lock().unwrap().push(name)))
}

#[test]
fn records_ctrl_space_on_the_first_release() {
    let d = shared();
    let (out, done) = collector();
    let _recording = record(&d, done);
    process(&d, key(true, CTRL));
    process(&d, key(true, SPACE));
    assert!(out.lock().unwrap().is_empty());
    process(&d, key(false, SPACE));
    assert_eq!(*out.lock().unwrap(), ["ctrl+space"]);
    process(&d, key(false, CTRL));
    process(&d, key(true, SPACE));
    process(&d, key(false, SPACE));
    assert_eq!(out.lock().unwrap().len(), 1, "done is called only once");
}

#[test]
fn does_not_swallow_keys() {
    let d = shared();
    let (_out, done) = collector();
    let _recording = record(&d, done);
    for ev in [
        key(true, CTRL),
        key(true, SPACE),
        key(false, SPACE),
        key(true, SPACE),
        key(false, CTRL),
    ] {
        assert!(process(&d, ev));
    }
}

#[test]
fn shift_changes_the_recorded_name_like_python() {
    let expected = common::golden("keyboard_event_names")
        .into_iter()
        .find(|c| {
            c["input"]["scan"] == 2 && c["input"]["modifiers"] == serde_json::json!(["shift"])
        })
        .expect("shift+1 case")["output"]
        .as_str()
        .unwrap()
        .to_string();
    let d = shared();
    let (out, done) = collector();
    let _recording = record(&d, done);
    process(&d, key(true, SHIFT));
    process(&d, key(true, ONE));
    process(&d, key(false, ONE));
    assert_eq!(*out.lock().unwrap(), [format!("shift+{expected}")]);
}

#[test]
fn dropping_a_recording_removes_its_handler() {
    let d = shared();
    let before = d.lock().unwrap().handler_count();
    let (out, done) = collector();
    let recording = record(&d, done);
    assert_eq!(d.lock().unwrap().handler_count(), before + 1);
    drop(recording);
    assert_eq!(d.lock().unwrap().handler_count(), before);
    process(&d, key(true, SPACE));
    process(&d, key(false, SPACE));
    assert!(out.lock().unwrap().is_empty());
}

#[test]
fn a_release_of_a_key_pressed_before_recording_is_ignored() {
    const ENTER: (i32, u32) = (28, 0x0D);
    let d = shared();
    let (out, done) = collector();
    let _recording = record(&d, done);
    assert!(process(&d, key(false, ENTER)));
    assert!(out.lock().unwrap().is_empty());
    process(&d, key(true, CTRL));
    process(&d, key(true, SPACE));
    process(&d, key(false, SPACE));
    assert_eq!(*out.lock().unwrap(), ["ctrl+space"]);
}

#[test]
fn escape_pressed_while_recording_is_reported() {
    const ESC: (i32, u32) = (1, 0x1B);
    let d = shared();
    let (out, done) = collector();
    let _recording = record(&d, done);
    process(&d, key(true, ESC));
    process(&d, key(false, ESC));
    assert_eq!(*out.lock().unwrap(), ["esc"]);
}

#[test]
fn logs_each_event_it_sees_until_the_first_release() {
    common::log_capture::capture_logs();
    let d = shared();
    process(&d, key(true, ONE));
    let (_out, done) = collector();
    let recording = record(&d, done);
    process(&d, key(true, SPACE));
    process(&d, key(false, SPACE));
    process(&d, key(true, (88, 0x7B)));
    drop(recording);
    let log = common::log_capture::log_text();
    assert!(
        log.contains("[hotkey] recording started (events processed so far=1,"),
        "{log}"
    );
    assert!(
        log.contains("[hotkey] recorder saw code=57 vk=0x20 down=true name=Some(\"space\")"),
        "{log}"
    );
    assert!(
        log.contains("[hotkey] recorder saw code=57 vk=0x20 down=false"),
        "{log}"
    );
    assert!(
        !log.contains("[hotkey] recorder saw code=88"),
        "nothing is logged after the first release: {log}"
    );
    assert!(
        log.contains("[hotkey] recording stopped (events processed so far=4,"),
        "{log}"
    );
}
