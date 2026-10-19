//! 主程式接線：收訊指紋、翻譯端的建構、熱鍵註冊與套用設定、單一實例與關閉順序。

#[allow(dead_code)]
use crate::common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::{Duration, Instant};

use common::fake_backend::{Fake, JA, spec};
use common::log_capture::{capture_logs, log_lines_with};
use common::reader_fixture::{EXPECTED_PANIC, quiet_panics};

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wizard101_chat_translator::app_main::{
    InputWiring, Running, SUPERVISOR_JOIN_BUDGET, Translation, apply_settings, build_translation,
    claim_instance, hotkey_mask, incoming_fingerprint, install_paste, on_paste_hotkey,
    outgoing_translate, reconfigure_translation, region_runner, register_hotkeys, run_ui_then,
    shutdown_steps,
};
use wizard101_chat_translator::composer::paste::{TypingEnv, type_into_window, typing_in_progress};
use wizard101_chat_translator::config::services::resolve;
use wizard101_chat_translator::config::{Config, ServiceSlot, ServiceSpec, Settings, app_name};
use wizard101_chat_translator::i18n::t;
use wizard101_chat_translator::input::dispatch::{ChatInputTracker, Foreground};
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
use wizard101_chat_translator::translation::prompts::OUTGOING_LANGUAGE;
use wizard101_chat_translator::translation::translator::{ExampleCoordinator, Translator};
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

/// 設定存檔的一組測試環境：共用設定、磁碟上的範例、已建好的翻譯端。
///
/// log 是整個執行檔共用的，各測試以 `tag` 夾進模型名稱，靠它篩出自己的 log。
struct Saved {
    _dir: tempfile::TempDir,
    _rt: tokio::runtime::Runtime,
    store: Arc<ExampleStore>,
    settings: Arc<RwLock<Config>>,
    built: Translation,
    tag: &'static str,
}

/// 三格模型名稱都帶上 `tag` 的設定。
fn tagged(tag: &str) -> Config {
    let mut cfg = cfg();
    for (index, model) in ["qwen3", "gpt-x", "region-model"].iter().enumerate() {
        cfg["services"][index]["model"] = json!(format!("{model}-{tag}"));
    }
    cfg
}

/// 所有格的範例都已在磁碟上：建構與之後的重建都走快取、不碰網路。
fn saved(tag: &'static str) -> Saved {
    saved_from(tag, tagged(tag))
}

fn saved_from(tag: &'static str, cfg: Config) -> Saved {
    capture_logs();
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = Arc::new(ExampleStore::new(dir.path().join("examples.json")));
    seed(&store, &cfg, &examples());
    let settings = Arc::new(RwLock::new(cfg));
    let built = build_translation(
        &settings,
        no_results(),
        rt.handle(),
        Arc::clone(&store),
        dir.path().join("cache.json"),
    );
    Saved {
        _dir: dir,
        _rt: rt,
        store,
        settings,
        built,
        tag,
    }
}

/// 為 `cfg` 的收訊與框選兩格備妥磁碟上的範例。
fn seed(store: &ExampleStore, cfg: &Config, set: &ExampleSet) {
    let target = Settings::from_config(cfg).target_language;
    for slot in [ServiceSlot::Incoming, ServiceSlot::Region] {
        store.put(&examples_fingerprint(&resolve(cfg, slot), &target), set);
    }
}

impl Saved {
    /// 照呼叫端的規矩：先寫共用設定，再重設翻譯端；改完新設定的範例也已在磁碟上。
    fn save(&self, edit: impl FnOnce(&mut Config)) {
        let cfg = {
            let mut cfg = self.settings.write().unwrap();
            edit(&mut cfg);
            cfg.clone()
        };
        seed(&self.store, &cfg, &examples());
        reconfigure_translation(&cfg, &self.built);
    }

    fn describes(&self) -> [String; 3] {
        let b = &self.built;
        [&b.incoming, &b.outgoing, &b.region].map(|t| t.describe())
    }

    /// 這個測試重建過哪幾格（依 log 的 slot 鍵）。
    fn rebuilt_slots(&self) -> Vec<&'static str> {
        ServiceSlot::ALL
            .into_iter()
            .map(ServiceSlot::key)
            .filter(|key| {
                log_lines_with(&format!("[translate] {key} translator rebuilt"))
                    .iter()
                    .any(|line| line.contains(self.tag))
            })
            .collect()
    }
}

fn set_model(cfg: &mut Config, index: usize, model: &str) {
    cfg["services"][index]["model"] = json!(model);
}

#[test]
fn changing_only_the_region_slot_leaves_incoming_and_the_cache_alone() {
    let s = saved("region_only");
    let before = s.describes();
    let fingerprint = s.built.cache.fingerprint();
    s.save(|c| set_model(c, 2, "region-model-2-region_only"));
    let after = s.describes();
    assert!(after[2].contains("model=region-model-2-region_only"));
    assert_eq!((&after[0], &after[1]), (&before[0], &before[1]));
    assert_eq!(s.built.cache.fingerprint(), fingerprint);
    assert_eq!(s.rebuilt_slots(), ["region"]);
    let line = log_lines_with("[translate] region translator rebuilt");
    assert!(
        line.iter().any(|l| l.ends_with(
            "[translate] region translator rebuilt: \
             provider=custom, model=region-model-2-region_only"
        )),
        "{line:?}"
    );
}

#[test]
fn changing_the_incoming_slot_invalidates_the_cache() {
    let s = saved("incoming_change");
    s.save(|c| set_model(c, 0, "qwen3-2-incoming_change"));
    assert!(
        s.built
            .incoming
            .describe()
            .contains("model=qwen3-2-incoming_change")
    );
    assert_eq!(
        s.built.cache.fingerprint(),
        incoming_fingerprint(&s.settings.read().unwrap(), Some(&examples()))
    );
    assert!(
        s.built
            .cache
            .fingerprint()
            .contains("qwen3-2-incoming_change")
    );
    assert_eq!(s.rebuilt_slots(), ["incoming"]);
}

#[test]
fn reconfigure_ensures_examples_only_for_rebuilt_slots() {
    let s = saved("ensure_rebuilt");
    let newer = ExampleSet {
        system: Some(("s2".into(), "o2".into())),
        ..examples()
    };
    let mut changed = tagged("ensure_rebuilt");
    set_model(&mut changed, 2, "region-model-3-ensure_rebuilt");
    *s.settings.write().unwrap() = changed.clone();
    seed(&s.store, &changed, &newer);
    reconfigure_translation(&changed, &s.built);
    // 只有被重建的框選格換上新指紋的範例；收訊格沒被碰
    assert_eq!(s.built.region.examples(), Some(newer));
    assert_eq!(s.built.incoming.examples(), Some(examples()));
    assert_eq!(s.built.outgoing.examples(), None);
}

#[test]
fn reconfigure_without_changes_keeps_examples_and_the_cache() {
    let s = saved("no_change");
    let fingerprint = s.built.cache.fingerprint();
    s.save(|_| {});
    assert!(s.rebuilt_slots().is_empty());
    assert_eq!(s.built.incoming.examples(), Some(examples()));
    assert_eq!(s.built.region.examples(), Some(examples()));
    assert_eq!(s.built.cache.fingerprint(), fingerprint);
}

#[test]
fn saving_unrelated_settings_rebuilds_no_translator() {
    let s = saved("unrelated");
    let before = s.describes();
    s.save(|c| {
        c.insert("overlay_alpha".into(), json!(0.5));
    });
    assert_eq!(s.describes(), before);
    assert!(s.rebuilt_slots().is_empty());
}

#[test]
fn editing_the_service_a_slot_uses_rebuilds_it() {
    for (field, tag, value) in [
        ("model", "edit_model", "gpt-y-edit_model"),
        ("api_key", "edit_api_key", "sk-edit_api_key"),
        (
            "base_url",
            "edit_base_url",
            "http://elsewhere-edit_base_url",
        ),
    ] {
        let s = saved(tag);
        // 發話格跟隨預設服務（第二筆）；金鑰與端點不在 describe 裡，以重建與否判斷
        s.save(|c| c["services"][1][field] = json!(value));
        assert_eq!(s.rebuilt_slots(), ["outgoing"], "{field}");
        if field == "model" {
            assert!(
                s.built
                    .outgoing
                    .describe()
                    .contains("model=gpt-y-edit_model")
            );
        }
        assert!(
            log_lines_with("sk-edit_api_key").is_empty(),
            "the key must never reach the log"
        );
    }
}

#[test]
fn changing_the_target_language_rebuilds_every_translator() {
    let s = saved("target_language");
    s.save(|c| {
        c.insert("target_language".into(), json!("한국어"));
    });
    for t in [&s.built.incoming, &s.built.outgoing, &s.built.region] {
        assert_eq!(t.target_language(), "한국어");
    }
    assert_eq!(s.rebuilt_slots(), ["incoming", "outgoing", "region"]);
    assert_eq!(s.built.incoming.examples(), Some(examples()));
    assert_eq!(s.built.region.examples(), Some(examples()));
    assert_eq!(s.built.outgoing.examples(), None);
}

#[test]
fn switching_the_incoming_slot_between_same_named_models_invalidates_the_cache() {
    let mut twin = tagged("twin");
    twin["services"].as_array_mut().unwrap().push(json!(
        {"id": "aaaa0004", "name": "Remote", "provider": "custom",
         "base_url": "http://remote-twin", "model": "qwen3-twin", "api_key": "",
         "thinking": false}
    ));
    let s = saved_from("twin", twin);
    let before = s.built.cache.fingerprint();
    s.save(|c| c["service_slots"]["incoming"] = json!("aaaa0004"));
    assert_ne!(s.built.cache.fingerprint(), before);
    assert!(s.built.cache.fingerprint().contains("http://remote-twin"));
}

/// 三格各指向自己的假伺服器：只有收訊那台會回覆，其餘兩台若被呼叫會留下請求紀錄。
struct Endpoints {
    rt: tokio::runtime::Runtime,
    servers: Vec<wiremock::MockServer>,
}

impl Endpoints {
    fn start() -> Self {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let rt = tokio::runtime::Runtime::new().unwrap();
        let servers: Vec<_> = (0..3).map(|_| rt.block_on(MockServer::start())).collect();
        let sse = "data: {\"choices\": [{\"delta\": {\"content\": \"嗨\"}}]}\n\ndata: [DONE]\n\n";
        rt.block_on(
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
                .mount(&servers[0]),
        );
        Self { rt, servers }
    }

    /// 三格服務都是自訂端點，網址指向各自的假伺服器。
    fn config(&self) -> Config {
        let mut cfg = cfg();
        for (index, model) in ["qwen3", "gpt-x", "region-model"].iter().enumerate() {
            cfg["services"][index] = json!({
                "id": format!("aaaa000{}", index + 1), "name": format!("Slot {index}"),
                "provider": "custom", "base_url": self.servers[index].uri(),
                "model": model, "api_key": "", "thinking": false,
            });
        }
        cfg
    }

    fn requests(&self, index: usize) -> usize {
        self.rt
            .block_on(self.servers[index].received_requests())
            .unwrap()
            .len()
    }
}

/// 建好翻譯端、照 `submit` 送進翻譯池，收齊 `expected` 筆（msg_id，譯文，是否失敗）。
fn deliveries_through(
    endpoints: &Endpoints,
    submit: impl FnOnce(&Translation),
    expected: usize,
) -> Vec<(u64, String, bool)> {
    capture_logs();
    let cfg = endpoints.config();
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ExampleStore::new(dir.path().join("examples.json")));
    seed(&store, &cfg, &examples());
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let deliver: ResultFn = Arc::new(move |id, text, failed| {
        let _ = tx.lock().unwrap().send((id, text, failed));
    });
    let built = build_translation(
        &Arc::new(RwLock::new(cfg)),
        deliver,
        endpoints.rt.handle(),
        store,
        dir.path().join("cache.json"),
    );
    submit(&built);
    (0..expected)
        .map(|_| rx.recv_timeout(common::WAIT).expect("delivery"))
        .collect()
}

#[test]
fn both_pools_translate_through_the_incoming_service() {
    let endpoints = Endpoints::start();
    let results = deliveries_through(
        &endpoints,
        |built| {
            built.pool.submit("[Amy] hi".into(), vec![], 1);
            built
                .system_pool
                .submit("Server restarting".into(), vec![], 2);
        },
        2,
    );
    assert!(results.iter().all(|(_, _, failed)| !failed), "{results:?}");
    assert_eq!(
        [
            endpoints.requests(0),
            endpoints.requests(1),
            endpoints.requests(2)
        ],
        [2, 0, 0],
        "both pools translate through the incoming service only"
    );
}

#[test]
fn the_system_pool_translates_through_the_incoming_service() {
    let endpoints = Endpoints::start();
    let results = deliveries_through(
        &endpoints,
        |built| {
            built
                .system_pool
                .submit("Server restarting".into(), vec![], 7);
        },
        1,
    );
    assert_eq!(results[0].0, 7);
    assert!(!results[0].2, "{results:?}");
    assert_eq!(
        [
            endpoints.requests(0),
            endpoints.requests(1),
            endpoints.requests(2)
        ],
        [1, 0, 0]
    );
}

#[test]
fn build_app_wires_each_consumer_to_its_own_slot() {
    let source = include_str!("../../src/main.rs");
    assert!(source.contains("Arc::clone(&built.translation.outgoing)"));
    assert!(source.contains("Arc::clone(&built.translation.region)"));
}

#[test]
fn build_app_spawns_readers_through_the_supervisor() {
    let source = include_str!("../../src/app_main.rs");
    assert!(source.contains("supervise(&stop, spawn, game_windows, &board)"));
    assert!(source.contains("MessageLog::new(Arc::clone(&messages), slot)"));
}

#[test]
fn reconfigure_resizes_both_pools() {
    let s = saved("resize");
    s.save(|c| {
        c.insert("max_parallel_translations".into(), json!(7));
    });
    assert_eq!(
        log_lines_with("[translate] pool resized to 7 workers").len(),
        2
    );
}

/// 單執行緒 runtime 在 `block_on` 之前不跑背景工作：生成停在「已發出、未完成」，由測試決定何時晚到。
fn paused_rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// 讓 `rt` 上的背景工作跑到 `done` 成立為止，最多等 [`common::WAIT`]。
fn run_until(rt: &tokio::runtime::Runtime, what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + common::WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting: {what}");
        rt.block_on(async { tokio::time::sleep(Duration::from_millis(10)).await });
    }
}

/// 目標語言是遊戲原生語言時，範例生成不打網路、直接給內建的一組。
fn offline_cfg(tag: &str) -> Config {
    let mut cfg = tagged(tag);
    cfg.insert("target_language".into(), json!(OUTGOING_LANGUAGE));
    cfg
}

#[test]
fn a_stale_generation_is_never_applied_and_lands_only_under_the_old_fingerprint() {
    capture_logs();
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = Arc::new(ExampleStore::new(dir.path().join("examples.json")));
    let applied = Arc::new(Mutex::new(Vec::new()));
    let coordinator = {
        let applied = Arc::clone(&applied);
        ExampleCoordinator::new(
            Arc::clone(&store),
            rt.handle().clone(),
            Arc::new(move |tr: &Translator| applied.lock().unwrap().push(tr.examples())),
        )
    };
    let fake = Fake::new("m", vec![Ok(JA.to_owned())]);
    let tr = {
        let fake = Arc::clone(&fake);
        Translator::with_client(move |_| fake.clone(), spec("m"), "日本語".into())
    };
    let old_fingerprint = tr.examples_fingerprint();
    let new = ExampleSet {
        incoming: Some(("s".into(), "new".into())),
        ..examples()
    };
    // 生成請求途中使用者按儲存：換服務、新指紋的範例已在快取，ensure 直接套用
    fake.on_call({
        let (weak, coordinator, store, new) = (
            Arc::downgrade(&tr),
            Arc::clone(&coordinator),
            Arc::clone(&store),
            new.clone(),
        );
        move |_| {
            let tr = weak.upgrade().unwrap();
            assert!(tr.reconfigure(spec("m2"), "日本語".into()));
            store.put(&tr.examples_fingerprint(), &new);
            assert!(
                coordinator.ensure(&tr).is_none(),
                "the cache hit needs no generation"
            );
        }
    });

    let generation = coordinator
        .ensure(&tr)
        .expect("the old fingerprint is generated");
    rt.block_on(async { generation.await.unwrap() });

    let new_fingerprint = tr.examples_fingerprint();
    assert_ne!(new_fingerprint, old_fingerprint);
    assert_eq!(
        tr.examples(),
        Some(new.clone()),
        "the stale set is not applied"
    );
    assert_eq!(
        *applied.lock().unwrap(),
        [Some(new.clone())],
        "only the new set was applied"
    );
    assert_eq!(store.get(&new_fingerprint), Some(new));
    assert!(
        store
            .get(&old_fingerprint)
            .is_some_and(|set| set.complete()),
        "the stale result is kept under the old fingerprint"
    );
    // finish() 自己就挑不出符合的 translator；set_examples 的防線另由 tests/it/translator.rs 釘住
    let not_applied = format!(
        "[translate] examples stored but not applied, no translator matches (fingerprint={old_fingerprint})"
    );
    assert_eq!(log_lines_with(&not_applied).len(), 1);
    let rejected = format!("ignoring examples for a stale fingerprint: {old_fingerprint}");
    assert!(log_lines_with(&rejected).is_empty());
}

#[test]
fn late_examples_after_a_save_bind_the_cache_to_the_new_fingerprint() {
    capture_logs();
    let dir = tempfile::tempdir().unwrap();
    let rt = paused_rt();
    let mut cfg = offline_cfg("late_bind");
    let settings = Arc::new(RwLock::new(cfg.clone()));
    let store = Arc::new(ExampleStore::new(dir.path().join("examples.json")));
    let built = build_translation(
        &settings,
        no_results(),
        rt.handle(),
        store,
        dir.path().join("cache.json"),
    );

    // 新設定的範例不在磁碟上：生成在存檔之後才跑完，回呼讀到的必須是新設定
    set_model(&mut cfg, 0, "qwen3-new-late_bind");
    *settings.write().unwrap() = cfg.clone();
    reconfigure_translation(&cfg, &built);
    assert!(built.incoming.examples().is_none());
    assert!(built.cache.fingerprint().ends_with("x-"));

    // 生成套用後才 rebind，指紋帶上範例就代表兩步都做完了
    run_until(&rt, "the late generation rebinds the cache", || {
        !built.cache.fingerprint().ends_with("x-")
    });
    let applied = built.incoming.examples();
    assert!(applied.is_some(), "the late generation was applied");
    let fingerprint = built.cache.fingerprint();
    assert_eq!(fingerprint, incoming_fingerprint(&cfg, applied.as_ref()));
    assert!(fingerprint.contains("qwen3-new-late_bind"));
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
        foreground: || Foreground {
            hwnd: || 0x77,
            exe: || Some(r"C:\Wizard101\Bin\WizardGraphicalClient.exe".into()),
        },
    }
}

fn no_mask() -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(|| {})
}

#[test]
fn register_hotkeys_writes_the_effective_hotkeys_back() {
    capture_logs();
    let (ui, _rx) = UiPoster::for_test();
    let settings = input_cfg("ctrl+nope", "ctrl+space", false);
    let (_hotkey, region) =
        register_hotkeys(&settings, &ui, &dispatcher(), &wiring(false), no_mask());
    assert!(region.is_none());
    let cfg = settings.read().unwrap();
    assert_eq!(cfg["hotkey"], "ctrl+space");
    assert_eq!(cfg["region_hotkey"], "ctrl+space");
    drop(cfg);

    let settings = input_cfg("ctrl+space", "ctrl+bogus", false);
    let (_hotkey, region) =
        register_hotkeys(&settings, &ui, &dispatcher(), &wiring(false), no_mask());
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
            raw: None,
        });
    }
    for name in names.iter().rev() {
        d.process(KeyEvent {
            down: false,
            code: key(name),
            raw: None,
        });
    }
    passed
}

#[test]
fn paste_installed_after_the_hotkeys_runs_last() {
    let _serial = TYPING_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    capture_logs();
    let (ui, rx) = UiPoster::for_test();
    let settings = input_cfg("ctrl+v", "shift+v", false);
    let d = dispatcher();
    let (keys, input) = (Arc::new(Keys::default()), wiring(true));
    let typing = typing_env(&keys);
    let _registered = register_hotkeys(&settings, &ui, &d, &input, hotkey_mask(&typing));
    install_paste(&settings, &d, &input, typing);

    // 鍵入進行中，貼上攔截器會吞掉每一顆 v；排在它前面的熱鍵才收得到
    let (release, gate) = mpsc::channel();
    let blocked = Arc::new(Keys::default());
    *blocked.gate.lock().unwrap() = Some(gate);
    let env = typing_env(&blocked);
    let typing = std::thread::spawn(move || type_into_window(&env, 0, "x", Duration::ZERO));
    let deadline = Instant::now() + common::WAIT;
    while !typing_in_progress() {
        assert!(Instant::now() < deadline, "typing never started");
        std::thread::yield_now();
    }

    assert!(!press(&d, &["ctrl", "v"]));
    assert!(
        matches!(rx.try_recv(), Ok(UiEvent::ShowInputBox { .. })),
        "the input box hotkey runs before paste"
    );
    assert!(!press(&d, &["shift", "v"]));
    assert!(
        matches!(rx.try_recv(), Ok(UiEvent::RegionHotkey { .. })),
        "the region hotkey runs before paste"
    );
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
        rx.recv_timeout(common::WAIT).unwrap(),
        UiEvent::OutgoingDone {
            session: 3,
            result: Ok("hi".into())
        }
    );
    let rx = o.submit("hello again", 0x99, 4);
    rx.recv_timeout(common::WAIT).unwrap();

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
        rx.recv_timeout(common::WAIT).unwrap(),
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
    let deadline = Instant::now() + common::WAIT;
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
        rx.recv_timeout(common::WAIT).unwrap(),
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
    let deadline = Instant::now() + common::WAIT;
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
        rx.recv_timeout(common::WAIT).unwrap(),
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
        rx.recv_timeout(common::WAIT).unwrap(),
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
    let _registered = register_hotkeys(&settings, &ui, &d, &wiring, no_mask());
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

/// 已接好線的執行期：tempdir 的設定檔、不裝 hook 的分派器、範例都在磁碟上（不碰網路）。
struct Live {
    dir: tempfile::TempDir,
    _rt: tokio::runtime::Runtime,
    store: Arc<ExampleStore>,
    settings: Arc<RwLock<Config>>,
    running: Running,
    ui: UiPoster,
    rx: Receiver<UiEvent>,
    d: SharedDispatcher,
}

const OVERLAY: &str = r#"{"x": 10, "y": 20, "width": 300, "height": 400}"#;

fn live(tag: &'static str) -> Live {
    capture_logs();
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = Arc::new(ExampleStore::new(dir.path().join("examples.json")));
    let mut cfg = tagged(tag);
    cfg.insert("hotkey".into(), json!("ctrl+alt+j"));
    cfg.insert("region_hotkey".into(), json!("ctrl+alt+r"));
    cfg.insert("paste_hotkey".into(), json!(false));
    cfg.insert("overlay".into(), serde_json::from_str(OVERLAY).unwrap());
    seed(&store, &cfg, &examples());
    let settings = Arc::new(RwLock::new(cfg));
    let translation = build_translation(
        &settings,
        no_results(),
        rt.handle(),
        Arc::clone(&store),
        dir.path().join("cache.json"),
    );
    let (ui, rx) = UiPoster::for_test();
    let d = dispatcher();
    let input = wiring(true);
    let typing = typing_env(&Arc::new(Keys::default()));
    let hotkeys = register_hotkeys(&settings, &ui, &d, &input, hotkey_mask(&typing));
    install_paste(&settings, &d, &input, Arc::clone(&typing));
    let running = Running {
        stop: CancellationToken::new(),
        translation,
        supervisor: None,
        keyboard: None,
        contexts: ContextMap::default(),
        input,
        typing,
        hotkeys,
        dispatcher: Arc::clone(&d),
        config_path: dir.path().join("config.json"),
    };
    Live {
        dir,
        _rt: rt,
        store,
        settings,
        running,
        ui,
        rx,
        d,
    }
}

impl Live {
    /// 合併後的完整設定，先把它的範例放上磁碟再套用。
    fn apply(&mut self, partial: Config) -> (String, Option<String>) {
        let mut merged = self.settings.read().unwrap().clone();
        merged.extend(partial.clone());
        seed(&self.store, &merged, &examples());
        apply_settings(&mut self.running, &self.settings, partial, &self.ui)
    }

    fn input_box_shown(&self) -> usize {
        self.rx
            .try_iter()
            .filter(|e| matches!(e, UiEvent::ShowInputBox { .. }))
            .count()
    }

    fn handlers(&self) -> usize {
        self.d.lock().unwrap().handler_count()
    }

    fn saved_file(&self) -> Value {
        let text = std::fs::read_to_string(self.dir.path().join("config.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }
}

fn partial(pairs: &[(&str, Value)]) -> Config {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

#[test]
fn apply_settings_saves_rebinds_and_reregisters() {
    let mut l = live("apply_rebind");
    assert!(!press(&l.d, &["ctrl", "alt", "j"]));
    assert_eq!(l.input_box_shown(), 1);
    let handlers = l.handlers();

    let mut services = l.settings.read().unwrap()["services"].clone();
    services[0]["model"] = json!("qwen3-2-apply_rebind");
    let (hotkey, region) = l.apply(partial(&[
        ("hotkey", json!("ctrl+alt+k")),
        ("services", services),
    ]));

    assert_eq!(hotkey, "ctrl+alt+k");
    assert_eq!(region.as_deref(), Some("ctrl+alt+r"));
    assert_eq!(l.handlers(), handlers);
    assert!(press(&l.d, &["ctrl", "alt", "j"]), "the old hotkey passes");
    assert_eq!(l.input_box_shown(), 0);
    assert!(!press(&l.d, &["ctrl", "alt", "k"]));
    assert_eq!(l.input_box_shown(), 1);

    let saved = l.saved_file();
    assert_eq!(saved["hotkey"], "ctrl+alt+k");
    assert_eq!(saved["services"][0]["model"], "qwen3-2-apply_rebind");
    let fingerprint = l.running.translation.cache.fingerprint();
    assert_eq!(
        fingerprint,
        incoming_fingerprint(&l.settings.read().unwrap(), Some(&examples()))
    );
    assert!(fingerprint.contains("qwen3-2-apply_rebind"));
    let applied = log_lines_with("[settings] applied; ");
    assert!(
        applied.iter().any(|l| l.contains("qwen3-2-apply_rebind")),
        "{applied:?}"
    );
}

#[test]
fn apply_settings_merges_the_partial_map_into_the_live_config() {
    let mut l = live("apply_merge");
    l.apply(partial(&[("max_messages", json!(50))]));
    let overlay: Value = serde_json::from_str(OVERLAY).unwrap();
    assert_eq!(l.settings.read().unwrap()["overlay"], overlay);
    assert_eq!(l.settings.read().unwrap()["max_messages"], 50);
    let saved = l.saved_file();
    assert_eq!(saved["overlay"], overlay);
    assert_eq!(saved["max_messages"], 50);
    assert_eq!(saved["hotkey"], "ctrl+alt+j");
}

#[test]
fn region_hotkey_equal_to_the_input_hotkey_is_not_registered() {
    let mut l = live("apply_collide");
    let handlers = l.handlers();
    let (hotkey, region) = l.apply(partial(&[("region_hotkey", json!("ctrl+alt+j"))]));
    assert_eq!(hotkey, "ctrl+alt+j");
    assert_eq!(region, None);
    assert!(l.running.hotkeys.1.is_none());
    assert_eq!(l.handlers(), handlers - 1);
    assert!(
        press(&l.d, &["ctrl", "alt", "r"]),
        "the old region hotkey passes"
    );
    assert!(!press(&l.d, &["ctrl", "alt", "j"]));
    assert_eq!(l.input_box_shown(), 1);

    let (ui, _rx) = UiPoster::for_test();
    let d = dispatcher();
    let settings = input_cfg("ctrl+space", "ctrl+space", false);
    let (_hotkey, region) = register_hotkeys(&settings, &ui, &d, &wiring(false), no_mask());
    assert!(region.is_none());
    assert_eq!(d.lock().unwrap().handler_count(), 1);
}
