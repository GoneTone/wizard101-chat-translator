//! 主程式接線：收訊指紋、翻譯端的建構、單一實例與關閉順序。

#[allow(dead_code)]
mod common;

use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::{Duration, Instant};

use common::log_capture::{capture_logs, log_lines_with};
use common::reader_fixture::{EXPECTED_PANIC, quiet_panics};

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wizard101_chat_translator::app_main::{
    SUPERVISOR_JOIN_BUDGET, build_translation, claim_instance, incoming_fingerprint, run_ui_then,
    shutdown_steps,
};
use wizard101_chat_translator::config::services::resolve;
use wizard101_chat_translator::config::{Config, ServiceSlot, app_name};
use wizard101_chat_translator::reader::supervisor::JOIN_TIMEOUT;
use wizard101_chat_translator::translation::cache::TranslationCache;
use wizard101_chat_translator::translation::examples::{
    ExampleSet, ExampleStore, examples_fingerprint,
};
use wizard101_chat_translator::translation::pool::ResultFn;

/// 收訊指自訂端點、預設（發話）是 OpenAI、框選另指一筆，三格的模型各不相同。
fn cfg() -> Config {
    let Value::Object(cfg) = json!({
        "services": [
            {"id": "aaaa0001", "name": "In", "provider": "custom", "base_url": "http://in",
             "model": "qwen3", "api_key": "", "thinking": false},
            {"id": "aaaa0002", "name": "Out", "provider": "openai", "model": "gpt-x",
             "api_key": "k", "thinking": false},
            {"id": "aaaa0003", "name": "Region", "provider": "custom",
             "base_url": "http://region", "model": "region-model", "api_key": "",
             "thinking": false},
        ],
        "default_service": "aaaa0002",
        "service_slots": {"incoming": "aaaa0001", "outgoing": null, "region": "aaaa0003"},
        "target_language": "日本語",
        "max_parallel_translations": 4,
    }) else {
        unreachable!()
    };
    cfg
}

fn examples() -> ExampleSet {
    ExampleSet {
        incoming: Some(("s".into(), "o".into())),
        system: None,
        region: None,
    }
}

fn fingerprint(slot: ServiceSlot) -> String {
    examples_fingerprint(&resolve(&cfg(), slot), "日本語")
}

fn no_results() -> ResultFn {
    Arc::new(|_, _, _| {})
}

#[test]
fn incoming_fingerprint_matches_python() {
    let mut cfg = cfg();
    assert_eq!(
        incoming_fingerprint(&cfg, None),
        "custom|http://in|qwen3|日本語|p6|x-"
    );
    assert_eq!(
        incoming_fingerprint(&cfg, Some(&examples())),
        "custom|http://in|qwen3|日本語|p6|x8e0aa6546381"
    );
    cfg["service_slots"]["incoming"] = Value::Null;
    assert_eq!(
        incoming_fingerprint(&cfg, None),
        "openai||gpt-x|日本語|p6|x-"
    );
}

#[test]
fn build_translation_prepares_examples_for_incoming_and_region_only() {
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = Arc::new(ExampleStore::new(dir.path().join("examples.json")));
    // 三格的範例都已在磁碟上：被 ensure 的才會套用，且都走快取、不碰網路
    for slot in ServiceSlot::ALL {
        store.put(&fingerprint(slot), &examples());
    }
    let built = build_translation(
        &Arc::new(RwLock::new(cfg())),
        no_results(),
        rt.handle(),
        store,
        dir.path().join("cache.json"),
    );
    assert_eq!(built.incoming.examples(), Some(examples()));
    assert_eq!(built.region.examples(), Some(examples()));
    assert_eq!(built.outgoing.examples(), None);
    assert!(built.incoming.describe().contains("model=qwen3"));
    assert!(built.outgoing.describe().contains("model=gpt-x"));
    assert!(built.region.describe().contains("model=region-model"));
    assert_eq!(
        built.cache.fingerprint(),
        incoming_fingerprint(&cfg(), Some(&examples()))
    );
}

#[test]
fn applying_incoming_examples_rebinds_the_system_message_cache() {
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = Arc::new(ExampleStore::new(dir.path().join("examples.json")));
    for slot in [ServiceSlot::Incoming, ServiceSlot::Region] {
        store.put(&fingerprint(slot), &examples());
    }
    let built = build_translation(
        &Arc::new(RwLock::new(cfg())),
        no_results(),
        rt.handle(),
        Arc::clone(&store),
        dir.path().join("cache.json"),
    );
    let newer = ExampleSet {
        system: Some(("s2".into(), "o2".into())),
        ..examples()
    };
    store.put(&fingerprint(ServiceSlot::Region), &newer);
    built.coordinator.ensure(&built.region);
    assert_eq!(
        built.cache.fingerprint(),
        incoming_fingerprint(&cfg(), Some(&examples())),
        "region examples leave the cache alone"
    );
    store.put(&fingerprint(ServiceSlot::Incoming), &newer);
    built.coordinator.ensure(&built.incoming);
    assert_eq!(
        built.cache.fingerprint(),
        incoming_fingerprint(&cfg(), Some(&newer))
    );
}

#[test]
fn restart_keeps_the_persisted_cache_when_examples_are_already_stored() {
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = Arc::new(ExampleStore::new(dir.path().join("examples.json")));
    store.put(&fingerprint(ServiceSlot::Incoming), &examples());
    store.put(&fingerprint(ServiceSlot::Region), &examples());
    let cache_path = dir.path().join("cache.json");
    let seeded = TranslationCache::new(
        cache_path.clone(),
        incoming_fingerprint(&cfg(), Some(&examples())),
    );
    assert!(seeded.put("hello", "bonjour", &seeded.fingerprint()));
    seeded.flush();

    let built = build_translation(
        &Arc::new(RwLock::new(cfg())),
        no_results(),
        rt.handle(),
        store,
        cache_path,
    );
    assert_eq!(built.cache.get("hello").as_deref(), Some("bonjour"));
}

#[test]
fn shutdown_unhooks_the_keyboard_after_pools_and_before_joining_readers() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let stop = CancellationToken::new();
    let (unhooked, readers_may_exit) = mpsc::channel();
    let supervisor = {
        let (calls, stop) = (Arc::clone(&calls), stop.clone());
        std::thread::spawn(move || {
            while !stop.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            // 等卸完 hook 才結束，記錄順序才不受執行緒排程影響
            readers_may_exit.recv().unwrap();
            calls.lock().unwrap().push("readers joined");
        })
    };
    let record = |what: &'static str| {
        let calls = Arc::clone(&calls);
        move || calls.lock().unwrap().push(what)
    };
    let close_pools = {
        let (record, stop) = (record("pools shut down"), stop.clone());
        move || {
            assert!(stop.is_cancelled(), "stop is set before the pools close");
            record();
        }
    };
    let unhook_keyboard = {
        let record = record("keyboard unhooked");
        move || {
            record();
            unhooked.send(()).unwrap();
        }
    };
    shutdown_steps(
        &stop,
        close_pools,
        unhook_keyboard,
        Some(supervisor),
        SUPERVISOR_JOIN_BUDGET,
        record("cache flushed"),
    );
    assert_eq!(
        *calls.lock().unwrap(),
        [
            "pools shut down",
            "keyboard unhooked",
            "readers joined",
            "cache flushed"
        ]
    );
}

#[test]
fn shutdown_gives_up_on_a_stuck_supervisor_after_the_budget() {
    capture_logs();
    assert_eq!(
        SUPERVISOR_JOIN_BUDGET,
        JOIN_TIMEOUT + Duration::from_secs(2)
    );
    let (_release, stuck) = mpsc::channel::<()>();
    let supervisor = std::thread::spawn(move || {
        let _ = stuck.recv();
    });
    let flushed = Mutex::new(false);
    let started = Instant::now();
    shutdown_steps(
        &CancellationToken::new(),
        || {},
        || {},
        Some(supervisor),
        Duration::from_millis(100),
        || *flushed.lock().unwrap() = true,
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(*flushed.lock().unwrap(), "the cache is still flushed");
    assert_eq!(
        log_lines_with("[app] reader supervisor still running after 0.1s; continuing shutdown")
            .len(),
        1
    );
}

#[test]
fn shutdown_still_runs_when_the_ui_panics() {
    capture_logs();
    quiet_panics();
    let ran = Mutex::new(false);
    run_ui_then(
        || panic!("{EXPECTED_PANIC} overlay frame exploded"),
        || *ran.lock().unwrap() = true,
    );
    assert!(*ran.lock().unwrap());
    assert!(!log_lines_with("[ui] overlay panicked; shutting down").is_empty());
}

#[test]
fn second_instance_focuses_and_exits() {
    let focused = Mutex::new(Vec::new());
    let focus = |title: &str| {
        focused.lock().unwrap().push(title.to_owned());
        true
    };
    assert!(claim_instance(|| None::<()>, focus).is_none());
    assert_eq!(*focused.lock().unwrap(), [app_name()]);

    focused.lock().unwrap().clear();
    assert_eq!(claim_instance(|| Some(7), focus), Some(7));
    assert!(
        focused.lock().unwrap().is_empty(),
        "the first instance focuses nothing"
    );
}

#[test]
fn exe_embeds_the_app_icon_resource() {
    use windows_sys::Win32::UI::Shell::ExtractIconExW;
    // Windows 11 工作列讀的是 exe 資源裡的圖示，不是執行期設到視窗上的那一份
    let exe: Vec<u16> = env!("CARGO_BIN_EXE_Wizard101ChatTranslator")
        .encode_utf16()
        .chain([0])
        .collect();
    let count = unsafe {
        ExtractIconExW(
            exe.as_ptr(),
            -1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(count, 1);
}
