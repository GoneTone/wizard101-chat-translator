//! 單筆服務編輯表單（移植自 Python 的 test_service_form 與 test_fields 的模型欄位部分）。

use crate::common;

use std::time::Duration;

use common::kittest::{has_label, painted_texts, run, settle, themed_harness, tr};
use common::log_capture::{capture_logs, log_text};
use common::settings_env::{FakeEnv, installed};
use eframe::egui::{self, Event, Ui};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use wizard101_chat_translator::config::services::{EFFORT_LOW, new_service, provider};
use wizard101_chat_translator::i18n::{self, t};
use wizard101_chat_translator::local::Snapshot;
use wizard101_chat_translator::local::store::Installed;
use wizard101_chat_translator::translation::clients::{ModelInfo, ThinkingCaps};
use wizard101_chat_translator::translation::errors::{
    TranslateError, friendly_error, friendly_text,
};
use wizard101_chat_translator::translation::translator::{TEST_SAMPLE, TestOutcome};
use wizard101_chat_translator::ui::settings::env::SettingsEvent;
use wizard101_chat_translator::ui::settings::service_form::{
    FormResponse, ServiceForm, format_seconds,
};

/// 本檔所有測試都用同一個介面語言，不會互相干擾。
const LANGUAGE: &str = "en-US";
const TARGET: &str = "Japanese";

struct State {
    form: ServiceForm,
    env: FakeEnv,
    /// 每一輪 `ui` 的回報；egui 重排一幀時同一幀會跑兩次。
    responses: Vec<FormResponse>,
}

fn form(service: Value, services: Vec<Value>) -> ServiceForm {
    i18n::set_language(LANGUAGE);
    ServiceForm::new(service, services)
}

fn blank() -> ServiceForm {
    form(new_service("openai", &[]), vec![])
}

fn filled(provider: &str) -> Value {
    let mut service = new_service(provider, &[]);
    service["model"] = json!("gpt-x");
    service["api_key"] = json!("sk-1");
    service
}

fn harness(form: ServiceForm) -> Harness<'static, State> {
    let mut h = opening_harness(form);
    settle(&mut h);
    h
}

/// 剛開啟、開啟時的載入佔位還在的表單：harness 建好後才換上 `form`，佔位的時間從這一幀起算。
fn opening_harness(form: ServiceForm) -> Harness<'static, State> {
    let state = State {
        form: blank(),
        env: FakeEnv::default(),
        responses: Vec::new(),
    };
    let mut h = themed_harness(
        egui::vec2(540.0, 760.0),
        LANGUAGE,
        |ui: &mut Ui, s: &mut State| {
            let response = s.form.ui(ui, &s.env, &s.env.snapshot.borrow(), TARGET);
            s.responses.push(response);
        },
        state,
    );
    h.state_mut().form = form;
    h.step();
    h
}

fn click(h: &mut Harness<State>, label: &str) {
    h.get_by_label(label).click();
    run(h);
}

fn type_into(h: &mut Harness<State>, label: &str, text: &str) {
    click(h, label);
    h.event(Event::Text(text.into()));
    run(h);
}

fn start_test(h: &mut Harness<State>) -> u64 {
    click(h, &tr("button.test"));
    h.state().env.tests.borrow().last().expect("test started").0
}

fn finish_test(h: &mut Harness<State>, round: u64, result: Result<TestOutcome, TranslateError>) {
    h.state_mut()
        .form
        .apply(&SettingsEvent::TestDone { round, result });
    run(h);
}

fn translated(text: &str, ms: u64) -> Result<TestOutcome, TranslateError> {
    Ok(TestOutcome {
        translated: text.into(),
        elapsed: Duration::from_millis(ms),
    })
}

/// 餵一次模型清單結果：開啟時的自動載入還在跑就餵給它，否則按〔更新〕。
fn refresh_models(h: &mut Harness<State>, result: Result<Vec<ModelInfo>, TranslateError>) {
    if h.query_by_label(&tr("button.loading")).is_none() {
        click(h, &tr("button.refresh"));
    }
    let round = h.state().env.lists.borrow().last().expect("list started").0;
    h.state_mut()
        .form
        .apply(&SettingsEvent::ModelsDone { round, result });
    run(h);
}

/// 把模型欄整個換成 `text`。
fn replace_model(h: &mut Harness<State>, text: &str) {
    click(h, &tr("field.model"));
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.event(Event::Text(text.into()));
    run(h);
}

fn with_caps(id: &str, thinking: ThinkingCaps) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        thinking,
    }
}

fn locked(supported: Option<bool>, can_disable: Option<bool>) -> ThinkingCaps {
    ThinkingCaps {
        supported,
        can_disable,
        efforts: None,
    }
}

fn switch_on(h: &Harness<State>) -> bool {
    h.get_by_label(&tr("field.thinking"))
        .accesskit_node()
        .toggled()
        .is_some_and(|t| t == egui::accesskit::Toggled::True)
}

fn names(items: &[&str]) -> Vec<ModelInfo> {
    items.iter().copied().map(ModelInfo::bare).collect()
}

/// 清掉之前的回報，跑一幀，回傳這一幀有沒有回報 `pick`。
fn step_reports(h: &mut Harness<State>, pick: fn(&FormResponse) -> bool) -> bool {
    h.step();
    let reported = h.state().responses.iter().any(pick);
    h.state_mut().responses.clear();
    reported
}

fn changed(r: &FormResponse) -> bool {
    r.changed
}

fn button_enabled(h: &Harness<State>, label: &str) -> bool {
    !h.get_by_label(label).accesskit_node().is_disabled()
}

// --- 值與名稱 ---

#[test]
fn new_form_starts_from_the_providers_defaults() {
    let values = blank().values();
    assert_eq!(values["provider"], "openai");
    assert_eq!(values["name"], "OpenAI");
    assert_eq!(values["model"], "");
}

#[test]
fn editing_keeps_the_id() {
    let mut service = new_service("claude", &[]);
    service["name"] = json!("我的 Claude");
    service["model"] = json!("claude-x");
    service["api_key"] = json!("sk-ant");
    let values = form(service.clone(), vec![]).values();
    assert_eq!(values["id"], service["id"]);
    assert_eq!(values["name"], "我的 Claude");
    assert_eq!(values["model"], "claude-x");
}

#[test]
fn values_only_carry_the_fields_that_provider_has() {
    let values = form(new_service("claude", &[]), vec![]).values();
    let keys: Vec<&str> = values
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "id", "name", "provider", "model", "api_key", "thinking", "effort"
        ]
    );
}

#[test]
fn api_values_drop_the_bookkeeping_fields() {
    let api = blank().api_values();
    assert!(api.get("id").is_none() && api.get("name").is_none());
    assert_eq!(api["provider"], "openai");
}

#[test]
fn values_strip_the_text_fields() {
    let mut service = filled("openai");
    service["model"] = json!("  gpt-x \n");
    assert_eq!(form(service, vec![]).api_values()["model"], "gpt-x");
}

#[test]
fn switching_provider_rewrites_an_untouched_name() {
    let mut f = blank();
    f.set_provider("claude");
    assert_eq!(f.values()["name"], "Claude");
}

#[test]
fn switching_provider_keeps_a_name_the_user_typed() {
    let mut f = blank();
    f.set_name("戰鬥用");
    f.set_provider("claude");
    assert_eq!(f.values()["name"], "戰鬥用");
}

#[test]
fn switching_provider_rewrites_a_suffixed_untouched_name() {
    // 清單裡已有一筆 OpenAI，草稿的自動名稱是「OpenAI (2)」，仍算沒改過名字
    let existing = new_service("openai", &[]);
    let draft = new_service("openai", std::slice::from_ref(&existing));
    let mut f = form(draft, vec![]);
    f.set_provider("claude");
    assert_eq!(f.values()["name"], "Claude");
}

#[test]
fn a_blank_name_blocks_saving() {
    let mut f = form(filled("openai"), vec![]);
    f.set_name("   ");
    assert_eq!(f.errors(), vec!["error.need_name".to_string()]);
}

#[test]
fn switching_provider_dedupes_the_new_name_in_the_field() {
    // 撞名要在換服務商當下就補序號，欄位裡看到的必須是實際會存的名字
    let existing = new_service("claude", &[]);
    let draft = new_service("openai", std::slice::from_ref(&existing));
    let mut h = harness(form(draft, vec![existing]));
    h.state_mut().form.set_provider("claude");
    run(&mut h);
    assert_eq!(
        h.get_by_label(&tr("field.service_name")).value().as_deref(),
        Some("Claude (2)")
    );
    assert_eq!(h.state().form.values()["name"], "Claude (2)");
}

#[test]
fn switching_provider_to_a_free_short_name_has_no_suffix() {
    let other = new_service("custom", &[]);
    let draft = new_service("openai", std::slice::from_ref(&other));
    let mut f = form(draft, vec![other]);
    f.set_provider("claude");
    assert_eq!(f.values()["name"], "Claude");
}

#[test]
fn editing_a_service_switching_away_and_back_keeps_its_bare_name() {
    // 這筆本來就叫 Claude，切走再切回來不能跟自己撞名變成「Claude (2)」
    let service = new_service("claude", &[]);
    let mut f = form(service.clone(), vec![service]);
    f.set_provider("openai");
    f.set_provider("claude");
    assert_eq!(f.values()["name"], "Claude");
}

#[test]
fn switching_provider_keeps_a_typed_name_even_with_a_collision() {
    let existing = new_service("claude", &[]);
    let draft = new_service("openai", std::slice::from_ref(&existing));
    let mut f = form(draft, vec![existing]);
    f.set_name("戰鬥用");
    f.set_provider("claude");
    assert_eq!(f.values()["name"], "戰鬥用");
}

#[test]
fn switching_provider_does_not_carry_values_across() {
    let mut f = form(filled("openai"), vec![]);
    f.set_provider("claude");
    assert_eq!(f.values()["model"], "");
    assert_eq!(f.values()["api_key"], "");
}

#[test]
fn claude_keeps_its_effort_choice() {
    let mut service = new_service("claude", &[]);
    service["effort"] = json!(EFFORT_LOW);
    assert_eq!(form(service, vec![]).values()["effort"], EFFORT_LOW);
}

#[test]
fn switching_provider_takes_the_new_providers_effort_default() {
    let mut f = blank();
    f.set_provider("gemini");
    assert_eq!(f.values()["effort"], EFFORT_LOW);
    f.set_provider("claude");
    assert_eq!(f.values()["effort"], "auto");
}

// --- 版面 ---

#[test]
fn the_provider_row_shows_the_current_provider() {
    let label = tr(provider("openai").unwrap().label_key);
    let h = harness(blank());
    assert!(painted_texts(&h).contains(&label));
}

#[test]
fn the_change_button_asks_the_owner_to_pick_a_provider() {
    let mut h = harness(blank());
    assert!(!step_reports(&mut h, |r| r.change_provider));
    h.get_by_label(&tr("button.change")).click();
    assert!(step_reports(&mut h, |r| r.change_provider));
}

#[test]
fn every_provider_shows_the_switch() {
    for key in ["openai", "claude", "gemini", "custom"] {
        let h = harness(form(new_service(key, &[]), vec![]));
        assert!(has_label(&h, &tr("field.thinking")), "{key}");
    }
}

#[test]
fn effort_hidden_while_switch_off() {
    let mut h = harness(blank());
    assert!(!has_label(&h, &tr("field.effort")));
    click(&mut h, &tr("field.thinking"));
    assert!(has_label(&h, &tr("field.effort")));
    assert!(painted_texts(&h).contains(&tr("hint.effort")));
}

#[test]
fn the_thinking_toggle_flips_the_value() {
    let mut h = harness(blank());
    click(&mut h, &tr("field.thinking"));
    assert_eq!(h.state().form.values()["thinking"], true);
}

#[test]
fn the_effort_dropdown_shows_the_saved_choice() {
    let mut service = new_service("claude", &[]);
    service["effort"] = json!(EFFORT_LOW);
    let h = harness(form(service, vec![]));
    assert!(painted_texts(&h).contains(&tr("effort.low")));
}

#[test]
fn choosing_an_effort_stores_its_code() {
    let mut h = harness(form(new_service("claude", &[]), vec![]));
    click(&mut h, &tr("field.effort"));
    click(&mut h, &tr("effort.low"));
    assert_eq!(h.state().form.values()["effort"], EFFORT_LOW);
}

#[test]
fn the_custom_endpoint_asks_for_the_url_first_and_the_key_is_optional() {
    let h = harness(form(new_service("custom", &[]), vec![]));
    let url = h.get_by_label(&tr("field.base_url")).rect();
    let key = h.get_by_label(&tr("field.api_key_optional")).rect();
    assert!(url.top() < key.top());
    assert!(painted_texts(&h).contains(&tr("hint.custom_endpoint")));
    assert!(!has_label(&h, &tr("link.get_key")));
}

#[test]
fn official_providers_link_to_their_key_page() {
    let h = harness(form(new_service("claude", &[]), vec![]));
    let label = tr("link.get_key");
    let link = h.get_by_label(&label);
    assert_eq!(link.accesskit_node().role(), egui::accesskit::Role::Link);
    assert!(!has_label(&h, &tr("field.base_url")));
}

#[test]
fn the_api_key_is_masked_until_revealed() {
    let mut service = new_service("openai", &[]);
    service["api_key"] = json!("sk-secret-123");
    let mut h = harness(form(service, vec![]));
    assert!(
        !painted_texts(&h)
            .iter()
            .any(|s| s.contains("sk-secret-123"))
    );
    click(&mut h, &tr("button.show"));
    assert!(
        painted_texts(&h)
            .iter()
            .any(|s| s.contains("sk-secret-123"))
    );
    // 顯示中按鈕改成「隱藏」
    assert!(h.query_by_label(&tr("button.show")).is_none());
    click(&mut h, &tr("button.hide"));
    assert!(
        !painted_texts(&h)
            .iter()
            .any(|s| s.contains("sk-secret-123"))
    );
}

// --- 變動通知 ---

#[test]
fn editing_a_field_notifies_the_owner() {
    // 欄位一改就得回報，否則精靈的〔下一步〕永遠停在上一次的判定
    let mut h = harness(blank());
    click(&mut h, &tr("field.model"));
    assert!(!step_reports(&mut h, changed));
    h.event(Event::Text("g".into()));
    assert!(step_reports(&mut h, changed));
    assert!(!step_reports(&mut h, changed), "reported once per edit");
}

#[test]
fn renaming_is_not_an_api_change() {
    let mut h = harness(blank());
    click(&mut h, &tr("field.service_name"));
    h.event(Event::Text("x".into()));
    assert!(!step_reports(&mut h, changed));
    assert_eq!(h.state().form.values()["name"], "OpenAIx");
}

#[test]
fn switching_provider_notifies_once() {
    let mut h = harness(blank());
    h.state_mut().form.set_provider("claude");
    assert!(step_reports(&mut h, changed));
    assert!(!step_reports(&mut h, changed));
}

// --- 測試連線 ---

#[test]
fn format_seconds_rounds_half_up() {
    let cases = [
        (1240, "1.2"),
        (1250, "1.3"),
        (49, "0.0"),
        (50, "0.1"),
        (61049, "61.0"),
    ];
    for (ms, expected) in cases {
        assert_eq!(
            format_seconds(Duration::from_millis(ms)),
            expected,
            "{ms}ms"
        );
    }
}

#[test]
fn test_connection_sends_the_form_values_and_target() {
    let mut h = harness(form(filled("openai"), vec![]));
    start_test(&mut h);
    let tests = h.state().env.tests.borrow();
    let (_, spec, target) = &tests[0];
    assert_eq!(spec.provider, "openai");
    assert_eq!(spec.model, "gpt-x");
    assert_eq!(spec.api_key, "sk-1");
    assert_eq!(target, TARGET);
}

#[test]
fn testing_disables_the_button_until_the_result_arrives() {
    let mut h = harness(form(filled("openai"), vec![]));
    let round = start_test(&mut h);
    assert!(!button_enabled(&h, &tr("button.testing")));
    finish_test(&mut h, round, translated("x", 100));
    assert!(button_enabled(&h, &tr("button.test")));
}

#[test]
fn a_successful_test_shows_time_source_and_translation() {
    let mut h = harness(form(filled("openai"), vec![]));
    let round = start_test(&mut h);
    finish_test(&mut h, round, translated("[Tester] こんにちは", 1250));
    assert!(h.state().form.test_passed());
    h.get_by_label(&t("test.connected", &[("seconds", "1.3")]));
    let texts = painted_texts(&h);
    for text in [
        tr("test.source"),
        TEST_SAMPLE.to_owned(),
        tr("test.translation"),
        "[Tester] こんにちは".to_owned(),
    ] {
        assert!(texts.contains(&text), "{text} in {texts:?}");
    }
}

#[test]
fn a_failed_test_shows_the_friendly_error() {
    let mut h = harness(form(filled("openai"), vec![]));
    let round = start_test(&mut h);
    let error = TranslateError::Config {
        status: Some(401),
        detail: "Incorrect API key".into(),
    };
    finish_test(&mut h, round, Err(error));
    assert!(!h.state().form.test_passed());
    let expected = t(
        "error.api_response",
        &[("status", "401"), ("message", "Incorrect API key")],
    );
    assert!(painted_texts(&h).concat().contains(&expected));
}

#[test]
fn test_errors_make_urls_clickable() {
    let mut h = harness(form(filled("openai"), vec![]));
    let round = start_test(&mut h);
    let error = TranslateError::Offline {
        status: Some(429),
        detail: "quota exceeded, read https://platform.example/docs/errors.".into(),
        retry_after: None,
    };
    finish_test(&mut h, round, Err(error));
    let link = h.get_by_label("https://platform.example/docs/errors");
    assert_eq!(link.accesskit_node().role(), egui::accesskit::Role::Link);
}

#[test]
fn testing_with_missing_fields_shows_them_without_a_request() {
    let mut h = harness(blank());
    click(&mut h, &tr("button.test"));
    assert!(h.state().env.tests.borrow().is_empty());
    let expected = [tr("error.need_model"), tr("error.need_api_key")].join(&tr("sep.errors"));
    assert!(painted_texts(&h).concat().contains(&expected));
    assert!(!h.state().form.test_passed());
}

fn custom(base_url: &str) -> ServiceForm {
    let mut service = new_service("custom", &[]);
    service["model"] = json!("m");
    service["base_url"] = json!(base_url);
    form(service, vec![])
}

#[test]
fn a_base_url_without_a_scheme_or_ending_in_v1_is_refused() {
    let cases = [
        ("localhost:1234", Some("error.base_url_scheme")),
        ("ftp://host", Some("error.base_url_scheme")),
        ("http://localhost:1234/v1", Some("error.base_url_v1")),
        ("HTTPS://host/V1/", Some("error.base_url_v1")),
        ("HTTP://localhost:1234", None),
        ("https://host/v1beta", None),
    ];
    for (url, expected) in cases {
        let errors = custom(url).errors();
        assert_eq!(errors, expected.into_iter().collect::<Vec<_>>(), "{url}");
    }
}

#[test]
fn testing_a_bad_base_url_shows_why_without_a_request() {
    let mut h = harness(custom("http://localhost:1234/v1"));
    click(&mut h, &tr("button.test"));
    assert!(h.state().env.tests.borrow().is_empty());
    assert!(
        painted_texts(&h)
            .concat()
            .contains(&tr("error.base_url_v1"))
    );
}

#[test]
fn an_unknown_effort_is_saved_as_auto() {
    // 畫面顯示「自動」，存回去的也要是自動，否則手改壞的值會原樣送給 API
    let mut service = new_service("claude", &[]);
    service["effort"] = json!("");
    assert_eq!(form(service, vec![]).values()["effort"], "auto");
}

#[test]
fn switching_provider_clears_the_test_result() {
    // 顯示中的結果是對上一家端點測出來的，換了一家就不算數
    let mut h = harness(form(filled("openai"), vec![]));
    let round = start_test(&mut h);
    finish_test(&mut h, round, translated("x", 100));
    assert!(h.state().form.test_passed());
    h.state_mut().form.set_provider("claude");
    run(&mut h);
    assert!(!h.state().form.test_passed());
    assert!(!painted_texts(&h).contains(&TEST_SAMPLE.to_owned()));
}

#[test]
fn editing_after_a_passed_test_voids_it() {
    let mut h = harness(form(filled("openai"), vec![]));
    let round = start_test(&mut h);
    finish_test(&mut h, round, translated("x", 100));
    type_into(&mut h, &tr("field.model"), "2");
    assert!(!h.state().form.test_passed());
    assert!(!painted_texts(&h).contains(&TEST_SAMPLE.to_owned()));
}

#[test]
fn a_failed_test_stays_visible_after_editing() {
    // 失敗原因留著方便對照修改；成功的綠字才會誤導
    let mut h = harness(form(filled("openai"), vec![]));
    let round = start_test(&mut h);
    finish_test(&mut h, round, Err(TranslateError::offline("refused")));
    type_into(&mut h, &tr("field.model"), "2");
    assert!(!h.state().form.test_passed());
    let expected = t("error.offline_detail", &[("message", "refused")]);
    assert!(painted_texts(&h).concat().contains(&expected));
}

#[test]
fn the_form_leaves_the_callers_spacing_alone() {
    i18n::set_language(LANGUAGE);
    let state = (blank(), FakeEnv::default(), None);
    let h = themed_harness(
        egui::vec2(540.0, 760.0),
        LANGUAGE,
        |ui: &mut Ui, s: &mut (ServiceForm, FakeEnv, Option<(f32, f32)>)| {
            let before = ui.spacing().item_spacing.y;
            s.0.ui(ui, &s.1, &Snapshot::default(), TARGET);
            s.2 = Some((before, ui.spacing().item_spacing.y));
        },
        state,
    );
    let (before, after) = h.state().2.expect("form drawn");
    assert_eq!(before, after);
}

#[test]
fn a_result_after_editing_is_discarded_and_the_button_recovers() {
    // 改金鑰
    let mut h = harness(form(filled("openai"), vec![]));
    let round = start_test(&mut h);
    type_into(&mut h, &tr("field.api_key"), "2");
    finish_test(&mut h, round, translated("x", 100));
    assert!(!h.state().form.test_passed());
    assert!(!painted_texts(&h).contains(&TEST_SAMPLE.to_owned()));
    assert!(button_enabled(&h, &tr("button.test")));

    // 換服務商
    let mut h = harness(form(filled("openai"), vec![]));
    let round = start_test(&mut h);
    h.state_mut().form.set_provider("gemini");
    run(&mut h);
    finish_test(&mut h, round, translated("x", 100));
    assert!(!h.state().form.test_passed());
    assert!(button_enabled(&h, &tr("button.test")));

    // 模型清單（開啟時自動載入的那一輪）
    let mut h = harness(form(filled("openai"), vec![]));
    let round = h.state().env.lists.borrow()[0].0;
    assert!(!button_enabled(&h, &tr("button.loading")));
    type_into(&mut h, &tr("field.api_key"), "2");
    let result = Ok(names(&["gpt-a", "gpt-b"]));
    h.state_mut()
        .form
        .apply(&SettingsEvent::ModelsDone { round, result });
    run(&mut h);
    assert!(painted_texts(&h).contains(&tr("hint.model_idle")));
    assert!(button_enabled(&h, &tr("button.refresh")));
}

// --- 模型欄 ---

#[test]
fn model_field_shows_fetched_models() {
    let mut service = filled("openai");
    service["model"] = json!("");
    let mut h = harness(form(service, vec![]));
    refresh_models(&mut h, Ok(names(&["gemma3", "qwen3"])));
    assert!(painted_texts(&h).contains(&t("hint.model_found", &[("count", "2")])));
    click(&mut h, &tr("field.model"));
    h.get_by_label("gemma3");
}

#[test]
fn model_field_says_when_the_server_has_no_models() {
    let mut h = harness(form(filled("openai"), vec![]));
    refresh_models(&mut h, Ok(vec![]));
    assert!(painted_texts(&h).contains(&tr("hint.model_empty")));
}

#[test]
fn model_field_unsupported_endpoint_hints_manual_input() {
    let mut service = new_service("custom", &[]);
    service["base_url"] = json!("http://x");
    let mut h = harness(form(service, vec![]));
    refresh_models(&mut h, Err(TranslateError::NoModelList));
    assert!(painted_texts(&h).contains(&tr("hint.model_no_list")));
}

#[test]
fn model_field_error_uses_friendly_message() {
    let mut h = harness(form(filled("openai"), vec![]));
    refresh_models(&mut h, Err(TranslateError::offline("refused")));
    let expected = t("error.offline_detail", &[("message", "refused")]);
    assert!(painted_texts(&h).concat().contains(&expected));
}

#[test]
fn refresh_result_reports_failures_for_providers_without_base_url() {
    // 回歸：openai／claude 沒有 base_url，失敗結果仍要顯示出來，按鈕也不能卡在載入中
    let mut h = harness(form(filled("openai"), vec![]));
    let error = TranslateError::Config {
        status: Some(401),
        detail: "Incorrect API key".into(),
    };
    refresh_models(&mut h, Err(error));
    let expected = t(
        "error.api_response",
        &[("status", "401"), ("message", "Incorrect API key")],
    );
    assert!(painted_texts(&h).concat().contains(&expected));
    assert!(button_enabled(&h, &tr("button.refresh")));
}

#[test]
fn model_field_error_makes_urls_clickable() {
    let mut h = harness(form(filled("openai"), vec![]));
    let error = TranslateError::Config {
        status: Some(400),
        detail: "see https://a.example/docs for models".into(),
    };
    refresh_models(&mut h, Err(error));
    let link = h.get_by_label("https://a.example/docs");
    assert_eq!(link.accesskit_node().role(), egui::accesskit::Role::Link);
}

#[test]
fn refresh_checks_the_endpoint_fields_first() {
    let mut h = harness(blank());
    click(&mut h, &tr("button.refresh"));
    assert!(h.state().env.lists.borrow().is_empty());
    assert!(painted_texts(&h).contains(&tr("error.need_api_key")));
}

#[test]
fn model_field_typing_keeps_every_fetched_option() {
    let mut service = filled("openai");
    service["model"] = json!("");
    let mut h = harness(form(service, vec![]));
    refresh_models(&mut h, Ok(names(&["gemma3", "qwen3-32b", "qwen3-8b"])));
    type_into(&mut h, &tr("field.model"), "qwen");
    for option in ["gemma3", "qwen3-32b", "qwen3-8b"] {
        h.get_by_label(option);
    }
}

#[test]
fn model_dropdown_closes_on_outside_click_and_lists_all_when_nothing_matches() {
    let mut service = filled("openai");
    service["model"] = json!("");
    let mut h = harness(form(service, vec![]));
    refresh_models(&mut h, Ok(names(&["alpha", "beta"])));
    click(&mut h, &tr("field.model"));
    h.get_by_label("alpha");
    click(&mut h, &tr("field.service_name"));
    assert!(h.query_by_label("alpha").is_none());

    type_into(&mut h, &tr("field.model"), "zzz");
    h.get_by_label("alpha");
    h.get_by_label("beta");
}

#[test]
fn model_field_hint_aligns_with_input() {
    let h = harness(blank());
    let input = h.get_by_label(&tr("field.model")).rect();
    let hint = h
        .get_by(|n| n.value().as_deref() == Some(tr("hint.model_idle").as_str()))
        .rect();
    assert!(
        (hint.left() - input.left()).abs() < 12.0,
        "{hint:?} vs {input:?}"
    );
}

#[test]
fn switching_provider_clears_the_fetched_model_list() {
    // 模型清單是上一家端點給的，換一家就不算數
    let mut h = harness(form(filled("openai"), vec![]));
    refresh_models(&mut h, Ok(names(&["gpt-5.6-sol", "gpt-5.6-luna"])));
    h.state_mut().form.set_provider("claude");
    run(&mut h);
    assert!(painted_texts(&h).contains(&tr("hint.model_idle")));
    click(&mut h, &tr("field.model"));
    assert!(h.query_by_label("gpt-5.6-sol").is_none());
}

// --- friendly_error（移植自 test_fields） ---

#[test]
fn friendly_error_guesses_from_status_when_api_gave_no_message() {
    let config = |status| TranslateError::Config {
        status: Some(status),
        detail: String::new(),
    };
    assert_eq!(friendly_error(&config(401)), ("error.bad_key", vec![]));
    assert_eq!(
        friendly_error(&config(404)),
        ("error.model_not_found", vec![])
    );
    assert_eq!(
        friendly_error(&config(400)),
        ("error.api_http", vec![("status", "400".to_owned())])
    );
    let server = TranslateError::Offline {
        status: Some(503),
        detail: String::new(),
        retry_after: None,
    };
    assert_eq!(
        friendly_error(&server),
        ("error.server_http", vec![("status", "503".to_owned())])
    );
    assert_eq!(
        friendly_error(&TranslateError::offline("")),
        ("error.offline", vec![])
    );
}

#[test]
fn friendly_error_prefers_the_api_message_over_status_guess() {
    // 狀態碼猜的提示會誤導（自架端點 404 常是網址錯而非模型錯），有 API 說明就用它
    let error = TranslateError::Config {
        status: Some(401),
        detail: "Incorrect API key".into(),
    };
    assert_eq!(
        friendly_error(&error),
        (
            "error.api_response",
            vec![
                ("status", "401".to_owned()),
                ("message", "Incorrect API key".to_owned())
            ]
        )
    );
}

#[test]
fn friendly_error_shows_connection_failure_reason() {
    let error = TranslateError::offline("[Errno 11001] getaddrinfo failed");
    assert_eq!(
        friendly_error(&error),
        (
            "error.offline_detail",
            vec![("message", "[Errno 11001] getaddrinfo failed".to_owned())]
        )
    );
}

// --- 依模型能力鎖定思考開關 ---

fn claude_with(models: Vec<ModelInfo>, model: &str) -> Harness<'static, State> {
    let mut service = filled("claude");
    service["model"] = json!(model);
    let mut h = harness(form(service, vec![]));
    refresh_models(&mut h, Ok(models));
    h
}

#[test]
fn unsupported_model_locks_switch_off() {
    let h = claude_with(vec![with_caps("plain", locked(Some(false), None))], "plain");
    assert!(!switch_on(&h));
    assert!(!button_enabled(&h, &tr("field.thinking")));
    assert!(painted_texts(&h).contains(&tr("hint.thinking_unsupported")));
    assert!(!has_label(&h, &tr("field.effort")));
    assert_eq!(h.state().form.values()["thinking"], false);
}

#[test]
fn always_thinking_model_locks_switch_on() {
    let mut service = filled("openai");
    service["model"] = json!("deep");
    let mut h = harness(form(service, vec![]));
    refresh_models(
        &mut h,
        Ok(vec![with_caps("deep", locked(Some(true), Some(false)))]),
    );
    assert!(switch_on(&h));
    assert!(!button_enabled(&h, &tr("field.thinking")));
    assert!(painted_texts(&h).contains(&tr("hint.thinking_always")));
    assert!(has_label(&h, &tr("field.effort")));
    assert_eq!(h.state().form.values()["thinking"], true);
}

#[test]
fn caps_follow_the_typed_model_id() {
    let models = vec![
        with_caps("always", locked(Some(true), Some(false))),
        with_caps("free", ThinkingCaps::default()),
    ];
    let mut h = claude_with(models, "always");
    assert!(!button_enabled(&h, &tr("field.thinking")));
    replace_model(&mut h, "free");
    assert!(button_enabled(&h, &tr("field.thinking")));
    replace_model(&mut h, "always");
    assert!(!button_enabled(&h, &tr("field.thinking")));
    replace_model(&mut h, "unlisted");
    assert!(button_enabled(&h, &tr("field.thinking")));
    assert!(painted_texts(&h).contains(&tr("hint.thinking")));
}

#[test]
fn locked_switch_restores_the_users_choice() {
    let mut service = filled("openai");
    service["model"] = json!("free");
    service["thinking"] = json!(false);
    let models = vec![
        with_caps("always", locked(Some(true), Some(false))),
        with_caps("free", ThinkingCaps::default()),
    ];
    let mut h = harness(form(service, vec![]));
    refresh_models(&mut h, Ok(models));
    replace_model(&mut h, "always");
    assert_eq!(h.state().form.values()["thinking"], true);
    replace_model(&mut h, "free");
    assert_eq!(h.state().form.values()["thinking"], false);
}

#[test]
fn effort_resets_when_new_model_lacks_it() {
    let mut service = filled("claude");
    service["model"] = json!("free");
    service["effort"] = json!("xhigh");
    let narrow = ThinkingCaps {
        efforts: Some(vec!["low".into()]),
        ..ThinkingCaps::default()
    };
    let mut h = harness(form(service, vec![]));
    refresh_models(
        &mut h,
        Ok(vec![
            with_caps("free", ThinkingCaps::default()),
            with_caps("narrow", narrow),
        ]),
    );
    assert_eq!(h.state().form.values()["effort"], "xhigh");
    replace_model(&mut h, "narrow");
    assert_eq!(h.state().form.values()["effort"], "auto");
}

#[test]
fn effort_menu_lists_only_supported_levels() {
    let narrow = ThinkingCaps {
        efforts: Some(vec!["low".into(), "high".into()]),
        ..ThinkingCaps::default()
    };
    let mut h = claude_with(vec![with_caps("narrow", narrow)], "narrow");
    click(&mut h, &tr("field.effort"));
    assert!(has_label(&h, &tr("effort.low")));
    assert!(has_label(&h, &tr("effort.high")));
    assert!(!has_label(&h, &tr("effort.medium")));
}

// --- 開啟表單時自動載入模型清單 ---

fn list_count(h: &Harness<State>) -> usize {
    h.state().env.lists.borrow().len()
}

#[test]
fn opening_a_filled_form_loads_models_once() {
    let mut h = harness(form(filled("openai"), vec![]));
    run(&mut h);
    run(&mut h);
    assert_eq!(list_count(&h), 1);
}

#[test]
fn opening_an_empty_form_does_not_load() {
    let h = harness(blank());
    assert_eq!(list_count(&h), 0);
    assert!(painted_texts(&h).contains(&tr("hint.model_idle")));
}

#[test]
fn key_focus_loss_loads_once() {
    let mut h = harness(blank());
    type_into(&mut h, &tr("field.api_key"), "sk-1");
    assert_eq!(list_count(&h), 0);
    click(&mut h, &tr("field.service_name"));
    assert_eq!(list_count(&h), 1);
    click(&mut h, &tr("field.api_key"));
    click(&mut h, &tr("field.service_name"));
    assert_eq!(list_count(&h), 1);
}

#[test]
fn auto_load_without_model_list_keeps_switch_usable() {
    let mut service = new_service("custom", &[]);
    service["base_url"] = json!("http://127.0.0.1:8080");
    let mut h = harness(form(service, vec![]));
    assert_eq!(list_count(&h), 1);
    let round = h.state().env.lists.borrow()[0].0;
    h.state_mut().form.apply(&SettingsEvent::ModelsDone {
        round,
        result: Err(TranslateError::NoModelList),
    });
    run(&mut h);
    assert!(painted_texts(&h).contains(&tr("hint.model_no_list")));
    assert!(button_enabled(&h, &tr("field.thinking")));
}

#[test]
fn refresh_button_still_loads() {
    let mut h = harness(form(filled("openai"), vec![]));
    assert_eq!(list_count(&h), 1);
    refresh_models(&mut h, Ok(names(&["gpt-a"])));
    click(&mut h, &tr("button.refresh"));
    assert_eq!(list_count(&h), 2);
}

// --- 審查修正 ---

#[test]
fn caps_log_only_when_the_result_changes() {
    capture_logs();
    let mut h = harness(blank());
    type_into(&mut h, &tr("field.model"), "flood-mark");
    for c in ["a", "b", "c"] {
        h.event(Event::Text(c.into()));
        run(&mut h);
    }
    let lines = log_text()
        .lines()
        .filter(|l| l.contains("thinking caps resolved") && l.contains("model=flood-mark"))
        .count();
    // 一路都是「不知道、可切換、同一組選項」，判定結果沒變就不記
    assert_eq!(lines, 0);
}

#[test]
fn edit_and_tab_in_one_frame_still_auto_loads() {
    let mut h = harness(blank());
    click(&mut h, &tr("field.api_key"));
    h.event(Event::Text("sk-1".into()));
    for pressed in [true, false] {
        h.event(Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
    }
    run(&mut h);
    assert_eq!(list_count(&h), 1);
    let round = h.state().env.lists.borrow()[0].0;
    h.state_mut().form.apply(&SettingsEvent::ModelsDone {
        round,
        result: Ok(names(&["gpt-a"])),
    });
    run(&mut h);
    assert!(painted_texts(&h).contains(&t("hint.model_found", &[("count", "1")])));
}

#[test]
fn switching_provider_back_reloads_the_same_endpoint() {
    let mut service = new_service("custom", &[]);
    service["base_url"] = json!("http://127.0.0.1:8080");
    let mut h = harness(form(service, vec![]));
    assert_eq!(list_count(&h), 1);
    h.state_mut().form.set_provider("openai");
    h.state_mut().form.set_provider("custom");
    run(&mut h);
    type_into(&mut h, &tr("field.base_url"), "http://127.0.0.1:8080");
    click(&mut h, &tr("field.service_name"));
    assert_eq!(list_count(&h), 2);
}

#[test]
fn effort_comes_back_when_the_model_supports_it_again() {
    let mut service = filled("claude");
    service["model"] = json!("free");
    service["effort"] = json!("xhigh");
    let narrow = ThinkingCaps {
        efforts: Some(vec!["low".into()]),
        ..ThinkingCaps::default()
    };
    let mut h = harness(form(service, vec![]));
    refresh_models(
        &mut h,
        Ok(vec![
            with_caps("free", ThinkingCaps::default()),
            with_caps("narrow", narrow),
        ]),
    );
    replace_model(&mut h, "narrow");
    assert_eq!(h.state().form.values()["effort"], "auto");
    replace_model(&mut h, "free");
    assert_eq!(h.state().form.values()["effort"], "xhigh");
}

#[test]
fn caps_arriving_after_a_passed_test_clear_it_when_thinking_changes() {
    let mut service = filled("openai");
    service["model"] = json!("deep");
    let mut h = harness(form(service, vec![]));
    let round = start_test(&mut h);
    finish_test(&mut h, round, translated("x", 100));
    assert!(h.state().form.test_passed());
    refresh_models(
        &mut h,
        Ok(vec![with_caps("deep", locked(Some(true), Some(false)))]),
    );
    assert!(!h.state().form.test_passed());
}

#[test]
fn effort_hint_drops_the_unsupported_warning_when_levels_are_known() {
    let narrow = ThinkingCaps {
        efforts: Some(vec!["low".into(), "high".into()]),
        ..ThinkingCaps::default()
    };
    let mut h = claude_with(
        vec![
            with_caps("narrow", narrow),
            with_caps("free", ThinkingCaps::default()),
        ],
        "narrow",
    );
    assert!(painted_texts(&h).contains(&tr("hint.effort_known")));
    assert!(!painted_texts(&h).contains(&tr("hint.effort")));
    replace_model(&mut h, "free");
    assert!(painted_texts(&h).contains(&tr("hint.effort")));
}

// --- 換服務不沿用狀態、開啟時的載入佔位 ---

fn switch_node_id(h: &Harness<State>) -> String {
    format!(
        "{:?}",
        h.get_by_label(&tr("field.thinking")).accesskit_node().id()
    )
}

#[test]
fn each_service_keeps_its_own_widget_state() {
    let mut a = new_service("openai", &[]);
    a["thinking"] = json!(true);
    let b = new_service("openai", &[a.clone()]);
    let mut h = harness(form(a.clone(), vec![]));
    let first = switch_node_id(&h);
    h.state_mut().form = form(b, vec![]);
    run(&mut h);
    assert_ne!(switch_node_id(&h), first);
    h.state_mut().form = form(a, vec![]);
    run(&mut h);
    assert_eq!(switch_node_id(&h), first);
}

#[test]
fn opening_a_filled_form_shows_a_loading_placeholder() {
    let h = opening_harness(form(filled("openai"), vec![]));
    assert!(has_label(&h, &tr("form.loading")));
    assert!(!has_label(&h, &tr("field.service_name")));
}

#[test]
fn the_placeholder_gives_way_when_the_list_arrives() {
    let mut h = opening_harness(form(filled("claude"), vec![]));
    let round = h.state().env.lists.borrow()[0].0;
    h.state_mut().form.apply(&SettingsEvent::ModelsDone {
        round,
        result: Ok(names(&["claude-x"])),
    });
    h.step();
    assert!(has_label(&h, &tr("field.service_name")));
    assert!(!has_label(&h, &tr("form.loading")));
}

#[test]
fn the_placeholder_gives_way_after_its_time_cap() {
    let mut h = opening_harness(form(filled("openai"), vec![]));
    assert!(!has_label(&h, &tr("field.service_name")));
    settle(&mut h);
    assert!(has_label(&h, &tr("field.service_name")));
    assert!(has_label(&h, &tr("button.loading")));
}

#[test]
fn a_form_without_an_endpoint_has_no_placeholder() {
    let h = opening_harness(blank());
    assert!(has_label(&h, &tr("field.service_name")));
}

#[test]
fn later_loads_never_show_the_placeholder() {
    let mut h = opening_harness(blank());
    type_into(&mut h, &tr("field.api_key"), "sk-1");
    click(&mut h, &tr("field.service_name"));
    assert_eq!(list_count(&h), 1);
    assert!(has_label(&h, &tr("field.service_name")));
}

/// 這一幀畫出的實心矩形。
fn painted_rects(h: &Harness<State>) -> Vec<egui::Rect> {
    h.output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) => Some(rect.rect),
            _ => None,
        })
        .collect()
}

#[test]
fn placeholder_blocks_sit_where_the_fields_appear() {
    let mut h = opening_harness(form(filled("openai"), vec![]));
    let blocks = painted_rects(&h);
    settle(&mut h);
    for label in [tr("field.service_name"), tr("button.test")] {
        let field = h.get_by_label(&label).rect();
        assert!(
            blocks
                .iter()
                .any(|b| (b.center() - field.center()).length() < 1.0),
            "{label}: {field:?} not in {blocks:?}"
        );
    }
}

// --- 關閉思考被拒的說明、錯誤訊息可複製 ---

fn rejected(detail: &str) -> Result<TestOutcome, TranslateError> {
    Err(TranslateError::Config {
        status: Some(400),
        detail: detail.into(),
    })
}

#[test]
fn a_400_with_thinking_off_shows_only_the_server_error() {
    let mut service = filled("gemini");
    service["thinking"] = json!(false);
    let mut h = harness(form(service, vec![]));
    let round = start_test(&mut h);
    let error = TranslateError::Config {
        status: Some(400),
        detail: "Request contains an invalid argument.".into(),
    };
    finish_test(&mut h, round, Err(error.clone()));
    assert!(painted_texts(&h).contains(&friendly_text(&error)));
}

#[test]
fn the_test_error_can_be_selected_and_copied() {
    let mut h = harness(form(filled("openai"), vec![]));
    let round = start_test(&mut h);
    finish_test(
        &mut h,
        round,
        rejected("Request contains an invalid argument."),
    );
    let text = h
        .query_all_by(|n| {
            n.label()
                .or(n.value())
                .is_some_and(|l| l.contains("invalid argument"))
        })
        .next()
        .expect("error text shown")
        .rect();
    let (from, to) = (
        text.left_top() + egui::vec2(1.0, 4.0),
        text.right_bottom() - egui::vec2(1.0, 4.0),
    );
    h.event(Event::PointerMoved(from));
    h.event(Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    h.event(Event::PointerMoved(to));
    h.step();
    h.event(Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    h.event(Event::Copy);
    h.step();
    let copied: Vec<String> = h
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(
        copied.iter().any(|c| c.contains("invalid argument")),
        "{copied:?}"
    );
}

fn local_harness(models: Vec<Installed>) -> Harness<'static, State> {
    let mut service = new_service("local", &[]);
    service["model"] = json!("ready");
    let h = harness(form(service, vec![]));
    h.state().env.snapshot.borrow_mut().models = models;
    h
}

#[test]
fn local_form_has_no_key_or_endpoint_fields() {
    let h = local_harness(vec![]);
    for key in ["field.api_key", "field.api_key_optional", "field.base_url"] {
        assert!(!has_label(&h, &tr(key)), "{key}");
    }
    assert!(!has_label(&h, &tr("field.effort")));
    assert!(!has_label(&h, &tr("button.refresh")));
    assert!(has_label(&h, &tr("field.thinking")));
    assert!(has_label(&h, &tr("local.form.test_hint")));
}

#[test]
fn model_dropdown_lists_only_complete_downloads() {
    let mut h = local_harness(vec![
        installed("ready", true, 0),
        installed("partial", false, 0),
    ]);
    run(&mut h);
    click(&mut h, &tr("field.model"));
    let option = |id: &str| {
        t(
            "local.form.model_option",
            &[("name", &format!("{id} · Q4_K_M")), ("size", "0.0")],
        )
    };
    assert!(has_label(&h, &option("ready")));
    assert!(!has_label(&h, &option("partial")));
}

#[test]
fn picking_a_model_writes_its_id() {
    let mut h = local_harness(vec![
        installed("ready", true, 0),
        installed("other", true, 0),
    ]);
    click(&mut h, &tr("field.model"));
    let name = t(
        "local.form.model_option",
        &[("name", "other · Q4_K_M"), ("size", "0.0")],
    );
    click(&mut h, &name);
    assert_eq!(h.state().form.values()["model"], json!("other"));
}

#[test]
fn a_gone_model_is_flagged_and_the_link_asks_for_the_models_page() {
    let mut h = local_harness(vec![installed("other", true, 0)]);
    assert!(has_label(&h, &tr("local.form.model_missing")));
    assert!(has_label(&h, &tr("local.form.model_hint")));
    h.state_mut().responses.clear();
    click(&mut h, &tr("local.form.open_models"));
    assert!(h.state().responses.iter().any(|r| r.open_local_models));
}
