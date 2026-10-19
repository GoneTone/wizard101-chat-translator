//! Translator 與 ExampleCoordinator 的行為測試：假 backend 記錄收到的 system 與對話輪，不打真 API。

#[allow(dead_code)]
use crate::common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

use common::fake_backend::{Fake, JA, spec};
use common::log_capture::{log_lines_with, log_text};
use common::reader_fixture::{EXPECTED_PANIC, quiet_panics};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::config::ServiceSpec;
use wizard101_chat_translator::translation::Turn;
use wizard101_chat_translator::translation::clients::{ChatBackend, MAX_TOKENS_REGION};
use wizard101_chat_translator::translation::errors::TranslateError;
use wizard101_chat_translator::translation::examples::{
    ExampleSet, ExampleStore, GAME_LANGUAGE_EXAMPLES, examples_fingerprint,
};
use wizard101_chat_translator::translation::prompts::{
    CONTEXT_ACK, OUTGOING_LANGUAGE, build_incoming_system, build_outgoing_system,
    build_region_system, build_system_message_system, fewshot_outgoing_turns,
};
use wizard101_chat_translator::translation::translator::{
    ExampleCoordinator, TEST_SAMPLE, Translator, generate_and_store, test_translate,
};

const ZH: &str = "繁體中文（台灣）";

/// 每次（含 reconfigure）都接到同一個假 backend。
fn make_with(fake: &Arc<Fake>, target: &str) -> Arc<Translator> {
    let fake = Arc::clone(fake);
    Translator::with_client(move |_| fake.clone(), spec("m"), target.into())
}

fn make(fake: &Arc<Fake>) -> Arc<Translator> {
    make_with(fake, ZH)
}

/// 第一個 backend 是 `first`，之後 reconfigure 重建的都是 `late`（看請求有沒有跑去新設定）。
fn make_with_late(first: &Arc<Fake>, target: &str) -> (Arc<Translator>, Arc<Fake>) {
    let late = Fake::new("m2", vec![Ok("late".into())]);
    let (first, l) = (Arc::clone(first), Arc::clone(&late));
    let built = AtomicUsize::new(0);
    let tr = Translator::with_client(
        move |s| -> Arc<dyn ChatBackend> {
            if built.fetch_add(1, Ordering::SeqCst) == 0 {
                first.clone()
            } else {
                assert_eq!(s.model, "m2");
                l.clone()
            }
        },
        spec("m"),
        target.into(),
    );
    (tr, late)
}

fn pair(a: &str, b: &str) -> Option<(String, String)> {
    Some((a.into(), b.into()))
}

fn ex_incoming(a: &str, b: &str) -> ExampleSet {
    ExampleSet {
        incoming: pair(a, b),
        ..ExampleSet::default()
    }
}

/// 請求途中把 translator 換成 m2／`target`（使用者按下儲存）。
fn reconfigure_on_call(fake: &Fake, tr: &Arc<Translator>, at: usize, target: &'static str) {
    let weak: Weak<Translator> = Arc::downgrade(tr);
    fake.on_call(move |n| {
        if n == at {
            weak.upgrade()
                .unwrap()
                .reconfigure(spec("m2"), target.into());
        }
    });
}

// --- 收訊 ---

#[tokio::test]
async fn incoming_uses_context_and_examples() {
    let fake = Fake::texts(&["[A] 譯文"]);
    let tr = make(&fake);
    assert!(tr.set_examples(Some(ex_incoming("src", "out")), &tr.examples_fingerprint()));
    tr.translate_incoming("[A] hi", &["[B] yo".into()])
        .await
        .unwrap();
    let calls = fake.calls();
    assert_eq!(calls[0].system, build_incoming_system(ZH));
    assert_eq!(calls[0].turns.len(), 5);
    assert_eq!(
        calls[0].turns[..2],
        [Turn::user("src"), Turn::assistant("out")]
    );
    assert!(calls[0].turns[2].content.contains("[B] yo"));
    assert_eq!(calls[0].turns[3], Turn::assistant(CONTEXT_ACK));
    assert_eq!(calls[0].turns[4], Turn::user("[A] hi"));
    assert!(calls[0].deterministic && calls[0].max_tokens.is_none());
}

#[tokio::test]
async fn incoming_without_context_is_single_turn_and_keeps_no_history() {
    let fake = Fake::texts(&["譯文"]);
    let tr = make(&fake);
    tr.translate_incoming("[A] one", &[]).await.unwrap();
    tr.translate_incoming("[B] two", &[]).await.unwrap();
    assert_eq!(fake.calls()[1].turns, [Turn::user("[B] two")]);
}

#[tokio::test]
async fn incoming_strips_invented_english() {
    let fake = Fake::texts(&["[艾米] 迷幻木頭(Mystic Wood)"]);
    let out = make(&fake).translate_incoming("[艾米] 迷幻木头", &[]).await;
    assert_eq!(out.unwrap(), "[艾米] 迷幻木頭");
}

#[tokio::test]
async fn incoming_keeps_the_original_sender_and_logs_the_change() {
    let fake = Fake::texts(&["[賈斯汀 渡鴉] 嗨"]);
    let out = make(&fake)
        .translate_incoming("[贾斯廷 渡鸦] hi", &[])
        .await;
    assert_eq!(out.unwrap(), "[贾斯廷 渡鸦] 嗨");
    let lines = log_lines_with("[贾斯廷 渡鸦] hi");
    assert!(
        lines
            .iter()
            .any(|l| l.contains("[translate] model altered the sender prefix, restored: source='[贾斯廷 渡鸦] hi' model_output='[賈斯汀 渡鴉] 嗨'")),
        "{lines:?}"
    );
}

#[tokio::test]
async fn only_whitespace_after_the_sender_is_not_logged_as_altered() {
    let fake = Fake::texts(&["[Amy]嗨"]);
    let out = make(&fake)
        .translate_incoming("[Amy] whitespace-only", &[])
        .await;
    assert_eq!(out.unwrap(), "[Amy] 嗨");
    let lines = log_lines_with("whitespace-only");
    assert!(
        !lines.iter().any(|l| l.contains("altered the sender")),
        "{lines:?}"
    );
}

#[tokio::test]
async fn incoming_translation_is_not_retried() {
    let fake = Fake::texts(&["Snowspike Hat"]);
    make(&fake).translate_incoming("雪刺帽", &[]).await.unwrap();
    assert_eq!(fake.calls().len(), 1);
}

#[tokio::test]
async fn incoming_logs_the_source_and_the_result() {
    let fake = Fake::texts(&["[A] 譯文-log"]);
    make(&fake)
        .translate_incoming("[A] log-me", &["[B] yo".into(), "[A] sup".into()])
        .await
        .unwrap();
    let lines = log_lines_with("log-me");
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = &lines[0];
    assert!(line.contains("[translate] incoming done in "), "{line}");
    assert!(
        line.ends_with("s (model=m, ctx=2): source='[A] log-me' translated='[A] 譯文-log'"),
        "{line}"
    );
}

// --- 發話 ---

#[tokio::test]
async fn outgoing_uses_fixed_fewshot_and_english() {
    let fake = Fake::texts(&["sure"]);
    let tr = make(&fake);
    tr.translate_outgoing("好啊", &["[A] want to trade?".into()], None)
        .await
        .unwrap();
    tr.translate_outgoing("在嗎", &[], None).await.unwrap();
    let calls = fake.calls();
    let fewshot = fewshot_outgoing_turns();
    for call in calls.iter() {
        assert_eq!(call.system, build_outgoing_system(OUTGOING_LANGUAGE));
        assert_eq!(call.turns[..fewshot.len()], fewshot[..]);
    }
    assert!(
        calls[0]
            .turns
            .iter()
            .any(|t| t.content.contains("[A] want to trade?"))
    );
    assert_eq!(calls[0].turns.last(), Some(&Turn::user("好啊")));
    assert_eq!(calls[1].turns.len(), fewshot.len() + 1);
}

#[tokio::test]
async fn outgoing_logs_how_many_context_lines_it_carried() {
    let fake = Fake::texts(&["hello"]);
    make(&fake)
        .translate_outgoing("哈囉-ctx", &["[B] yo".into()], None)
        .await
        .unwrap();
    let lines = log_lines_with("哈囉-ctx");
    assert!(
        lines
            .iter()
            .any(|l| l.contains("[translate] outgoing done in") && l.contains("ctx=1")),
        "{lines:?}"
    );
}

#[tokio::test]
async fn a_cancelled_request_is_never_sent() {
    let fake = Fake::texts(&["hello"]);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let out = make(&fake)
        .translate_outgoing("哈囉", &[], Some(&cancel))
        .await;
    assert_eq!(out, Err(TranslateError::Cancelled));
    assert!(fake.calls().is_empty());
}

// --- 系統訊息 ---

#[tokio::test]
async fn system_message_sends_no_context_turns_and_strips_invented_english() {
    let fake = Fake::texts(&["迷幻木頭(Mystic Wood)"]);
    let out = make(&fake).translate_system_message("迷幻木头").await;
    assert_eq!(out.unwrap(), "迷幻木頭");
    let calls = fake.calls();
    assert_eq!(calls[0].system, build_system_message_system(ZH, false));
    assert_eq!(calls[0].turns, [Turn::user("迷幻木头")]);
}

#[tokio::test]
async fn system_message_strict_retry_on_stray_latin_only_once() {
    let fake = Fake::texts(&["Snowspike Hat", "雪刺帽"]);
    let out = make(&fake).translate_system_message("雪刺帽").await;
    assert_eq!(out.unwrap(), "雪刺帽");
    assert_eq!(fake.calls().len(), 2);
    assert_eq!(
        fake.calls()[1].system,
        build_system_message_system(ZH, true)
    );

    let still_english = Fake::texts(&["Calamity 現在等級 {0}！"]);
    let out = make(&still_english)
        .translate_system_message("卡拉米蒂 现在等级 {0}！")
        .await;
    assert_eq!(out.unwrap(), "Calamity 現在等級 {0}！");
    assert_eq!(still_english.calls().len(), 2);
    let lines = log_lines_with("卡拉米蒂 现在等级");
    assert!(lines.iter().any(|l| l.contains(
        "[translate] system message is not in the target language, retrying strictly: source='卡拉米蒂 现在等级 {0}！' translated='Calamity 現在等級 {0}！'"
    )), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains(
        "[translate] strict retry is still not in the target language, using it as is: source='卡拉米蒂 现在等级 {0}！'"
    )), "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|l| l.contains("[translate] system message done in"))
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("[translate] system message (strict retry) done in"))
    );
}

#[tokio::test]
async fn system_message_does_not_retry_a_clean_translation() {
    let fake = Fake::texts(&["雪刺帽"]);
    make(&fake)
        .translate_system_message("雪刺帽")
        .await
        .unwrap();
    assert_eq!(fake.calls().len(), 1);
}

#[tokio::test]
async fn strict_retry_sends_the_same_example_turns() {
    let fake = Fake::texts(&["Gained gold"]);
    let tr = make(&fake);
    let examples = ExampleSet {
        system: pair("s", "o"),
        ..ExampleSet::default()
    };
    tr.set_examples(Some(examples), &tr.examples_fingerprint());
    tr.translate_system_message("Gained gold").await.unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), 2);
    for call in calls.iter() {
        assert_eq!(call.turns[..2], [Turn::user("s"), Turn::assistant("o")]);
    }
}

#[tokio::test]
async fn a_reconfigure_mid_request_does_not_mix_settings_in_the_strict_retry() {
    let fake = Fake::texts(&["Gained gold"]);
    let (tr, late) = make_with_late(&fake, ZH);
    reconfigure_on_call(&fake, &tr, 1, "한국어");
    let examples = ExampleSet {
        system: pair("s", "o"),
        ..ExampleSet::default()
    };
    tr.set_examples(Some(examples), &tr.examples_fingerprint());
    tr.translate_system_message("Gained gold").await.unwrap();
    assert!(late.calls().is_empty());
    let calls = fake.calls();
    assert_eq!(calls.len(), 2);
    for (i, call) in calls.iter().enumerate() {
        assert_eq!(call.system, build_system_message_system(ZH, i == 1));
        assert_eq!(call.turns[..2], [Turn::user("s"), Turn::assistant("o")]);
    }
}

#[tokio::test]
async fn the_success_log_names_the_model_that_served_the_request() {
    let fake = Fake::texts(&["[A] 譯文"]);
    let (tr, late) = make_with_late(&fake, ZH);
    reconfigure_on_call(&fake, &tr, 1, "日本語");
    tr.translate_incoming("[A] served-by", &[]).await.unwrap();
    assert!(late.calls().is_empty());
    let lines = log_lines_with("served-by");
    assert!(lines.iter().any(|l| l.contains("model=m,")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("model=m2")), "{lines:?}");
}

// --- 區域翻譯 ---

#[tokio::test]
async fn region_lines_roundtrip_without_numbers() {
    let fake = Fake::texts(&["跟莫爾談談\n第二行"]);
    let out = make(&fake)
        .translate_region_text("Talk to Merle Ambrose\n\n  Second line ", None)
        .await;
    assert_eq!(out.unwrap(), "跟莫爾談談\n第二行");
    let calls = fake.calls();
    assert_eq!(
        calls[0].turns,
        [Turn::user("Talk to Merle Ambrose\nSecond line")]
    );
    assert_eq!(calls[0].system, build_region_system(ZH));
    assert_eq!(calls[0].max_tokens, Some(MAX_TOKENS_REGION));
}

#[tokio::test]
async fn region_mismatched_count_returns_block() {
    let fake = Fake::texts(&["在鬼屋洞穴（Haunted Cave）擊敗骷髏，找回鑰匙。"]);
    let text = "Defeat the Rattlebones in\nthe Haunted Cave to\nrecover the stolen key.";
    let out = make(&fake).translate_region_text(text, None).await;
    assert_eq!(
        out.unwrap(),
        "在鬼屋洞穴（Haunted Cave）擊敗骷髏，找回鑰匙。"
    );
    assert!(
        log_text()
            .contains("[translate] region text came back as 1 lines for 3; showing it as a whole")
    );
}

#[tokio::test]
async fn region_drops_code_fences_around_the_translation() {
    let fake = Fake::texts(&["```\n跟莫爾談談\n第二行\n```"]);
    let out = make(&fake)
        .translate_region_text("Talk to Merle Ambrose\nSecond line", None)
        .await;
    assert_eq!(out.unwrap(), "跟莫爾談談\n第二行");
}

#[tokio::test]
async fn a_blank_model_reply_is_bad_output_for_incoming_and_system_messages() {
    for reply in [
        "", "  
",
    ] {
        let fake = Fake::texts(&[reply]);
        let tr = make(&fake);
        let expected = Err(TranslateError::BadOutput("model returned no text".into()));
        assert_eq!(tr.translate_incoming("[A] hi", &[]).await, expected);
        assert_eq!(tr.translate_system_message("hi").await, expected);
    }
}

#[tokio::test]
async fn region_empty_is_bad_output() {
    let fake = Fake::texts(&["```\n```"]);
    let out = make(&fake)
        .translate_region_text("Talk to Merle Ambrose", None)
        .await;
    assert_eq!(
        out,
        Err(TranslateError::BadOutput("model returned no text".into()))
    );
    assert!(log_text().contains("[translate] region text came back with no text for 1 lines"));
}

#[tokio::test]
async fn region_parentheses_are_judged_against_nearby_lines() {
    let cases = [
        (
            "若有時間，請拜訪天國大本營（Heavenly HQ）！",
            "若有时间，\n请拜访天国大本营！",
            "若有時間，請拜訪天國大本營！",
        ),
        (
            "若有時間，我希望你再次拜訪天國大本營（Heavenly Headquarters）！",
            "若有时间，我希望你再次拜访天国大本营！",
            "若有時間，我希望你再次拜訪天國大本營！",
        ),
        (
            "跟莫爾·安布羅斯（Merle Ambrose）談談\n天國大本營（Heavenly HQ）",
            "Talk to Merle Ambrose\n天国大本营",
            "跟莫爾·安布羅斯（Merle Ambrose）談談\n天國大本營",
        ),
        (
            "日安！我會在巫師城（Wizard City）等你\n！",
            "Good day! I'll be waiting for you\nin Wizard City!",
            "日安！我會在巫師城（Wizard City）等你\n！",
        ),
        (
            "跟莫爾談談\n第二行\n天國大本營（Merle Ambrose）",
            "Talk to Merle Ambrose\nSecond line\n天国大本营",
            "跟莫爾談談\n第二行\n天國大本營",
        ),
    ];
    for (reply, text, expected) in cases {
        let fake = Fake::texts(&[reply]);
        let out = make(&fake).translate_region_text(text, None).await;
        assert_eq!(out.unwrap(), expected, "{text}");
    }
}

#[tokio::test]
async fn region_logs_no_translation_content() {
    let fake = Fake::texts(&["譯文"]);
    let text = "Talk to Merle Region-Log";
    assert_eq!(
        make(&fake).translate_region_text(text, None).await.unwrap(),
        "譯文"
    );
    let log = log_text();
    assert!(!log.contains("Region-Log"));
    let source = format!("source='<text {} chars, 1 lines>'", text.chars().count());
    assert!(
        log.lines()
            .any(|l| l.contains(&format!("{source} translated=<2 chars>"))),
        "{log}"
    );
}

#[tokio::test]
async fn region_sends_the_region_example_first() {
    let fake = Fake::texts(&["1. 譯文"]);
    let tr = make(&fake);
    let examples = ExampleSet {
        region: pair("1. a", "1. b"),
        ..ExampleSet::default()
    };
    tr.set_examples(Some(examples), &tr.examples_fingerprint());
    tr.translate_region_text("hello", None).await.unwrap();
    assert_eq!(
        fake.calls()[0].turns[..2],
        [Turn::user("1. a"), Turn::assistant("1. b")]
    );
}

// --- 設定與範例 ---

#[test]
fn describe_names_the_service_and_never_the_key() {
    let fake = Fake::new("gpt-x", vec![Ok("x".into())]);
    let s = ServiceSpec {
        provider: "openai".into(),
        model: "gpt-x".into(),
        api_key: "sk-secret".into(),
        ..ServiceSpec::default()
    };
    let tr = Translator::with_client(move |_| fake.clone(), s, ZH.into());
    assert_eq!(tr.describe(), "provider=openai, model=gpt-x");
}

#[tokio::test]
async fn reconfigure_untouched_slot_returns_false() {
    let fake = Fake::texts(&["譯文"]);
    let (tr, late) = make_with_late(&fake, ZH);
    tr.set_examples(Some(ex_incoming("s", "o")), &tr.examples_fingerprint());
    let renamed = ServiceSpec {
        id: "svc-1".into(),
        name: "renamed".into(),
        ..spec("m")
    };
    assert!(!tr.reconfigure(renamed, ZH.into()));
    assert!(tr.examples().is_some());
    tr.translate_incoming("[A] hi", &[]).await.unwrap();
    assert_eq!(fake.calls().len(), 1);
    assert!(late.calls().is_empty());
}

#[test]
fn reconfigure_rebuilds_when_a_value_changed_and_clears_examples() {
    let fake = Fake::texts(&["譯文"]);
    let (tr, _late) = make_with_late(&fake, ZH);
    tr.set_examples(Some(ex_incoming("s", "o")), &tr.examples_fingerprint());
    assert!(tr.reconfigure(spec("m2"), "日本語".into()));
    assert_eq!(tr.describe(), "provider=custom, model=m2");
    assert_eq!(tr.target_language(), "日本語");
    assert!(tr.examples().is_none());
    assert_eq!(
        tr.examples_fingerprint(),
        examples_fingerprint(&spec("m2"), "日本語")
    );
}

#[test]
fn new_builds_the_backend_for_the_provider() {
    let s = ServiceSpec {
        provider: "claude".into(),
        model: "claude-opus-5".into(),
        api_key: "k".into(),
        ..ServiceSpec::default()
    };
    let tr = Translator::new(s, "日本語".into());
    assert_eq!(tr.describe(), "provider=claude, model=claude-opus-5");
    assert_eq!(tr.target_language(), "日本語");
}

#[tokio::test]
async fn a_reconfigure_mid_request_keeps_the_same_backend_for_incoming_and_region() {
    let fake = Fake::texts(&["1. 譯文"]);
    let (tr, late) = make_with_late(&fake, ZH);
    reconfigure_on_call(&fake, &tr, 1, "한국어");
    tr.translate_incoming("[A] hi", &[]).await.unwrap();
    let (tr, late_region) = make_with_late(&fake, ZH);
    reconfigure_on_call(&fake, &tr, 2, "한국어");
    tr.translate_region_text("hello", None).await.unwrap();
    assert!(late.calls().is_empty() && late_region.calls().is_empty());
    let calls = fake.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].system, build_incoming_system(ZH));
    assert_eq!(calls[1].system, build_region_system(ZH));
}

#[tokio::test]
async fn reconfigure_keeps_learned_parameters_when_nothing_changed() {
    let server = MockServer::start().await;
    let chat = || Mock::given(method("POST")).and(path("/v1/chat/completions"));
    chat()
        .respond_with(
            ResponseTemplate::new(400).set_body_json(json!({"error": {"message":
            "Unsupported parameter: 'temperature' is not supported with this model."}})),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    chat()
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "data: {\"choices\": [{\"delta\": {\"content\": \"譯文\"}}]}\n\ndata: [DONE]\n\n",
            "text/event-stream",
        ))
        .mount(&server)
        .await;
    let custom = ServiceSpec {
        provider: "custom".into(),
        base_url: server.uri(),
        model: "m".into(),
        ..ServiceSpec::default()
    };
    let tr = Translator::new(custom.clone(), ZH.into());
    tr.translate_incoming("[A] one", &[]).await.unwrap();
    assert!(!tr.reconfigure(custom, ZH.into()));
    tr.translate_incoming("[A] two", &[]).await.unwrap();
    let sent: Vec<serde_json::Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(sent.len(), 3, "rejected once, then learned for good");
    assert!(sent[0].get("temperature").is_some());
    assert!(sent[1].get("temperature").is_none() && sent[2].get("temperature").is_none());
}

#[test]
fn reconfigure_switches_provider() {
    let tr = Translator::new(spec("m"), ZH.into());
    assert_eq!(tr.describe(), "provider=custom, model=m");
    let claude = ServiceSpec {
        provider: "claude".into(),
        model: "claude-opus-5".into(),
        api_key: "k".into(),
        ..ServiceSpec::default()
    };
    assert!(tr.reconfigure(claude, "日本語".into()));
    assert_eq!(tr.describe(), "provider=claude, model=claude-opus-5");
}

#[tokio::test]
async fn set_examples_ignores_a_stale_fingerprint() {
    let fake = Fake::texts(&["譯文"]);
    let tr = make(&fake);
    assert!(!tr.set_examples(Some(ex_incoming("s", "o")), "other"));
    assert!(tr.examples().is_none());
    assert!(log_text().contains("[translate] ignoring examples for a stale fingerprint: other"));
    tr.translate_system_message("你获得了 {0} 金币！")
        .await
        .unwrap();
    assert_eq!(fake.calls()[0].turns, [Turn::user("你获得了 {0} 金币！")]);
}

#[test]
fn test_sample_is_fixed() {
    assert_eq!(TEST_SAMPLE, "[Tester] Hello! How are you?");
}

// --- 生成範例 ---

#[tokio::test]
async fn generate_examples_retries_bad_output_and_merges() {
    let no_system = JA.replace("{0} ゴールド", "ゴールド");
    let no_incoming = JA.replace("[Amy] ", "");
    let fake = Fake::new(
        "m",
        vec![
            Ok("garbage".into()),
            Err(TranslateError::BadOutput("output truncated".into())),
            Ok(no_system),
            Ok(no_incoming),
        ],
    );
    let tr = make_with(&fake, "日本語");
    let (fp, examples) = tr.generate_examples().await.unwrap();
    assert!(examples.complete());
    assert_eq!(fp, tr.examples_fingerprint());
    let calls = fake.calls();
    assert_eq!(calls.len(), 4);
    assert!(
        calls
            .iter()
            .all(|c| !c.deterministic && c.max_tokens == Some(MAX_TOKENS_REGION))
    );
}

#[tokio::test]
async fn generate_examples_gives_up_after_six_requests() {
    let fake = Fake::texts(&["garbage"]);
    let (_, examples) = make_with(&fake, "日本語")
        .generate_examples()
        .await
        .unwrap();
    assert!(examples.empty());
    assert_eq!(fake.calls().len(), 6);
}

#[tokio::test]
async fn generate_examples_does_not_retry_other_errors() {
    for error in [
        TranslateError::Config {
            status: Some(401),
            detail: "bad key".into(),
        },
        TranslateError::Unexpected("malformed".into()),
    ] {
        let fake = Fake::new("m", vec![Err(error.clone())]);
        let out = make_with(&fake, "日本語").generate_examples().await;
        assert_eq!(out, Err(error));
        assert_eq!(fake.calls().len(), 1);
    }
}

#[tokio::test]
async fn english_target_uses_game_language_examples() {
    let fake = Fake::texts(&["x"]);
    let tr = make_with(&fake, "English");
    let (fp, examples) = tr.generate_examples().await.unwrap();
    assert_eq!(examples, *GAME_LANGUAGE_EXAMPLES);
    assert_eq!(fp, tr.examples_fingerprint());
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn generate_examples_returns_the_fingerprint_of_the_settings_it_used() {
    let fake = Fake::texts(&["garbage", JA]);
    let (tr, late) = make_with_late(&fake, "日本語");
    reconfigure_on_call(&fake, &tr, 1, "한국어");
    let (fp, examples) = tr.generate_examples().await.unwrap();
    assert_eq!(fp, examples_fingerprint(&spec("m"), "日本語"));
    assert_ne!(fp, tr.examples_fingerprint());
    assert!(examples.complete() && late.calls().is_empty());
    assert_eq!(fake.calls().len(), 2);
}

// --- ExampleCoordinator ---

/// 單執行緒 runtime：背景生成只在測試 `block_on` 時才跑，等同 Python 測試的 `spawn=jobs.append`。
struct Harness {
    rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
    store: Arc<ExampleStore>,
    applied: Arc<AtomicUsize>,
    co: Arc<ExampleCoordinator>,
}

fn harness() -> Harness {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ExampleStore::new(dir.path().join("ex.json")));
    let applied = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&applied);
    let co = ExampleCoordinator::new(
        Arc::clone(&store),
        rt.handle().clone(),
        Arc::new(move |_: &Translator| {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
    );
    Harness {
        rt,
        _dir: dir,
        store,
        applied,
        co,
    }
}

impl Harness {
    fn run(&self, job: Option<tokio::task::JoinHandle<()>>) {
        self.rt
            .block_on(job.expect("a background generation"))
            .unwrap();
    }

    fn applied(&self) -> usize {
        self.applied.load(Ordering::SeqCst)
    }
}

fn full_set() -> ExampleSet {
    ExampleSet {
        incoming: pair("s", "o"),
        system: pair("s", "o"),
        region: pair("s", "o"),
    }
}

#[test]
fn cache_hit_applies_without_generating() {
    let h = harness();
    let fake = Fake::texts(&[JA]);
    let tr = make_with(&fake, "日本語");
    h.store.put(&tr.examples_fingerprint(), &full_set());
    assert!(h.co.ensure(&tr).is_none());
    assert_eq!(tr.examples(), Some(full_set()));
    assert!(fake.calls().is_empty());
    assert_eq!(h.applied(), 1);
}

#[test]
fn a_miss_generates_stores_and_applies() {
    let h = harness();
    let fake = Fake::texts(&[JA]);
    let tr = make_with(&fake, "日本語");
    h.run(h.co.ensure(&tr));
    let examples = tr.examples().unwrap();
    assert!(examples.complete());
    assert_eq!(h.store.get(&tr.examples_fingerprint()), Some(examples));
    assert_eq!(fake.calls().len(), 1);
    assert_eq!(h.applied(), 1);
}

#[test]
fn two_translators_on_the_same_service_share_one_generation() {
    let h = harness();
    let (fa, fb) = (Fake::texts(&[JA]), Fake::texts(&[JA]));
    let (a, b) = (make_with(&fa, "日本語"), make_with(&fb, "日本語"));
    let job = h.co.ensure(&a);
    assert!(h.co.ensure(&b).is_none());
    h.run(job);
    assert!(a.examples().is_some() && a.examples() == b.examples());
    assert_eq!((fa.calls().len(), fb.calls().len()), (1, 0));
    assert_eq!(h.applied(), 2);
}

#[test]
fn a_failed_fingerprint_is_not_retried_in_the_same_run() {
    let h = harness();
    let fake = Fake::new("m", vec![Err(TranslateError::offline("down"))]);
    let tr = make_with(&fake, "日本語");
    h.run(h.co.ensure(&tr));
    assert!(h.co.ensure(&tr).is_none());
    assert_eq!(fake.calls().len(), 1);
    assert!(tr.examples().is_none());
    assert!(log_text().contains("): TranslatorOffline: down"));
}

#[test]
fn an_empty_result_counts_as_a_failure() {
    let h = harness();
    let fake = Fake::texts(&["garbage"]);
    let tr = make_with(&fake, "日本語");
    h.run(h.co.ensure(&tr));
    assert!(h.co.ensure(&tr).is_none());
    assert_eq!(fake.calls().len(), 6);
    assert!(h.store.get(&tr.examples_fingerprint()).is_none());
}

#[test]
fn a_result_for_an_outdated_fingerprint_is_stored_but_not_applied() {
    let h = harness();
    let fake = Fake::texts(&[JA]);
    let tr = make_with(&fake, "日本語");
    let requested = tr.examples_fingerprint();
    let weak = Arc::downgrade(&tr);
    // 生成途中使用者切換了目標語言
    fake.on_call(move |_| {
        weak.upgrade().unwrap().reconfigure(spec("m"), ZH.into());
    });
    h.run(h.co.ensure(&tr));
    assert!(tr.examples().is_none());
    assert_eq!(h.applied(), 0);
    assert!(h.store.get(&requested).is_some_and(|e| e.complete()));
}

#[test]
fn a_result_is_applied_by_its_own_fingerprint_not_the_requested_one() {
    let h = harness();
    // b 自己的生成必定失敗：b 拿到範例只可能來自 a 的結果
    let (fa, fb) = (Fake::texts(&[JA]), Fake::texts(&["garbage"]));
    let (a, b) = (make_with(&fa, "日本語"), make_with(&fb, ZH));
    let requested = a.examples_fingerprint();
    let job = h.co.ensure(&a);
    assert!(h.co.ensure(&b).is_some());
    // 背景工作開始前設定就變了：生成走新設定，結果依自己的指紋套用
    a.reconfigure(spec("m"), ZH.into());
    h.run(job);
    assert!(b.examples().is_some());
    assert!(h.store.get(&b.examples_fingerprint()).is_some());
    assert!(h.store.get(&requested).is_none());
}

#[test]
fn registering_the_same_translator_twice_registers_and_generates_once() {
    let h = harness();
    let fake = Fake::texts(&[JA]);
    let tr = make_with(&fake, "日本語");
    h.run(h.co.ensure(&tr));
    assert!(h.co.ensure(&tr).is_none());
    // 重複登記的話，生成完成時會對同一個 translator 套用兩次
    assert_eq!(h.applied(), 2);
    assert_eq!(fake.calls().len(), 1);
}

#[test]
fn a_failure_after_the_settings_changed_does_not_block_a_later_retry() {
    let h = harness();
    let fake = Fake::new(
        "m",
        vec![Err(TranslateError::offline("client closed")), Ok(JA.into())],
    );
    let tr = make_with(&fake, "日本語");
    let weak = Arc::downgrade(&tr);
    fake.on_call(move |n| {
        if n == 1 {
            weak.upgrade().unwrap().reconfigure(spec("m"), ZH.into());
        }
    });
    h.run(h.co.ensure(&tr));
    assert!(log_text().contains("[translate] example generation abandoned, settings changed"));
    tr.reconfigure(spec("m"), "日本語".into());
    h.run(h.co.ensure(&tr));
    assert!(tr.examples().is_some_and(|e| e.complete()));
}

#[test]
fn a_generation_panic_marks_the_fingerprint_failed_instead_of_pending_forever() {
    quiet_panics();
    let h = harness();
    let fake = Fake::texts(&[JA]);
    fake.on_call(|_| panic!("{EXPECTED_PANIC} generation"));
    let tr = make_with(&fake, "panic-target");
    let job = h.co.ensure(&tr).expect("a background generation");
    assert!(h.rt.block_on(job).unwrap_err().is_panic());
    assert!(h.co.ensure(&tr).is_none());
    let fingerprint = tr.examples_fingerprint();
    let verdicts = log_lines_with(&format!("fingerprint={fingerprint}, "));
    assert!(
        verdicts.last().unwrap().ends_with(", failed)"),
        "{verdicts:?}"
    );
    assert!(
        !log_lines_with(&format!(
            "[translate] example generation panicked (fingerprint={fingerprint})"
        ))
        .is_empty()
    );
}

// --- 測試連線與範例快取 ---

fn store_in(dir: &tempfile::TempDir) -> Arc<ExampleStore> {
    Arc::new(ExampleStore::new(dir.path().join("examples.json")))
}

/// 第一次回譯文、之後回完整可驗證的範例改寫。
fn fake_with_examples() -> Arc<Fake> {
    Fake::texts(&["[Tester] 你好！", JA])
}

#[tokio::test]
async fn test_translate_times_only_the_sample_translation() {
    // 門檻等於範例生成的耗時：計到它就一定超過，不計時再慢的機器也遠低於此
    const GENERATION: std::time::Duration = std::time::Duration::from_secs(1);
    let fake = fake_with_examples();
    fake.on_call(|n| {
        if n == 2 {
            std::thread::sleep(GENERATION);
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let out = test_translate(make_with(&fake, "日本語"), Some(store_in(&dir)))
        .await
        .unwrap();
    assert_eq!(out.translated, "[Tester] 你好！");
    assert!(out.elapsed < GENERATION, "{:?}", out.elapsed);
    assert_eq!(fake.calls().len(), 2);
}

#[tokio::test]
async fn test_translate_generates_and_stores_examples() {
    let fake = fake_with_examples();
    let tr = make_with(&fake, "日本語");
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir);
    test_translate(Arc::clone(&tr), Some(Arc::clone(&store)))
        .await
        .unwrap();
    assert!(store.get(&tr.examples_fingerprint()).unwrap().complete());
    assert_eq!(fake.calls()[0].turns.last().unwrap().content, TEST_SAMPLE);
}

#[tokio::test]
async fn test_translate_without_a_store_skips_examples() {
    let fake = fake_with_examples();
    test_translate(make_with(&fake, "日本語"), None)
        .await
        .unwrap();
    assert_eq!(fake.calls().len(), 1);
}

#[tokio::test]
async fn example_failure_after_a_good_test_still_returns_the_translation() {
    let fake = Fake::new(
        "m",
        vec![
            Ok("[Tester] 你好！".into()),
            Err(TranslateError::Config {
                status: Some(429),
                detail: "quota".into(),
            }),
        ],
    );
    let dir = tempfile::tempdir().unwrap();
    let out = test_translate(make_with(&fake, "日本語"), Some(store_in(&dir)))
        .await
        .unwrap();
    assert_eq!(out.translated, "[Tester] 你好！");
    assert!(log_text().contains(
        "[translate] example generation after the connection test failed: TranslatorConfigError"
    ));
}

#[tokio::test]
async fn test_translate_returns_the_translation_error() {
    let err = TranslateError::Unexpected("boom".into());
    let fake = Fake::new("m", vec![Err(err.clone())]);
    let dir = tempfile::tempdir().unwrap();
    let out = test_translate(make_with(&fake, "日本語"), Some(store_in(&dir))).await;
    assert_eq!(out, Err(err));
    assert_eq!(fake.calls().len(), 1);
}

#[tokio::test]
async fn generate_and_store_uses_the_cache_and_skips_empty_results() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir);

    let empty = Fake::texts(&["garbage"]);
    let tr = make_with(&empty, "日本語");
    assert_eq!(generate_and_store(&tr, &store).await, Ok(None));
    assert!(store.get(&tr.examples_fingerprint()).is_none());

    let fake = Fake::texts(&[JA]);
    let tr = make_with(&fake, "日本語");
    let (fingerprint, generated) = generate_and_store(&tr, &store).await.unwrap().unwrap();
    assert_eq!(fingerprint, tr.examples_fingerprint());
    assert_eq!(store.get(&fingerprint), Some(generated.clone()));
    let calls = fake.calls().len();

    assert_eq!(
        generate_and_store(&tr, &store).await,
        Ok(Some((fingerprint.clone(), generated)))
    );
    assert_eq!(fake.calls().len(), calls);
    assert!(log_text().contains(&format!(
        "[translate] examples loaded from cache (fingerprint={fingerprint}):"
    )));
}

/// 測試連線卡在生成請求（第 2 次呼叫）時，coordinator 對同一 store／指紋 `ensure` `tb`；確認它在等，跑完 `while_waiting` 再放行。
fn race_with_connection_test(
    h: &Harness,
    fa: &Arc<Fake>,
    tb: &Arc<Translator>,
    while_waiting: impl FnOnce(),
) {
    let (entered_tx, entered) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    fa.on_call(move |n| {
        if n == 2 {
            entered_tx.send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
        }
    });
    let (ta, store) = (make_with(fa, "日本語"), Arc::clone(&h.store));
    let test = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(test_translate(ta, Some(store))).unwrap();
    });
    entered.recv().unwrap();

    let mut job = h.co.ensure(tb).expect("a cache miss starts a generation");
    h.rt.block_on(async {
        let waited = tokio::time::timeout(std::time::Duration::from_millis(200), &mut job).await;
        assert!(
            waited.is_err(),
            "the coordinator must wait for the running generation"
        );
        while_waiting();
        release.send(()).unwrap();
        job.await.unwrap();
    });
    test.join().unwrap();
}

/// 測試連線：翻譯成功、生成請求失敗。
fn fake_failing_generation() -> Arc<Fake> {
    Fake::new(
        "m",
        vec![
            Ok("[Tester] 你好！".into()),
            Err(TranslateError::offline("down")),
        ],
    )
}

#[test]
fn a_connection_test_and_the_coordinator_share_one_generation() {
    let h = harness();
    let (fa, fb) = (fake_with_examples(), Fake::texts(&[JA]));
    let tb = make_with(&fb, "日本語");
    race_with_connection_test(&h, &fa, &tb, || {});
    assert_eq!((fa.calls().len(), fb.calls().len()), (2, 0));
    assert!(tb.examples().is_some_and(|e| e.complete()));
    assert_eq!(h.applied(), 1);
}

#[test]
fn the_waiting_coordinator_takes_over_when_the_connection_test_generation_fails() {
    let h = harness();
    let (fa, fb) = (fake_failing_generation(), Fake::texts(&[JA]));
    let tb = make_with(&fb, "日本語");
    race_with_connection_test(&h, &fa, &tb, || {});
    assert_eq!((fa.calls().len(), fb.calls().len()), (2, 1));
    assert!(tb.examples().is_some_and(|e| e.complete()));
    assert_eq!(h.applied(), 1);
}

#[test]
fn settings_changed_while_waiting_abandon_the_generation() {
    let h = harness();
    let (fa, fb) = (fake_failing_generation(), Fake::texts(&[JA]));
    let tb = make_with(&fb, "日本語");
    let requested = tb.examples_fingerprint();
    race_with_connection_test(&h, &fa, &tb, || {
        tb.reconfigure(spec("m"), ZH.into());
    });
    assert!(fb.calls().is_empty());
    assert!(tb.examples().is_none());
    assert!(log_text().contains(&format!(
        "[translate] example generation skipped, settings changed while waiting (provider=custom, model=m, fingerprint={requested})"
    )));
    assert!(log_text().contains(&format!(
        "[translate] example generation abandoned, settings changed (fingerprint={requested})"
    )));
}

#[tokio::test]
async fn a_failed_generation_does_not_block_a_retry() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir);
    let down = Fake::new("m", vec![Err(TranslateError::offline("down"))]);
    assert!(
        generate_and_store(&make_with(&down, "日本語"), &store)
            .await
            .is_err()
    );
    let fake = Fake::texts(&[JA]);
    let tr = make_with(&fake, "日本語");
    assert!(generate_and_store(&tr, &store).await.unwrap().is_some());
    assert_eq!(fake.calls().len(), 1);
}
