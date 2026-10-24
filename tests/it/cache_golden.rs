//! 系統訊息快取：正規化、指紋、檔案位元組對照 Python golden，其餘行為用假 backend 驗證。

use crate::common;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wizard101_chat_translator::app_main::incoming_fingerprint;
use wizard101_chat_translator::config::Config;
use wizard101_chat_translator::translation::cache::{
    FLUSH_EVERY, MAX_ENTRIES, TranslationCache, fingerprint_of, normalize, placeholders_match,
    restore, translate_and_cache,
};
use wizard101_chat_translator::translation::clients::{ChatBackend, ChatRequest};
use wizard101_chat_translator::translation::errors::TranslateError;
use wizard101_chat_translator::translation::translator::Translator;
use wizard101_chat_translator::translation::{BoxFuture, Role};

const ZH: &str = "繁體中文（台灣）";
const FP: &str = "custom||gemma|繁體中文（台灣）|p6|x-";
/// 數字會被快取正規化掉，測試鍵改用純字母（與 golden 匯出同一算法）。
fn letters(i: usize) -> String {
    [i / 676 % 26, i / 26 % 26, i % 26]
        .iter()
        .map(|d| (b'a' + *d as u8) as char)
        .collect()
}

const SECRET_KEY: &str = "sk-test-secret-key";
const OTHER_FP: &str = "custom||gemma|日本語|p6|x-";

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

fn cache_in(dir: &tempfile::TempDir) -> TranslationCache {
    TranslationCache::new(dir.path().join("cache.json"), FP.into())
}

fn file_text(dir: &tempfile::TempDir) -> String {
    std::fs::read_to_string(dir.path().join("cache.json")).unwrap()
}

fn replay(cache: &TranslationCache, ops: &Value) {
    for op in ops.as_array().unwrap() {
        match s(&op["op"]) {
            "put" => assert!(cache.put(
                s(&op["text"]),
                "",
                s(&op["translated"]),
                &cache.fingerprint()
            )),
            _ => {
                cache.get(s(&op["text"]), "");
            }
        }
    }
}

#[test]
fn normalize_matches_python() {
    let cases = common::golden("cache_normalize");
    assert!(cases.len() >= 15);
    for c in cases {
        let (template, numbers) = normalize(s(&c["input"]));
        let out = &c["output"];
        assert_eq!(template, s(&out["template"]), "{}", c["input"]);
        let expected: Vec<&str> = out["numbers"].as_array().unwrap().iter().map(s).collect();
        assert_eq!(numbers, expected, "{}", c["input"]);
        assert_eq!(restore(&template, &numbers), s(&out["restored"]));
    }
}

#[test]
fn fingerprint_matches_python() {
    let cases = common::golden("cache_fingerprint");
    assert_eq!(cases.len(), 5);
    for c in cases {
        let i = &c["input"];
        let fp = fingerprint_of(
            s(&i["provider"]),
            s(&i["model"]),
            s(&i["target"]),
            s(&i["base_url"]),
            s(&i["digest"]),
        );
        assert_eq!(fp, s(&c["output"]));
    }
}

#[test]
fn flush_writes_the_golden_content() {
    let cases = common::golden("cache_file_bytes");
    assert_eq!(cases.len(), 7);
    for c in cases {
        let dir = tempfile::tempdir().unwrap();
        let cache = TranslationCache::new(
            dir.path().join("cache.json"),
            s(&c["input"]["fingerprint"]).into(),
        );
        replay(&cache, &c["input"]["ops"]);
        cache.flush();
        assert_eq!(
            common::json_content(&file_text(&dir)),
            common::json_content(s(&c["output"])),
            "{}",
            c["input"]["ops"]
        );
    }
}

#[test]
fn loads_an_old_format_cache() {
    for c in common::golden("cache_file_bytes") {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cache.json"), s(&c["output"])).unwrap();
        let cache = TranslationCache::new(
            dir.path().join("cache.json"),
            s(&c["input"]["fingerprint"]).into(),
        );
        cache.load();
        cache.flush();
        assert_eq!(
            common::json_content(&file_text(&dir)),
            common::json_content(s(&c["output"])),
            "load then flush must round-trip"
        );
    }
}

#[test]
fn load_discards_mismatched_corrupt_and_malformed_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.json");
    let body =
        |fp: &str, entries: &str| format!("{{\"fingerprint\": \"{fp}\", \"entries\": {entries}}}");
    for text in [
        body(OTHER_FP, "{\"熔岩百合\": \"x\"}"),
        "{ not json".to_string(),
        body(FP, "[\"not\", \"a\", \"dict\"]"),
        body(FP, "\"not a dict either\""),
        body(FP, "{\"熔岩百合\": 5}"),
        "[]".to_string(),
    ] {
        std::fs::write(&path, &text).unwrap();
        let cache = TranslationCache::new(path.clone(), FP.into());
        cache.load();
        assert_eq!(cache.get("熔岩百合", ""), None, "{text}");
    }
    std::fs::remove_file(&path).unwrap();
    let cache = TranslationCache::new(path.clone(), FP.into());
    cache.load();
    assert_eq!(cache.get("熔岩百合", ""), None);
}

#[test]
fn load_truncates_to_the_cap_keeping_the_newest() {
    common::log_capture::capture_logs();
    let dir = tempfile::tempdir().unwrap();
    let entries: Vec<String> = (0..MAX_ENTRIES + 2)
        .map(|i| format!("\"{0}\": \"{0}\"", letters(i)))
        .collect();
    let text = format!(
        "{{\"fingerprint\": \"{FP}\", \"entries\": {{{}}}}}",
        entries.join(", ")
    );
    std::fs::write(dir.path().join("cache.json"), text).unwrap();
    let cache = cache_in(&dir);
    cache.load();
    assert_eq!(cache.get(&letters(1), ""), None);
    assert_eq!(cache.get(&letters(2), ""), Some(letters(2)));
    let last = letters(MAX_ENTRIES + 1);
    assert_eq!(cache.get(&last, ""), Some(last));
    let path = dir.path().join("cache.json").display().to_string();
    let logged = common::log_capture::log_lines_with(&path);
    assert!(
        logged
            .iter()
            .any(|l| l.contains(&format!("loaded {MAX_ENTRIES} entries"))),
        "log reports the stored count, got {logged:?}"
    );
}

#[test]
fn put_get_round_trips_and_other_numbers_hit_the_same_entry() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    assert_eq!(cache.get("你获得了 39 金币！", ""), None);
    assert!(cache.put("你获得了 39 金币！", "", "你獲得了 {0} 金幣！", FP));
    assert_eq!(
        cache.get("你获得了 39 金币！", "").as_deref(),
        Some("你獲得了 39 金幣！")
    );
    assert_eq!(
        cache.get("你获得了 65 金币！", "").as_deref(),
        Some("你獲得了 65 金幣！")
    );
    assert_eq!(
        cache.get("你获得了 {0} 金币！", ""),
        None,
        "put takes raw text, not a template"
    );
}

#[test]
fn restore_follows_placeholder_numbers_not_positions() {
    let nums = ["3".to_string(), "39".to_string()];
    assert_eq!(restore("{1} gold and {0} XP", &nums), "39 gold and 3 XP");
    assert_eq!(restore("{5} stays", &nums), "{5} stays");
}

#[test]
fn placeholders_match_checks_counts() {
    assert!(placeholders_match(
        "你获得了 {0} 金币！",
        "你獲得了 {0} 金幣！"
    ));
    assert!(!placeholders_match("你获得了 {0} 金币！", "你獲得了金幣！"));
    assert!(!placeholders_match("熔岩百合", "熔岩百合 {0}"));
    assert!(!placeholders_match("{0} 金币", "{0} 金幣 {0}"));
}

#[test]
fn put_rejects_broken_placeholders() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    assert!(!cache.put("你获得了 39 金币！", "", "你獲得了金幣！", FP));
    assert_eq!(cache.get("你获得了 39 金币！", ""), None);
}

#[test]
fn fingerprint_change_clears_on_rebind() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    cache.put("熔岩百合", "", "熔岩百合(Lava Lily)", FP);
    cache.rebind(OTHER_FP);
    assert_eq!(cache.get("熔岩百合", ""), None);
    let saved: Value = serde_json::from_str(&file_text(&dir)).unwrap();
    assert_eq!(
        saved["fingerprint"], FP,
        "old content flushed under the old fingerprint"
    );
    assert_eq!(saved["entries"]["熔岩百合"], "熔岩百合(Lava Lily)");
    assert!(
        !cache.put("a", "", "A", FP),
        "stale fingerprint is rejected"
    );
    assert!(cache.put("a", "", "A", OTHER_FP));
}

#[test]
fn rebinding_back_restores_the_stashed_entries_and_file() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    cache.put("熔岩百合", "", "熔岩百合(Lava Lily)", FP);
    cache.rebind(OTHER_FP);
    cache.flush();
    cache.rebind(FP);
    assert_eq!(
        cache.get("熔岩百合", "").as_deref(),
        Some("熔岩百合(Lava Lily)")
    );
    let saved: Value = serde_json::from_str(&file_text(&dir)).unwrap();
    assert_eq!(saved["fingerprint"], FP);
    assert_eq!(saved["entries"]["熔岩百合"], "熔岩百合(Lava Lily)");
}

#[test]
fn rebind_to_the_same_fingerprint_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    cache.put("熔岩百合", "", "熔岩百合(Lava Lily)", FP);
    cache.rebind(FP);
    assert!(!dir.path().join("cache.json").exists());
    assert!(cache.get("熔岩百合", "").is_some());
}

#[test]
fn flushes_every_20_puts() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    for i in 0..FLUSH_EVERY - 1 {
        cache.put(&letters(i), "", "v", FP);
    }
    assert!(!dir.path().join("cache.json").exists());
    cache.put("last", "", "v", FP);
    assert!(dir.path().join("cache.json").exists());
}

#[test]
fn lru_caps_at_2000() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    for i in 0..MAX_ENTRIES {
        cache.put(&letters(i), "", "v", FP);
    }
    assert!(
        cache.get(&letters(0), "").is_some(),
        "touch the oldest so it survives"
    );
    cache.put("new", "", "v", FP);
    assert!(cache.get(&letters(0), "").is_some());
    assert!(
        cache.get(&letters(1), "").is_none(),
        "least recently used was evicted"
    );
    assert!(cache.get("new", "").is_some());
}

#[test]
fn clear_removes_entries_and_file_without_writing_back() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    cache.put("熔岩百合", "", "x", FP);
    cache.put("你获得了 39 金币！", "", "你獲得了 {0} 金幣！", FP);
    cache.flush();
    assert_eq!(cache.clear(), Some(2));
    assert!(!dir.path().join("cache.json").exists());
    assert_eq!(cache.get("熔岩百合", ""), None);
    cache.flush();
    let saved: Value = serde_json::from_str(&file_text(&dir)).unwrap();
    assert_eq!(saved["entries"], json!({}));
    assert_eq!(cache_in(&tempfile::tempdir().unwrap()).clear(), Some(0));
}

#[test]
fn clear_reports_failure_when_the_file_cannot_be_deleted() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("cache.json")).unwrap();
    assert_eq!(cache_in(&dir).clear(), None);
}

#[test]
fn a_cleared_cache_is_not_brought_back_by_switching_back() {
    // 使用者要求的清除不可被之後切回原指紋時換回暫存的舊條目
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    cache.put("熔岩百合", "", "熔岩百合(Lava Lily)", FP);
    cache.rebind(OTHER_FP);
    cache.put("迷幻木头", "", "迷幻木頭", OTHER_FP);
    assert_eq!(cache.clear(), Some(2), "live entries plus the stash");
    cache.rebind(FP);
    assert_eq!(cache.get("熔岩百合", ""), None);
}

#[test]
fn cache_still_usable_after_clear() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    cache.put("熔岩百合", "", "熔岩百合(Lava Lily)", FP);
    cache.clear();
    cache.put("迷幻木头", "", "迷幻木頭", FP);
    assert_eq!(cache.get("迷幻木头", "").as_deref(), Some("迷幻木頭"));
}

/// 收訊服務帶金鑰的完整設定，走真正的指紋組裝路徑（`incoming_fingerprint`）。
fn keyed_config() -> Config {
    let Value::Object(cfg) = json!({
        "services": [{"id": "aaaa0001", "name": "In", "provider": "custom",
            "base_url": "http://in", "model": "gemma", "api_key": SECRET_KEY, "thinking": false}],
        "default_service": "aaaa0001",
        "target_language": "日本語",
    }) else {
        unreachable!()
    };
    cfg
}

#[test]
fn fingerprint_never_contains_the_api_key() {
    let fp = incoming_fingerprint(&keyed_config(), None);
    assert!(fp.contains("gemma"), "{fp}");
    assert!(!fp.contains(SECRET_KEY), "{fp}");
}

#[test]
fn cache_file_never_contains_the_api_key() {
    let dir = tempfile::tempdir().unwrap();
    let cache = TranslationCache::new(
        dir.path().join("cache.json"),
        incoming_fingerprint(&keyed_config(), None),
    );
    cache.put("熔岩百合", "", "熔岩百合(Lava Lily)", &cache.fingerprint());
    cache.flush();
    assert!(file_text(&dir).contains("gemma"));
    assert!(!file_text(&dir).contains(SECRET_KEY));
}

/// 假 backend：依「最後一輪 user 內容」查表回覆，記錄每次請求；`hook` 在請求途中被呼叫。
struct Fake {
    replies: Mutex<Vec<(String, String)>>,
    requested: Mutex<Vec<String>>,
    hook: Mutex<VecDeque<Box<dyn FnOnce() + Send>>>,
}

impl Fake {
    fn new(replies: &[(&str, &str)]) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(
                replies
                    .iter()
                    .map(|(a, b)| (a.to_string(), b.to_string()))
                    .collect(),
            ),
            requested: Mutex::default(),
            hook: Mutex::default(),
        })
    }

    fn requested(&self) -> Vec<String> {
        self.requested.lock().unwrap().clone()
    }
}

impl ChatBackend for Fake {
    fn chat<'a>(
        &'a self,
        req: ChatRequest<'a>,
        _cancel: Option<&'a CancellationToken>,
    ) -> BoxFuture<'a, Result<String, TranslateError>> {
        let last = req.turns.last().unwrap();
        assert_eq!(last.role, Role::User);
        self.requested.lock().unwrap().push(last.content.clone());
        if let Some(hook) = self.hook.lock().unwrap().pop_front() {
            hook();
        }
        let reply = self
            .replies
            .lock()
            .unwrap()
            .iter()
            .find(|(k, _)| *k == last.content)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("unexpected request {:?}", last.content));
        Box::pin(async move { Ok(reply) })
    }

    fn model(&self) -> &str {
        "m"
    }
}

fn translator(fake: &Arc<Fake>) -> Arc<Translator> {
    translator_for(fake, ZH)
}

fn translator_for(fake: &Arc<Fake>, target: &str) -> Arc<Translator> {
    let fake = Arc::clone(fake);
    Translator::with_client(
        move |_| fake.clone(),
        common::fake_backend::spec("m"),
        target.into(),
    )
}

#[tokio::test]
async fn translate_and_cache_stores_the_normalized_template() {
    let fake = Fake::new(&[("你获得了 {0} 金币！", "你獲得了 {0} 金幣！")]);
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    let out = translate_and_cache(&translator(&fake), &cache, "你获得了 39 金币！")
        .await
        .unwrap();
    assert_eq!(out, "你獲得了 39 金幣！");
    assert_eq!(fake.requested(), ["你获得了 {0} 金币！"]);
    assert_eq!(
        cache.get("你获得了 65 金币！", "").as_deref(),
        Some("你獲得了 65 金幣！")
    );
}

#[tokio::test]
async fn does_not_store_on_placeholder_mismatch_and_translates_original() {
    let fake = Fake::new(&[
        ("你获得了 {0} 金币！", "你獲得了金幣！"),
        ("你获得了 39 金币！", "你獲得了 39 金幣！"),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    let out = translate_and_cache(&translator(&fake), &cache, "你获得了 39 金币！")
        .await
        .unwrap();
    assert_eq!(out, "你獲得了 39 金幣！");
    assert_eq!(
        fake.requested(),
        ["你获得了 {0} 金币！", "你获得了 39 金币！"]
    );
    assert_eq!(cache.get("你获得了 39 金币！", ""), None);
}

#[tokio::test]
async fn does_not_store_when_fingerprint_changed_mid_flight() {
    let fake = Fake::new(&[
        ("你获得了 {0} 金币！", "你獲得了 {0} 金幣！"),
        ("你获得了 39 金币！", "39 ゴールドを手に入れた！"),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let cache = Arc::new(cache_in(&dir));
    let c = Arc::clone(&cache);
    fake.hook
        .lock()
        .unwrap()
        .push_back(Box::new(move || c.rebind(OTHER_FP)));
    let out = translate_and_cache(&translator(&fake), &cache, "你获得了 39 金币！")
        .await
        .unwrap();
    assert_eq!(out, "39 ゴールドを手に入れた！");
    assert_eq!(cache.get("你获得了 39 金币！", ""), None);
    cache.flush();
    assert!(!file_text(&dir).contains("金幣"));
}

#[tokio::test]
async fn does_not_store_a_blank_translation() {
    let fake = Fake::new(&[("雪刺帽", "")]);
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    let out = translate_and_cache(&translator(&fake), &cache, "雪刺帽").await;
    assert_eq!(
        out,
        Err(TranslateError::BadOutput("model returned no text".into()))
    );
    assert_eq!(cache.get("雪刺帽", ""), None);
}

#[tokio::test]
async fn does_not_store_stray_latin() {
    let fake = Fake::new(&[("雪刺帽", "Snowspike Hat")]);
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    let out = translate_and_cache(&translator(&fake), &cache, "雪刺帽")
        .await
        .unwrap();
    assert_eq!(out, "Snowspike Hat");
    assert_eq!(cache.get("雪刺帽", ""), None);
    assert_eq!(
        fake.requested().len(),
        2,
        "translator's own strict retry only, no extra direct call"
    );
}

#[tokio::test]
async fn stores_a_translation_that_kept_the_source_english() {
    let fake = Fake::new(&[("death skeleturion", "死亡骷髏戰士")]);
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    translate_and_cache(&translator(&fake), &cache, "death skeleturion")
        .await
        .unwrap();
    assert_eq!(
        cache.get("death skeleturion", "").as_deref(),
        Some("死亡骷髏戰士")
    );
}

#[tokio::test]
async fn judges_with_the_target_captured_before_the_request() {
    let fake = Fake::new(&[("雪刺帽", "Sombrero de Nieve")]);
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_in(&dir);
    let tr = translator_for(&fake, "Español");
    let swapped = Arc::clone(&tr);
    fake.hook.lock().unwrap().push_back(Box::new(move || {
        swapped.reconfigure(common::fake_backend::spec("m"), ZH.into());
    }));
    let out = translate_and_cache(&tr, &cache, "雪刺帽").await.unwrap();
    assert_eq!(out, "Sombrero de Nieve");
    assert_eq!(
        cache.get("雪刺帽", "").as_deref(),
        Some("Sombrero de Nieve"),
        "judged against the target in force when the request left"
    );
}

#[test]
fn concurrent_puts_gets_and_flushes_leave_a_valid_file() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Arc::new(cache_in(&dir));
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let cache = Arc::clone(&cache);
            std::thread::spawn(move || {
                for i in 0..100 {
                    let key = letters(t * 100 + i);
                    assert!(cache.put(&key, "", &key.to_uppercase(), FP));
                    cache.get(&key, "");
                    cache.flush();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    // 並行 flush 的改名順序不定，較舊的快照可能最後落地；收尾再存一次
    cache.flush();
    let reloaded = cache_in(&dir);
    reloaded.load();
    for i in 0..800 {
        let key = letters(i);
        assert_eq!(reloaded.get(&key, ""), Some(key.to_uppercase()));
    }
    let leftovers = std::fs::read_dir(dir.path()).unwrap().count();
    assert_eq!(leftovers, 1, "no temp files left behind");
}
