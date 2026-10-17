//! 主程式接線：收訊指紋、翻譯端的建構、單一實例與關閉順序。

#[allow(dead_code)]
mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::{Duration, Instant};

use common::log_capture::{capture_logs, log_lines_with};
use common::reader_fixture::{EXPECTED_PANIC, quiet_panics};

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wizard101_chat_translator::app_main::{
    InputWiring, SUPERVISOR_JOIN_BUDGET, build_translation, claim_instance, incoming_fingerprint,
    install_input, on_paste_hotkey, outgoing_translate, region_runner, run_ui_then, shutdown_steps,
};
use wizard101_chat_translator::composer::paste::{TypingEnv, type_into_window, typing_in_progress};
use wizard101_chat_translator::config::services::resolve;
use wizard101_chat_translator::config::{Config, ServiceSlot, ServiceSpec, app_name};
use wizard101_chat_translator::i18n::t;
use wizard101_chat_translator::input::dispatch::ChatInputTracker;
use wizard101_chat_translator::input::dispatcher::{
    Dispatcher, KeyEvent, Pressed, SharedDispatcher,
};
use wizard101_chat_translator::input::inject::Injector;
use wizard101_chat_translator::input::keys::{Code, key_to_scan_codes};
use wizard101_chat_translator::reader::supervisor::JOIN_TIMEOUT;
use wizard101_chat_translator::region::ocr::OcrError;
use wizard101_chat_translator::region::pipeline::{
    Recognize, RegionError, RegionPipeline, RegionResult, describe_error,
};
use wizard101_chat_translator::translation::cache::TranslationCache;
use wizard101_chat_translator::translation::clients::{ChatBackend, ChatRequest};
use wizard101_chat_translator::translation::context::{ClientContexts, ContextMap};
use wizard101_chat_translator::translation::errors::TranslateError;
use wizard101_chat_translator::translation::examples::{
    ExampleSet, ExampleStore, examples_fingerprint,
};
use wizard101_chat_translator::translation::pool::ResultFn;
use wizard101_chat_translator::translation::translator::Translator;
use wizard101_chat_translator::translation::{BoxFuture, Turn};
use wizard101_chat_translator::ui::events::{UiEvent, UiPoster};
use wizard101_chat_translator::ui::input_box::OutgoingRequest;

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

/// 假鍵盤：記下打出的字（Enter 記成 `\n`）；`gate` 有值時每個字都等它放行。
#[derive(Default)]
struct Keys {
    typed: Mutex<String>,
    gate: Mutex<Option<Receiver<()>>>,
}

impl Injector for Keys {
    fn press(&self, code: Code) {
        if code == key("enter") {
            self.typed.lock().unwrap().push('\n');
        }
    }
    fn release(&self, _: Code) {}
    fn unicode(&self, ch: char) {
        if let Some(gate) = self.gate.lock().unwrap().as_ref() {
            let _ = gate.recv();
        }
        self.typed.lock().unwrap().push(ch);
    }
    fn mask(&self) {}
}

static SLEPT: Mutex<Vec<Duration>> = Mutex::new(Vec::new());

/// 鍵入的全域鎖整個執行檔共用，碰到它的測試排隊跑。
static TYPING_TESTS: Mutex<()> = Mutex::new(());

/// 不碰桌面的鍵入環境：前景固定 0x77、剪貼簿固定 "a\nb"。
fn typing_env(keys: &Arc<Keys>) -> Arc<TypingEnv> {
    Arc::new(TypingEnv {
        inject: Arc::clone(keys) as Arc<dyn Injector>,
        foreground: || 0x77,
        is_window: |_| true,
        force_foreground: |_| true,
        sleep: |d| SLEPT.lock().unwrap().push(d),
        now: Instant::now,
        pressed: Arc::new(Pressed::new),
        clipboard: || Ok(Some("a\nb".into())),
    })
}

fn input_cfg(hotkey: &str, region: &str, paste: bool) -> Arc<RwLock<Config>> {
    let mut c = cfg();
    c.insert("hotkey".into(), json!(hotkey));
    c.insert("region_hotkey".into(), json!(region));
    c.insert("paste_hotkey".into(), json!(paste));
    c.insert("type_delay".into(), json!(0.03));
    Arc::new(RwLock::new(c))
}

fn dispatcher() -> SharedDispatcher {
    Arc::new(Mutex::new(Dispatcher::new()))
}

fn wiring(box_open: bool) -> InputWiring {
    InputWiring {
        tracker: Arc::default(),
        box_open: Arc::new(AtomicBool::new(box_open)),
        selecting: Arc::default(),
    }
}

#[test]
fn install_input_writes_the_effective_hotkeys_back() {
    capture_logs();
    let (ui, _rx) = UiPoster::for_test();
    let keys = Arc::new(Keys::default());
    let settings = input_cfg("ctrl+nope", "ctrl+space", false);
    let (_hotkey, region) = install_input(
        &settings,
        &ui,
        &dispatcher(),
        &wiring(false),
        typing_env(&keys),
    );
    assert!(region.is_none());
    let cfg = settings.read().unwrap();
    assert_eq!(cfg["hotkey"], "ctrl+space");
    assert_eq!(cfg["region_hotkey"], "ctrl+space");
    drop(cfg);

    let settings = input_cfg("ctrl+space", "ctrl+bogus", false);
    let (_hotkey, region) = install_input(
        &settings,
        &ui,
        &dispatcher(),
        &wiring(false),
        typing_env(&keys),
    );
    assert!(region.is_some());
    assert_eq!(
        settings.read().unwrap()["region_hotkey"],
        "ctrl+shift+space"
    );
}

fn key(name: &str) -> Code {
    key_to_scan_codes(name).unwrap()[0]
}

/// 依序按下 `names` 再反序放開，回傳最後一顆按下時有沒有放行。
fn press(d: &SharedDispatcher, names: &[&str]) -> bool {
    let mut d = d.lock().unwrap();
    let mut passed = true;
    for name in names {
        passed = d.process(KeyEvent {
            down: true,
            code: key(name),
        });
    }
    for name in names.iter().rev() {
        d.process(KeyEvent {
            down: false,
            code: key(name),
        });
    }
    passed
}

#[test]
fn install_input_registers_paste_last() {
    let _serial = TYPING_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    capture_logs();
    let (ui, rx) = UiPoster::for_test();
    let settings = input_cfg("ctrl+v", "shift+v", false);
    let d = dispatcher();
    let keys = Arc::new(Keys::default());
    let _registered = install_input(&settings, &ui, &d, &wiring(true), typing_env(&keys));

    // 鍵入進行中，貼上攔截器會吞掉每一顆 v；排在它前面的熱鍵才收得到
    let (release, gate) = mpsc::channel();
    let blocked = Arc::new(Keys::default());
    *blocked.gate.lock().unwrap() = Some(gate);
    let env = typing_env(&blocked);
    let typing = std::thread::spawn(move || type_into_window(&env, 0, "x", Duration::ZERO));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !typing_in_progress() {
        assert!(Instant::now() < deadline, "typing never started");
        std::thread::yield_now();
    }

    assert!(!press(&d, &["ctrl", "v"]));
    assert!(
        matches!(rx.try_recv(), Ok(UiEvent::ShowInputBox { .. })),
        "the input box hotkey runs before paste"
    );
    let ignored = || log_lines_with("[region] hotkey ignored").len();
    let before = ignored();
    assert!(!press(&d, &["shift", "v"]));
    let region_ran =
        matches!(rx.try_recv(), Ok(UiEvent::RegionHotkey { .. })) || ignored() > before;
    assert!(region_ran, "the region hotkey runs before paste");
    assert!(!press(&d, &["v"]), "paste swallows v while typing");

    release.send(()).unwrap();
    typing.join().unwrap();
}

/// 假 backend：記下每次請求的對話輪，回固定結果。
struct Backend {
    reply: Result<String, TranslateError>,
    turns: Mutex<Vec<Vec<Turn>>>,
}

impl ChatBackend for Backend {
    fn chat<'a>(
        &'a self,
        req: ChatRequest<'a>,
        _cancel: Option<&'a CancellationToken>,
    ) -> BoxFuture<'a, Result<String, TranslateError>> {
        self.turns.lock().unwrap().push(req.turns.to_vec());
        let reply = self.reply.clone();
        Box::pin(async move { reply })
    }

    fn list_models(&self) -> BoxFuture<'_, Result<Vec<String>, TranslateError>> {
        Box::pin(async { Ok(vec![]) })
    }

    fn model(&self) -> &str {
        "fake-model"
    }
}

struct Outgoing {
    rt: tokio::runtime::Runtime,
    backend: Arc<Backend>,
    translator: Arc<Translator>,
    contexts: ContextMap,
}

fn outgoing(reply: Result<String, TranslateError>) -> Outgoing {
    capture_logs();
    let backend = Arc::new(Backend {
        reply,
        turns: Mutex::default(),
    });
    let fake = Arc::clone(&backend);
    let spec = ServiceSpec {
        provider: "custom".into(),
        base_url: "http://x".into(),
        model: "fake-model".into(),
        ..ServiceSpec::default()
    };
    Outgoing {
        rt: tokio::runtime::Runtime::new().unwrap(),
        translator: Translator::with_client(
            move |_| fake.clone() as Arc<dyn ChatBackend>,
            spec,
            "日本語".into(),
        ),
        backend,
        contexts: ContextMap::default(),
    }
}

impl Outgoing {
    fn submit(&self, text: &str, target: isize, session: u64) -> Receiver<UiEvent> {
        let (ui, rx) = UiPoster::for_test();
        let translate = outgoing_translate(
            Arc::clone(&self.translator),
            Arc::clone(&self.contexts),
            self.rt.handle().clone(),
            ui,
        );
        translate(OutgoingRequest {
            text: text.into(),
            target,
            session,
            cancel: CancellationToken::new(),
        });
        rx
    }
}

#[test]
fn outgoing_translate_uses_the_target_clients_current_tab_context() {
    let o = outgoing(Ok("hi".into()));
    let client = Arc::new(ClientContexts::new());
    client.set_current_tab(2);
    client.for_tab(1).push("other-tab-line");
    client.for_tab(2).push("current-tab-line");
    o.contexts.lock().unwrap().insert(0x10, client);

    let rx = o.submit("hello there", 0x10, 3);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        UiEvent::OutgoingDone {
            session: 3,
            result: Ok("hi".into())
        }
    );
    let rx = o.submit("hello again", 0x99, 4);
    rx.recv_timeout(Duration::from_secs(5)).unwrap();

    let turns = o.backend.turns.lock().unwrap();
    let mentions = |call: &[Turn], s: &str| call.iter().any(|t| t.content.contains(s));
    assert!(mentions(&turns[0], "current-tab-line"));
    assert!(!mentions(&turns[0], "other-tab-line"));
    assert!(!mentions(&turns[1], "current-tab-line"));
    assert!(
        turns[1].len() < turns[0].len(),
        "no client means no context"
    );
}

#[test]
fn outgoing_translate_posts_the_localized_error_and_logs_the_service() {
    let o = outgoing(Err(TranslateError::offline("conn refused")));
    let rx = o.submit("err-case-text", 0, 7);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        UiEvent::OutgoingDone {
            session: 7,
            result: Err(t("input.failed", &[("error", "conn refused")]))
        }
    );
    let lines = log_lines_with("[input] outgoing translation failed (chars=13,");
    assert_eq!(lines.len(), 1);
    assert!(lines[0].ends_with(
        "[input] outgoing translation failed \
         (chars=13, provider=custom, model=fake-model): TranslatorOffline: conn refused"
    ));
}

#[test]
fn outgoing_translate_cancel_logs_and_posts_nothing() {
    let o = outgoing(Err(TranslateError::Cancelled));
    let rx = o.submit("cancelled-text-17", 0, 9);
    let marker = "[input] outgoing translation cancelled (chars=17)";
    let deadline = Instant::now() + Duration::from_secs(5);
    while log_lines_with(marker).is_empty() {
        assert!(Instant::now() < deadline, "no cancel log");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
}

const REGION_RECT: (i32, i32, i32, i32) = (1, 2, 30, 40);

impl Outgoing {
    /// 用假辨識（固定回 `source`）跑一次框選翻譯。
    fn run_region(&self, source: &'static str, session: u64) -> Receiver<UiEvent> {
        self.run_region_with(Arc::new(move |_| Ok(source.to_owned())), session)
    }

    /// 用注入的辨識跑一次框選翻譯。
    fn run_region_with(&self, recognize: Recognize, session: u64) -> Receiver<UiEvent> {
        let (ui, rx) = UiPoster::for_test();
        let runner = region_runner(
            RegionPipeline::new(Arc::clone(&self.translator), recognize),
            self.rt.handle().clone(),
            ui,
        );
        runner.run(
            vec![1, 2, 3],
            REGION_RECT,
            session,
            CancellationToken::new(),
        );
        rx
    }
}

#[test]
fn region_runner_posts_the_result_with_its_session() {
    let o = outgoing(Ok("hi".into()));
    let rx = o.run_region("hello", 5);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        UiEvent::RegionDone {
            session: 5,
            result: Ok(RegionResult {
                text: "hi".into(),
                source: "hello".into()
            })
        }
    );
}

#[test]
fn region_runner_cancel_posts_nothing() {
    let o = outgoing(Err(TranslateError::Cancelled));
    let rx = o.run_region("hello", 41);
    let marker = "[region] request cancelled (session=41)";
    let deadline = Instant::now() + Duration::from_secs(5);
    while log_lines_with(marker).is_empty() {
        assert!(Instant::now() < deadline, "no cancel log");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
}

#[test]
fn region_runner_failure_logs_the_service() {
    let o = outgoing(Err(TranslateError::offline("region refused")));
    let rx = o.run_region("hello", 6);
    let expected = describe_error(&RegionError::Translate(TranslateError::offline(
        "region refused",
    )));
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        UiEvent::RegionDone {
            session: 6,
            result: Err(expected)
        }
    );
    let lines = log_lines_with("region refused");
    assert_eq!(lines.len(), 1);
    assert!(lines[0].ends_with(
        "[region] translate failed (rect=(1, 2, 30, 40), \
         provider=custom, model=fake-model): TranslatorOffline: region refused"
    ));
}

/// 辨識出錯或 panic 都走意外錯誤：卡片顯示 `error.unexpected`、記下 unexpected failure。
fn assert_region_unexpected(recognize: Recognize, session: u64, message: &str) {
    let o = outgoing(Ok("unused".into()));
    let rx = o.run_region_with(recognize, session);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        UiEvent::RegionDone {
            session,
            result: Err(t("error.unexpected", &[("error", message)]))
        }
    );
    let lines = log_lines_with(message);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].ends_with(&format!(
            "[region] unexpected failure (rect=(1, 2, 30, 40)): {message}"
        )),
        "{lines:?}"
    );
    assert!(o.backend.turns.lock().unwrap().is_empty());
}

#[test]
fn region_runner_recognition_error_is_unexpected() {
    assert_region_unexpected(
        Arc::new(|_| Err(OcrError::Failed("runner detect boom".into()))),
        8,
        "runner detect boom",
    );
}

#[test]
fn region_runner_recognize_panic_is_unexpected() {
    quiet_panics();
    let message = format!("{EXPECTED_PANIC} runner recognize exploded");
    let panicking = message.clone();
    assert_region_unexpected(Arc::new(move |_| panic!("{panicking}")), 9, &message);
}

#[test]
fn region_hotkey_cancels_while_selecting() {
    capture_logs();
    let (ui, rx) = UiPoster::for_test();
    let settings = input_cfg("ctrl+space", "ctrl+shift+space", false);
    let d = dispatcher();
    let wiring = wiring(false);
    wiring.selecting.store(true, Ordering::SeqCst);
    let keys = Arc::new(Keys::default());
    let _registered = install_input(&settings, &ui, &d, &wiring, typing_env(&keys));
    assert!(!press(&d, &["ctrl", "shift", "space"]));
    assert_eq!(rx.try_recv(), Ok(UiEvent::RegionHotkey { hwnd: 0 }));
}

#[test]
fn paste_hotkey_types_single_line_only_while_the_game_chat_box_is_open() {
    let _serial = TYPING_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    capture_logs();
    let settings = input_cfg("ctrl+space", "ctrl+shift+space", true);
    let tracker = ChatInputTracker::default();
    let keys = Arc::new(Keys::default());
    let env = typing_env(&keys);
    on_paste_hotkey(&settings, &tracker, Arc::clone(&env), || 0x77)
        .join()
        .unwrap();
    tracker.opened(0x77);
    on_paste_hotkey(&settings, &tracker, env, || 0x77)
        .join()
        .unwrap();
    assert_eq!(*keys.typed.lock().unwrap(), "a\nba b");
    let delay = Duration::from_secs_f64(0.03);
    assert!(SLEPT.lock().unwrap().contains(&delay));
}
