//! 熱鍵錄製器：以 `Dispatcher::process` 驅動，不裝真的鍵盤 hook。

use crate::common;

use std::sync::{Arc, Mutex};

use wizard101_chat_translator::input::dispatcher::{
    Dispatcher, KeyEvent, RawKey, SharedDispatcher,
};
use wizard101_chat_translator::input::keys::{event_name, hotkey_name};
use wizard101_chat_translator::input::recorder::record_with;

const CTRL: (i32, u32) = (29, 0xA2);
const SPACE: (i32, u32) = (57, 0x20);
const SHIFT: (i32, u32) = (42, 0xA0);
const ONE: (i32, u32) = (2, 0x31);
const A: (i32, u32) = (30, 0x41);

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

/// 這顆鍵在目前鍵盤配置下的名稱；名稱隨配置而變，名稱表本身由 `keys_golden` 對照。
fn name((code, vk): (i32, u32), mods: &[&str]) -> String {
    event_name(code as u32, vk, false, mods).unwrap()
}

fn ctrl_space() -> String {
    hotkey_name(&[name(CTRL, &[]), name(SPACE, &[])])
}

fn shared() -> SharedDispatcher {
    Arc::new(Mutex::new(Dispatcher::new()))
}

fn process(d: &SharedDispatcher, ev: KeyEvent) -> bool {
    d.lock().unwrap().process(ev)
}

type Done = Box<dyn FnOnce(String) + Send>;

fn no_locks() -> Vec<&'static str> {
    Vec::new()
}

fn record(
    d: &SharedDispatcher,
    done: Done,
) -> wizard101_chat_translator::input::recorder::Recording {
    record_with(d, done, no_locks)
}

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
    assert_eq!(*out.lock().unwrap(), [ctrl_space()]);
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
    let shifted = name(ONE, &["shift"]);
    assert_ne!(shifted, name(ONE, &[]));
    let d = shared();
    let (out, done) = collector();
    let _recording = record(&d, done);
    process(&d, key(true, SHIFT));
    process(&d, key(true, ONE));
    process(&d, key(false, ONE));
    assert_eq!(
        *out.lock().unwrap(),
        [hotkey_name(&[name(SHIFT, &[]), shifted])]
    );
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
    assert_eq!(*out.lock().unwrap(), [ctrl_space()]);
}

#[test]
fn escape_pressed_while_recording_is_reported() {
    const ESC: (i32, u32) = (1, 0x1B);
    let d = shared();
    let (out, done) = collector();
    let _recording = record(&d, done);
    process(&d, key(true, ESC));
    process(&d, key(false, ESC));
    assert_eq!(*out.lock().unwrap(), [hotkey_name(&[name(ESC, &[])])]);
}

#[test]
fn logs_the_event_counts_when_recording_starts_and_stops() {
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
        log.contains("[hotkey] recording started (events processed so far=1, hook calls so far="),
        "{log}"
    );
    assert!(
        log.contains("[hotkey] recording stopped (events processed so far=4, hook calls so far="),
        "{log}"
    );
}

#[test]
fn a_key_without_a_name_is_logged_and_left_out_of_the_hotkey() {
    common::log_capture::capture_logs();
    let d = shared();
    let (out, done) = collector();
    let _recording = record(&d, done);
    let nameless = (0x1fe, 0xFF);
    process(&d, key(true, CTRL));
    process(&d, key(true, nameless));
    process(&d, key(false, nameless));
    assert_eq!(*out.lock().unwrap(), [hotkey_name(&[name(CTRL, &[])])]);
    let log = common::log_capture::log_text();
    assert!(
        log.contains("[hotkey] key without a name ignored: code=510 vk=0xff"),
        "{log}"
    );
}

#[test]
fn a_lit_lock_key_changes_the_recorded_name() {
    let name_with = |locks: fn() -> Vec<&'static str>| {
        let d = shared();
        let (out, done) = collector();
        let _recording = record_with(&d, done, locks);
        process(&d, key(true, A));
        process(&d, key(false, A));
        std::mem::take(&mut *out.lock().unwrap())
    };
    let (plain, caps) = (name(A, &[]), name(A, &["caps lock"]));
    assert_ne!(plain, caps);
    assert_eq!(name_with(no_locks), [plain]);
    assert_eq!(name_with(|| vec!["caps lock"]), [caps]);
}
