//! 設定視窗的內容：四個分頁、熱鍵錄製、檢查更新、儲存與取消（移植自 Python 的 test_window）。
//!
//! 換介面語言的測試在 `settings_language.rs`；本檔一律用同一個介面語言。

use crate::common;

use std::sync::{Arc, Mutex};

use eframe::egui::accesskit::{Role, Toggled};
use eframe::egui::{self, Key, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use common::kittest::{click, click_change, has_label, run, settle, text_sections, tr};
use common::log_capture::{capture_logs, log_lines_with};
use common::settings_env::FakeEnv;
use common::settings_window::{State, custom_cfg, harness as settings_harness};
use wizard101_chat_translator::config::services::{ServiceSlot, new_service};
use wizard101_chat_translator::config::{Config, app_dir, default_config};
use wizard101_chat_translator::i18n::{self, current_language, language_name, t};
use wizard101_chat_translator::input::dispatcher::Dispatcher;
use wizard101_chat_translator::input::dispatcher::testing::key;
use wizard101_chat_translator::runtime::updater::{
    AUTHOR_URL, CROWDIN_URL, ISSUES_URL, PROJECT_URL, Release,
};
use wizard101_chat_translator::ui::settings::env::SettingsEvent;
use wizard101_chat_translator::ui::settings::theme;
use wizard101_chat_translator::ui::settings::window::{Page, SettingsAction};
use wizard101_chat_translator::ui::text_select::{find_text_shape, opened_urls};
use wizard101_chat_translator::util;

const LANGUAGE: &str = "en-US";

/// 一份有單一預設服務（金鑰與模型已填）的設定；`fields` 覆寫該服務的欄位。
fn configured(provider: &str, fields: Value) -> Config {
    i18n::set_language(LANGUAGE);
    let mut cfg = default_config();
    let mut service = new_service(provider, &[]);
    let minimal = json!({"api_key": "k", "model": "m"});
    for (key, value) in minimal
        .as_object()
        .unwrap()
        .iter()
        .chain(fields.as_object().unwrap())
    {
        service[key] = value.clone();
    }
    cfg.insert("default_service".into(), service["id"].clone());
    cfg.insert("services".into(), json!([service]));
    cfg
}

fn configured_cfg() -> Config {
    i18n::set_language(LANGUAGE);
    custom_cfg()
}

/// 在 `cfg` 加一筆設定完整（或用 `fields` 改過）的服務並回傳它。
fn add_service(cfg: &mut Config, provider: &str, fields: Value) -> Value {
    let services = cfg["services"].as_array().unwrap().clone();
    let mut service = new_service(provider, &services);
    service["api_key"] = json!("k");
    service["model"] = json!("m");
    for (key, value) in fields.as_object().unwrap() {
        service[key] = value.clone();
    }
    cfg["services"]
        .as_array_mut()
        .unwrap()
        .push(service.clone());
    service
}

fn harness(cfg: Config) -> Harness<'static, State> {
    harness_with(cfg, FakeEnv::default(), egui::vec2(840.0, 660.0))
}

fn harness_with(cfg: Config, env: FakeEnv, size: egui::Vec2) -> Harness<'static, State> {
    settings_harness(&cfg, env, size, LANGUAGE)
}

/// 點連結，回傳這一幀要開啟的網址（開啟指令只在點下的那一幀輸出）。
fn click_link<S>(h: &mut Harness<S>, label: &str) -> Vec<String> {
    h.get_by_role_and_label(Role::Link, label).click();
    h.step();
    opened_urls(&h.output().platform_output)
}

/// 顯示 `value` 這段文字的標籤（不含它底下的文字段節點）。
fn text<'h, S>(h: &'h Harness<S>, value: &'h str) -> egui_kittest::Node<'h> {
    h.get_by(|n| n.role() == Role::Label && n.value().as_deref() == Some(value))
}

fn go(h: &mut Harness<State>, tab: &str) {
    click(h, Role::Tab, &tr(tab));
}

fn save(h: &mut Harness<State>) -> Option<Config> {
    click(h, Role::Button, &tr("button.save"));
    h.state().actions.iter().find_map(|a| match a {
        SettingsAction::Save(cfg) => Some(cfg.clone()),
        _ => None,
    })
}

fn top<S>(h: &Harness<S>, role: Role, label: &str) -> f32 {
    h.get_by_role_and_label(role, label).rect().top()
}

/// 依畫面由上而下的第 `index` 個〔更改〕：0 是輸入框熱鍵，1 是框選熱鍵。
/// 錄下 `combo` 交給框選熱鍵那一格。
fn record_region(h: &mut Harness<State>, combo: &str) {
    click_change(h, 1);
    let round = *h.state().env.recordings.borrow().last().unwrap();
    h.state_mut().win.apply(&SettingsEvent::HotkeyRecorded {
        round,
        combo: combo.into(),
    });
    run(h);
}

fn service_name(cfg: &Config, index: usize) -> String {
    cfg["services"][index]["name"].as_str().unwrap().to_owned()
}

// --- 版面 ---

#[test]
fn the_services_tab_sits_between_basic_and_advanced() {
    // 服務是設定的主角，要緊接在基本分頁之後，不能被塞到「進階」裡
    let h = harness(configured_cfg());
    let mut tabs: Vec<_> = h.query_all_by_role(Role::Tab).collect();
    tabs.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    let labels: Vec<String> = tabs
        .iter()
        .filter_map(|n| n.accesskit_node().label())
        .collect();
    assert_eq!(
        labels,
        [
            tr("settings.tab.basic"),
            tr("settings.tab.services"),
            tr("settings.tab.advanced"),
            tr("settings.tab.about")
        ]
    );
    assert_eq!(h.state().win.page(), Page::Basic);
}

#[test]
fn the_sidebar_shows_the_version() {
    let h = harness(configured_cfg());
    assert!(has_label(&h, &format!("v{}", env!("CARGO_PKG_VERSION"))));
}

#[test]
fn both_language_fields_come_before_the_hotkeys() {
    // 介面語言與翻譯目標語言是最容易被搞混的一對，要相鄰且排在熱鍵設定之前
    let h = harness(configured_cfg());
    let ui_language = top(&h, Role::ComboBox, &tr("field.ui_language"));
    let target = top(&h, Role::TextInput, &tr("settings.target_language"));
    let mut changes: Vec<f32> = h
        .query_all_by_role_and_label(Role::Button, &tr("button.change"))
        .map(|n| n.rect().top())
        .collect();
    changes.sort_by(f32::total_cmp);
    assert!(
        ui_language < target && target < changes[0],
        "{ui_language} {target} {changes:?}"
    );
}

#[test]
fn service_summary_row_sits_between_target_language_and_hotkey_label() {
    let h = harness(configured_cfg());
    let target = top(&h, Role::TextInput, &tr("settings.target_language"));
    let manage = top(&h, Role::Button, &tr("button.manage"));
    let hotkey = text(&h, &tr("settings.hotkey")).rect().top();
    assert!(
        target < manage && manage < hotkey,
        "{target} {manage} {hotkey}"
    );
}

#[test]
fn the_basic_tab_labels_the_service_row_with_the_default_service() {
    // 那一列自成一條文案：沿用分頁標題或分頁內的「預設服務」都會讓人誤讀
    let h = harness(configured_cfg());
    let label = text(&h, &tr("settings.default_service")).rect();
    let manage = h
        .get_by_role_and_label(Role::Button, &tr("button.manage"))
        .rect();
    assert!(
        (label.center().y - manage.center().y).abs() < 1.0,
        "{label:?} vs {manage:?}"
    );
}

#[test]
fn service_summary_shows_the_draft_default_service_name() {
    let cfg = configured_cfg();
    let name = service_name(&cfg, 0);
    let h = harness(cfg);
    let summary = text(&h, &name).rect();
    let manage = h
        .get_by_role_and_label(Role::Button, &tr("button.manage"))
        .rect();
    assert!(
        summary.right() <= manage.left(),
        "{summary:?} vs {manage:?}"
    );
}

#[test]
fn service_summary_shows_a_dash_without_services() {
    i18n::set_language(LANGUAGE);
    let h = harness(default_config());
    text(&h, "—");
}

#[test]
fn manage_button_switches_to_the_services_page() {
    let mut h = harness(configured_cfg());
    click(&mut h, Role::Button, &tr("button.manage"));
    assert_eq!(h.state().win.page(), Page::Services);
    h.get_by_role_and_label(Role::ComboBox, &tr("service.default"));
}

#[test]
fn service_summary_refreshes_when_the_default_changes() {
    let mut cfg = configured_cfg();
    let added = add_service(&mut cfg, "openai", json!({}));
    let added_name = added["name"].as_str().unwrap().to_owned();
    let mut h = harness(cfg);
    go(&mut h, "settings.tab.services");
    click(&mut h, Role::ComboBox, &tr("service.default"));
    click(&mut h, Role::Button, &added_name);
    go(&mut h, "settings.tab.basic");
    text(&h, &added_name);
}

#[test]
fn set_page_shows_that_page() {
    let mut h = harness(configured_cfg());
    h.state_mut().win.set_page(Page::About);
    run(&mut h);
    assert_eq!(h.state().win.page(), Page::About);
    assert!(has_label(&h, &tr("about.logs")));
}

#[test]
fn the_footer_buttons_end_at_the_content_margin() {
    // right_aligned 不可因為 item_spacing 把最後一顆按鈕推出右緣
    let h = harness(configured_cfg());
    // 測試框架在外圍留 8 的邊，按鈕列右側內距 24
    let right = h.ctx.content_rect().right() - 8.0 - 24.0;
    let save = h
        .get_by_role_and_label(Role::Button, &tr("button.save"))
        .rect();
    let cancel = h
        .get_by_role_and_label(Role::Button, &tr("button.cancel"))
        .rect();
    assert!((save.right() - right).abs() < 1.0, "{save:?} vs {right}");
    assert!(cancel.right() < save.left(), "{cancel:?} then {save:?}");
}

#[test]
fn the_footer_stays_visible_in_a_small_window() {
    let h = harness_with(
        configured_cfg(),
        FakeEnv::default(),
        egui::vec2(640.0, 360.0),
    );
    let save = h
        .get_by_role_and_label(Role::Button, &tr("button.save"))
        .rect();
    assert!(save.bottom() <= 360.0, "{save:?}");
    let paste = h.get_by_label(&tr("field.paste_hotkey")).rect();
    assert!(
        paste.top() > save.top(),
        "the last card is below the fold: {paste:?}"
    );
}

// --- 儲存 ---

/// 設定視窗管的鍵；儲存只送這些，由 App 併進目前共用的設定。
const OWNED_KEYS: [&str; 15] = [
    "services",
    "default_service",
    "service_slots",
    "target_language",
    "hotkey",
    "region_hotkey",
    "auto_show_input",
    "paste_hotkey",
    "translate_system_messages",
    "fade_seconds",
    "max_messages",
    "max_parallel_translations",
    "type_delay",
    "overlay_alpha",
    "ui_language",
];

#[test]
fn saving_an_unchanged_window_sends_its_own_keys_with_the_ui_language() {
    let cfg = configured_cfg();
    let mut h = harness(cfg.clone());
    let saved = save(&mut h).expect("saved");
    let mut expected = cfg;
    expected.insert("ui_language".into(), json!(LANGUAGE));
    expected.retain(|key, _| OWNED_KEYS.contains(&key.as_str()));
    assert_eq!(saved, expected);
}

#[test]
fn save_carries_only_the_keys_the_window_owns() {
    // overlay 位置等鍵在設定視窗開著時仍會被別處改寫，送開窗時的舊值會把它們蓋回去
    let mut cfg = configured_cfg();
    cfg.insert("unknown_key".into(), json!("kept elsewhere"));
    let mut h = harness(cfg);
    let saved = save(&mut h).unwrap();
    let mut keys: Vec<&str> = saved.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut owned = OWNED_KEYS.to_vec();
    owned.sort_unstable();
    assert_eq!(keys, owned);
    for foreign in [
        "overlay",
        "bubble_position",
        "config_version",
        "unknown_key",
    ] {
        assert!(!saved.contains_key(foreign), "{foreign} must not be saved");
    }
}

#[test]
fn unknown_service_slot_keys_survive_a_save() {
    let mut cfg = configured_cfg();
    cfg["service_slots"]["future_slot"] = json!("x");
    let mut h = harness(cfg);
    assert_eq!(
        save(&mut h).unwrap()["service_slots"]["future_slot"],
        json!("x")
    );
}

#[test]
fn toggles_follow_python_truthiness() {
    // 手改設定檔寫成 1／0 也要照 Python 的真值顯示
    let mut cfg = configured_cfg();
    cfg.insert("translate_system_messages".into(), json!(1));
    cfg.insert("paste_hotkey".into(), json!(0));
    let mut h = harness(cfg);
    let label = tr("field.paste_hotkey");
    let paste = h.get_by_label(&label).accesskit_node().toggled();
    assert_eq!(paste, Some(Toggled::False));
    go(&mut h, "settings.tab.advanced");
    let label = tr("field.translate_system");
    let system = h.get_by_label(&label).accesskit_node().toggled();
    assert_eq!(system, Some(Toggled::True));
    let saved = save(&mut h).unwrap();
    assert_eq!(saved["translate_system_messages"], json!(true));
    assert_eq!(saved["paste_hotkey"], json!(false));
}

#[test]
fn a_missing_alpha_falls_back_to_the_default() {
    let mut cfg = configured_cfg();
    cfg.insert("overlay_alpha".into(), json!(null));
    let mut h = harness(cfg);
    assert_eq!(
        save(&mut h).unwrap()["overlay_alpha"],
        default_config()["overlay_alpha"]
    );
}

#[test]
fn save_stores_the_system_message_toggle() {
    let mut h = harness(configured_cfg());
    go(&mut h, "settings.tab.advanced");
    let label = tr("field.translate_system");
    let toggle = h.get_by_label(&label);
    assert_eq!(toggle.accesskit_node().toggled(), Some(Toggled::False)); // 預設關閉
    toggle.click();
    run(&mut h);
    assert_eq!(
        save(&mut h).unwrap()["translate_system_messages"],
        json!(true)
    );
}

#[test]
fn reopening_settings_reflects_the_saved_toggle() {
    let mut cfg = configured_cfg();
    cfg.insert("translate_system_messages".into(), json!(true));
    let mut h = harness(cfg);
    go(&mut h, "settings.tab.advanced");
    let label = tr("field.translate_system");
    let toggle = h.get_by_label(&label);
    assert_eq!(toggle.accesskit_node().toggled(), Some(Toggled::True));
}

#[test]
fn paste_hotkey_defaults_on_and_saves_from_the_settings_toggle() {
    assert_eq!(default_config()["paste_hotkey"], json!(true));
    let mut h = harness(configured_cfg());
    let label = tr("field.paste_hotkey");
    let toggle = h.get_by_label(&label);
    assert_eq!(toggle.accesskit_node().toggled(), Some(Toggled::True));
    toggle.click();
    run(&mut h);
    assert_eq!(save(&mut h).unwrap()["paste_hotkey"], json!(false));
}

#[test]
fn auto_input_saves_from_its_toggle() {
    let mut h = harness(configured_cfg());
    click(&mut h, Role::Switch, &tr("field.auto_input"));
    assert_eq!(save(&mut h).unwrap()["auto_show_input"], json!(false));
}

#[test]
fn save_writes_the_edited_service_list_into_cfg() {
    let cfg = configured_cfg();
    let name = service_name(&cfg, 0);
    let mut h = harness(cfg);
    go(&mut h, "settings.tab.services");
    click(&mut h, Role::Button, &tr("button.edit"));
    settle(&mut h);
    h.get_by_label(&tr("field.service_name")).click();
    run(&mut h);
    h.event(egui::Event::Text(" 2".into()));
    run(&mut h);
    h.get_by_role_and_label(Role::Dialog, &tr("service.dialog_title"))
        .get_by_role_and_label(Role::Button, &tr("button.ok"))
        .click();
    run(&mut h);
    let saved = save(&mut h).unwrap();
    assert_eq!(service_name(&saved, 0), format!("{name} 2"));
}

#[test]
fn save_writes_the_advanced_values() {
    let mut h = harness(configured_cfg());
    go(&mut h, "settings.tab.advanced");
    let increase: Vec<_> = h.query_all_by_role_and_label(Role::Button, "+").collect();
    assert_eq!(increase.len(), 4);
    let mut increase = increase;
    increase.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    increase[0].click();
    run(&mut h);
    let saved = save(&mut h).unwrap();
    assert_eq!(saved["fade_seconds"], json!(10));
    assert_eq!(saved["max_messages"], json!(200));
}

#[test]
fn an_out_of_range_advanced_value_is_clamped_when_the_field_loses_focus() {
    let mut h = harness(configured_cfg());
    go(&mut h, "settings.tab.advanced");
    let label = tr("settings.max_messages");
    h.get_by_label(&label).click();
    run(&mut h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text("5000".into()));
    run(&mut h);
    assert_eq!(h.get_by_label(&label).value().as_deref(), Some("5000"));
    h.key_press(Key::Tab);
    run(&mut h);
    assert_eq!(h.get_by_label(&label).value().as_deref(), Some("1000"));
}

#[test]
fn a_non_number_advanced_value_blocks_the_save() {
    let mut h = harness(configured_cfg());
    go(&mut h, "settings.tab.advanced");
    h.get_by_label(&tr("settings.max_messages")).click();
    run(&mut h);
    h.event(egui::Event::Text("x".into()));
    run(&mut h);
    assert!(save(&mut h).is_none());
    assert!(has_label(&h, &tr("error.advanced_not_number")));
}

#[test]
fn save_refuses_an_incomplete_default_service() {
    // 清單裡明明有一組服務，要指出缺的是哪一欄，不可報「請先新增一組翻譯服務」
    let mut h = harness(configured("openai", json!({"api_key": ""})));
    assert!(save(&mut h).is_none());
    assert!(has_label(&h, &tr("dialog.incomplete_title")));
    assert!(has_label(&h, &tr("error.need_api_key")));
    assert!(!has_label(&h, &tr("error.need_service")));
}

#[test]
fn save_without_any_service_asks_for_one() {
    i18n::set_language(LANGUAGE);
    let mut h = harness(default_config());
    assert!(save(&mut h).is_none());
    assert!(has_label(&h, &tr("error.need_service")));
}

#[test]
fn save_refuses_a_service_a_slot_points_at() {
    // 插槽指到的服務也要填得完整：執行期真的會拿它送請求
    let mut cfg = configured_cfg();
    let half_done = add_service(&mut cfg, "openai", json!({"model": ""}));
    cfg["service_slots"][ServiceSlot::Incoming.key()] = half_done["id"].clone();
    let mut h = harness(cfg);
    assert!(save(&mut h).is_none());
    assert!(has_label(&h, &tr("error.need_model")));
}

#[test]
fn save_lists_a_shared_services_problem_only_once() {
    let mut cfg = configured("openai", json!({"api_key": ""}));
    cfg["service_slots"][ServiceSlot::Outgoing.key()] = cfg["default_service"].clone();
    let mut h = harness(cfg);
    assert!(save(&mut h).is_none());
    assert_eq!(h.query_all_by_label(&tr("error.need_api_key")).count(), 1);
}

#[test]
fn save_is_blocked_with_a_callout_listing_each_problem() {
    capture_logs();
    let mut cfg = configured("openai", json!({"api_key": ""}));
    cfg.insert("region_hotkey".into(), cfg["hotkey"].clone());
    let mut h = harness(cfg);
    assert!(save(&mut h).is_none());
    let title = text(&h, &tr("dialog.incomplete_title")).rect();
    let key = text(&h, &tr("error.need_api_key")).rect();
    let hotkeys = text(&h, &tr("error.hotkeys_same")).rect();
    assert!(title.top() < key.top() && key.top() < hotkeys.top());
    let cancel = h
        .get_by_role_and_label(Role::Button, &tr("button.cancel"))
        .rect();
    assert!(
        hotkeys.right() < cancel.left(),
        "the callout sits left of the buttons"
    );
    assert!(
        !log_lines_with("[settings] save blocked: error.need_api_key, error.hotkeys_same")
            .is_empty()
    );
}

#[test]
fn a_blocked_save_clears_its_callout_once_fixed() {
    let mut cfg = configured_cfg();
    cfg.insert("region_hotkey".into(), cfg["hotkey"].clone());
    let mut h = harness(cfg);
    assert!(save(&mut h).is_none());
    record_region(&mut h, "ctrl+alt+r");
    assert!(!has_label(&h, &tr("error.hotkeys_same")));
    assert!(save(&mut h).is_some());
}

#[test]
fn save_blank_target_language_is_refused() {
    let mut cfg = configured_cfg();
    cfg.insert("target_language".into(), json!("   "));
    let mut h = harness(cfg);
    assert!(save(&mut h).is_none());
    assert!(has_label(&h, &tr("error.need_target_language")));
}

#[test]
fn a_blank_target_language_tests_the_connection_in_the_ui_language() {
    let mut cfg = configured_cfg();
    cfg.insert("target_language".into(), json!("   "));
    let mut h = harness(cfg);
    go(&mut h, "settings.tab.services");
    click(&mut h, Role::Button, &tr("button.edit"));
    settle(&mut h);
    click(&mut h, Role::Button, &tr("button.test"));
    let tests = h.state().env.tests.borrow();
    let (_, _, target) = tests.last().expect("test started");
    assert_eq!(*target, language_name(&current_language()));
}

#[test]
fn the_target_language_is_saved_stripped() {
    let mut cfg = configured_cfg();
    cfg.insert("target_language".into(), json!("  日本語 "));
    let mut h = harness(cfg);
    assert_eq!(save(&mut h).unwrap()["target_language"], json!("日本語"));
}

// --- 熱鍵錄製 ---

#[test]
fn save_stores_the_region_hotkey() {
    let mut h = harness(configured_cfg());
    record_region(&mut h, "ctrl+alt+r");
    assert_eq!(save(&mut h).unwrap()["region_hotkey"], json!("ctrl+alt+r"));
}

#[test]
fn save_rejects_identical_hotkeys() {
    let mut h = harness(configured_cfg());
    record_region(&mut h, "ctrl+space");
    assert!(save(&mut h).is_none());
    assert!(has_label(&h, &tr("error.hotkeys_same")));
}

#[test]
fn save_rejects_a_ctrl_v_hotkey_while_paste_is_on() {
    let mut h = harness(configured_cfg());
    record_region(&mut h, "ctrl+v");
    assert!(save(&mut h).is_none());
    assert!(has_label(&h, &tr("error.hotkey_is_paste")));
}

#[test]
fn a_ctrl_v_hotkey_saves_while_paste_is_off() {
    let mut cfg = configured_cfg();
    cfg.insert("paste_hotkey".into(), json!(false));
    let mut h = harness(cfg);
    record_region(&mut h, "ctrl+v");
    assert_eq!(save(&mut h).unwrap()["region_hotkey"], json!("ctrl+v"));
}

#[test]
fn recording_shows_the_press_key_prompt_and_releases_the_recorder_after() {
    let mut h = harness(configured_cfg());
    click_change(&mut h, 0);
    assert_eq!(h.state().env.recordings.borrow().len(), 1);
    assert!(has_label(&h, &tr("button.press_key")));
    assert_eq!(
        h.query_all_by_role_and_label(Role::Button, &tr("button.change"))
            .count(),
        1
    );
    let round = h.state().env.recordings.borrow()[0];
    h.state_mut().win.apply(&SettingsEvent::HotkeyRecorded {
        round,
        combo: "ctrl+alt+k".into(),
    });
    run(&mut h);
    assert_eq!(h.state().env.cancels.get(), 1);
    assert!(!has_label(&h, &tr("button.press_key")));
    assert!(has_label(&h, "K"));
    assert_eq!(save(&mut h).unwrap()["hotkey"], json!("ctrl+alt+k"));
}

#[test]
fn enter_on_a_focused_change_button_does_not_record_enter() {
    let dispatcher = Arc::new(Mutex::new(Dispatcher::new()));
    let env = FakeEnv {
        dispatcher: Some(dispatcher.clone()),
        ..FakeEnv::default()
    };
    let mut h = harness_with(configured_cfg(), env, egui::vec2(840.0, 660.0));
    let change = tr("button.change");
    for _ in 0..40 {
        if h.query_by(|n| n.is_focused() && n.label().as_deref() == Some(change.as_str()))
            .is_some()
        {
            break;
        }
        h.key_press(Key::Tab);
        run(&mut h);
    }
    h.key_down(Key::Enter);
    run(&mut h);
    assert_eq!(h.state().env.recordings.borrow().len(), 1);
    // 鍵盤 hook 看到的是錄製開始後才來的 Enter 放開
    let enter_up = key(false, 28, 0x0D);
    dispatcher.lock().unwrap().process(enter_up);
    h.key_up(Key::Enter);
    run(&mut h);
    assert!(h.state().env.recorded.lock().unwrap().is_empty());
    assert!(has_label(&h, &tr("button.press_key")));
    assert_eq!(save(&mut h).unwrap()["hotkey"], json!("ctrl+space"));
}

#[test]
fn escape_or_nothing_keeps_the_hotkey() {
    let mut h = harness(configured_cfg());
    record_region(&mut h, "esc");
    record_region(&mut h, "");
    assert_eq!(
        save(&mut h).unwrap()["region_hotkey"],
        json!("ctrl+shift+space")
    );
}

#[test]
fn closing_while_recording_cancels_it() {
    let mut h = harness(configured_cfg());
    click_change(&mut h, 0);
    let State { win, env, .. } = h.state_mut();
    let actions = win.cancel(env);
    assert_eq!(actions, [SettingsAction::Cancel]);
    assert_eq!(h.state().env.cancels.get(), 1);
}

#[test]
fn changing_page_while_recording_cancels_it() {
    let mut h = harness(configured_cfg());
    click_change(&mut h, 0);
    go(&mut h, "settings.tab.advanced");
    assert_eq!(h.state().env.cancels.get(), 1);
    let round = h.state().env.recordings.borrow()[0];
    h.state_mut().win.apply(&SettingsEvent::HotkeyRecorded {
        round,
        combo: "ctrl+alt+k".into(),
    });
    go(&mut h, "settings.tab.basic");
    assert!(!has_label(&h, &tr("button.press_key")));
    assert_eq!(save(&mut h).unwrap()["hotkey"], json!("ctrl+space"));
}

#[test]
fn set_page_while_recording_cancels_it() {
    let mut h = harness(configured_cfg());
    click_change(&mut h, 0);
    h.state_mut().win.set_page(Page::About);
    run(&mut h);
    assert_eq!(h.state().env.cancels.get(), 1);
}

#[test]
fn starting_another_recording_cancels_the_first() {
    let mut h = harness(configured_cfg());
    click_change(&mut h, 0);
    click_change(&mut h, 0); // 錄製中只剩框選那一格的〔更改〕
    assert_eq!(h.state().env.recordings.borrow().len(), 2);
    assert_eq!(h.state().env.cancels.get(), 1);
    let first = h.state().env.recordings.borrow()[0];
    h.state_mut().win.apply(&SettingsEvent::HotkeyRecorded {
        round: first,
        combo: "ctrl+alt+k".into(),
    });
    run(&mut h);
    let saved = save(&mut h).unwrap();
    assert_eq!(saved["hotkey"], json!("ctrl+space"));
    assert_eq!(saved["region_hotkey"], json!("ctrl+shift+space"));
}

// --- 透明度與取消 ---

#[test]
fn alpha_slider_previews_and_cancel_restores() {
    let mut h = harness(configured_cfg());
    go(&mut h, "settings.tab.advanced");
    h.get_by_role(Role::Slider).set_value_accesskit(0.5);
    run(&mut h);
    assert!(
        h.state()
            .actions
            .contains(&SettingsAction::PreviewAlpha(0.5))
    );
    assert!(has_label(&h, "0.50"));
    h.state_mut().actions.clear();
    click(&mut h, Role::Button, &tr("button.cancel"));
    assert_eq!(
        h.state().actions,
        [SettingsAction::PreviewAlpha(0.8), SettingsAction::Cancel]
    );
}

#[test]
fn the_previewed_alpha_is_saved() {
    let mut h = harness(configured_cfg());
    go(&mut h, "settings.tab.advanced");
    h.get_by_role(Role::Slider).set_value_accesskit(0.5);
    run(&mut h);
    assert_eq!(save(&mut h).unwrap()["overlay_alpha"], json!(0.5));
}

#[test]
fn cancel_without_changes_only_cancels() {
    let mut h = harness(configured_cfg());
    click(&mut h, Role::Button, &tr("button.cancel"));
    assert_eq!(h.state().actions, [SettingsAction::Cancel]);
}

// --- 關於 ---

#[test]
fn about_tab_shows_version_and_links() {
    let mut h = harness(configured_cfg());
    go(&mut h, "settings.tab.about");
    text(
        &h,
        &format!("{} v{}", tr("about.version"), env!("CARGO_PKG_VERSION")),
    );
    for url in [PROJECT_URL, CROWDIN_URL, ISSUES_URL] {
        h.get_by_role_and_label(Role::Link, url.trim_start_matches("https://"));
    }
    assert_eq!(click_link(&mut h, "GoneTone"), [AUTHOR_URL]);
    let project = PROJECT_URL.trim_start_matches("https://");
    assert_eq!(click_link(&mut h, project), [PROJECT_URL]);
}

#[test]
fn about_tab_shows_the_log_folder() {
    let mut h = harness(configured_cfg());
    go(&mut h, "settings.tab.about");
    text(&h, &app_dir().display().to_string());
    click(&mut h, Role::Button, &tr("button.open_folder"));
    assert_eq!(*h.state().env.opened.borrow(), [app_dir()]);
}

#[test]
fn about_tab_shows_the_translators() {
    // en-US 的譯者掛名是 Claude Code；掛名接在開發者之下、協助翻譯與回報問題之前
    let mut h = harness(configured_cfg());
    go(&mut h, "settings.tab.about");
    let author = top(&h, Role::Link, "GoneTone");
    let translators = top(&h, Role::Link, "Claude Code");
    let contribute = top(&h, Role::Link, CROWDIN_URL.trim_start_matches("https://"));
    let issues = top(&h, Role::Link, ISSUES_URL.trim_start_matches("https://"));
    assert!(author < translators && translators < contribute && contribute < issues);
    let label = text(&h, &tr("credit.translators")).rect();
    assert!((label.top() - translators).abs() < 24.0);
}

const UP_TO_DATE_LOG: &str = "[update] manual check: already up to date";

/// 把檢查結果交給視窗，回傳這一次多記了幾行 `UP_TO_DATE_LOG`；所有送 `UpdateChecked` 的測試都走這裡，
/// 鎖才能讓各測試的 log 筆數互不干擾。
fn deliver_check(
    h: &mut Harness<State>,
    round: u64,
    result: Result<Option<Release>, String>,
) -> usize {
    static LOCK: Mutex<()> = Mutex::new(());
    let _serial = util::lock(&LOCK);
    capture_logs();
    let before = log_lines_with(UP_TO_DATE_LOG).len();
    h.state_mut()
        .win
        .apply(&SettingsEvent::UpdateChecked { round, result });
    run(h);
    log_lines_with(UP_TO_DATE_LOG).len() - before
}

/// 在關於頁按〔檢查更新〕並送回 `result`；回傳多記的 `UP_TO_DATE_LOG` 行數。
fn check_update(h: &mut Harness<State>, result: Result<Option<Release>, String>) -> usize {
    go(h, "settings.tab.about");
    click(h, Role::Button, &tr("button.check_update"));
    let round = *h.state().env.checks.borrow().last().unwrap();
    deliver_check(h, round, result)
}

fn release() -> Release {
    Release {
        version: "9.9.9".into(),
        url: "https://example.invalid/rel".into(),
    }
}

fn update_button_enabled(h: &Harness<State>) -> bool {
    !h.get_by_role_and_label(Role::Button, &tr("button.check_update"))
        .accesskit_node()
        .is_disabled()
}

#[test]
fn checking_disables_the_button_until_the_result() {
    let mut h = harness(configured_cfg());
    go(&mut h, "settings.tab.about");
    click(&mut h, Role::Button, &tr("button.check_update"));
    assert_eq!(h.state().env.checks.borrow().len(), 1);
    let label = tr("button.checking");
    let checking = h.get_by_role_and_label(Role::Button, &label);
    assert!(checking.accesskit_node().is_disabled());
    let round = h.state().env.checks.borrow()[0];
    deliver_check(&mut h, round, Ok(None));
    assert!(update_button_enabled(&h));
}

#[test]
fn manual_check_reports_up_to_date() {
    let mut h = harness(configured_cfg());
    let logged = check_update(&mut h, Ok(None));
    text(&h, &tr("update.latest"));
    assert!(update_button_enabled(&h));
    assert_eq!(logged, 1);
}

#[test]
fn manual_check_keeps_update_found_quiet_when_up_to_date() {
    let mut h = harness(configured_cfg());
    check_update(&mut h, Ok(None));
    assert!(
        !h.state()
            .actions
            .iter()
            .any(|a| matches!(a, SettingsAction::UpdateFound(_)))
    );
}

#[test]
fn manual_check_hands_a_new_version_to_update_found() {
    let mut h = harness(configured_cfg());
    check_update(&mut h, Ok(Some(release())));
    assert_eq!(h.state().actions, [SettingsAction::UpdateFound(release())]);
}

#[test]
fn manual_check_reports_a_new_version() {
    let mut h = harness(configured_cfg());
    check_update(&mut h, Ok(Some(release())));
    let text = t("update.available", &[("version", "9.9.9")]);
    assert_eq!(click_link(&mut h, &text), ["https://example.invalid/rel"]);
}

#[test]
fn update_result_is_only_clickable_over_its_text() {
    // 「有新版」整行是連結，但只到文字為止；不可撐滿整列讓空白也可點
    let mut h = harness(configured_cfg());
    check_update(&mut h, Ok(Some(release())));
    let label = t("update.available", &[("version", "9.9.9")]);
    let link = h.get_by_role_and_label(Role::Link, &label).rect();
    let text = find_text_shape(&h.output().shapes, &label)
        .map(|s| s.galley.rect.width())
        .expect("link text painted");
    assert!((link.width() - text).abs() < 1.0, "{link:?} vs text {text}");
}

#[test]
fn manual_check_reports_failure() {
    let mut h = harness(configured_cfg());
    check_update(&mut h, Err("HTTP 403".into()));
    text(&h, &t("update.failed", &[("error", "HTTP 403")]));
    assert!(update_button_enabled(&h));
}

#[test]
fn about_tab_hides_the_cache_row_without_a_cache() {
    let mut h = harness(configured_cfg());
    go(&mut h, "settings.tab.about");
    assert!(!has_label(&h, &tr("about.cache")));
    assert!(!has_label(&h, &tr("button.clear_cache")));
}

fn cache_harness(count: usize) -> Harness<'static, State> {
    let env = FakeEnv {
        cache: Some(count),
        ..FakeEnv::default()
    };
    let mut h = harness_with(configured_cfg(), env, egui::vec2(840.0, 660.0));
    go(&mut h, "settings.tab.about");
    h
}

#[test]
fn clear_cache_button_clears_and_reports_the_count() {
    let mut h = cache_harness(42);
    let cleared = t("about.cache_cleared", &[("count", "42")]);
    assert!(!has_label(&h, &cleared)); // 還沒按之前不顯示任何結果
    click(&mut h, Role::Button, &tr("button.clear_cache"));
    assert_eq!(h.state().env.cleared.get(), 1);
    text(&h, &cleared);
}

#[test]
fn a_failed_cache_clear_reports_failure() {
    let env = FakeEnv {
        cache: Some(3),
        clear_fails: true,
        ..FakeEnv::default()
    };
    let mut h = harness_with(configured_cfg(), env, egui::vec2(840.0, 660.0));
    go(&mut h, "settings.tab.about");
    click(&mut h, Role::Button, &tr("button.clear_cache"));
    text(&h, &tr("about.cache_clear_failed"));
    assert!(!has_label(&h, &t("about.cache_cleared", &[("count", "3")])));
}

#[test]
fn clear_cache_reports_zero_when_the_cache_was_already_empty() {
    let mut h = cache_harness(0);
    click(&mut h, Role::Button, &tr("button.clear_cache"));
    text(&h, &t("about.cache_cleared", &[("count", "0")]));
}

// --- 介面語言列的譯者掛名 ---

#[test]
fn translators_row_follows_the_ui_language_field() {
    // 譯者掛的是選到的這個介面語言，所以就在介面語言那一列，不與翻譯目標語言隔開
    let h = harness(configured_cfg());
    let ui_language = h
        .get_by_role_and_label(Role::ComboBox, &tr("field.ui_language"))
        .rect();
    let target = top(&h, Role::TextInput, &tr("settings.target_language"));
    let credit = h.get_by_role_and_label(Role::Link, "Claude Code").rect();
    let help = h
        .get_by_role_and_label(Role::Link, &format!("{} ↗", tr("credit.help_translate")))
        .rect();
    for rect in [credit, help] {
        assert!(
            rect.top() >= ui_language.top() - 20.0 && rect.bottom() < target,
            "{rect:?}"
        );
        assert!(rect.right() < ui_language.left());
    }
    let label = h
        .get_by(|n| {
            n.role() == Role::Label
                && n.value()
                    .is_some_and(|v| v.trim() == tr("credit.translators"))
        })
        .rect();
    assert!(label.right() <= credit.left() + 1.0);
}

#[test]
fn help_translate_link_opens_crowdin() {
    let mut h = harness(configured_cfg());
    let label = format!("{} ↗", tr("credit.help_translate"));
    assert_eq!(click_link(&mut h, &label), [CROWDIN_URL]);
}

#[test]
fn the_target_language_offers_the_common_languages() {
    let mut h = harness(configured_cfg());
    h.get_by_role_and_label(Role::TextInput, &tr("settings.target_language"))
        .click();
    run(&mut h);
    let rect: Rect = h.get_by_role_and_label(Role::Button, "日本語").rect();
    assert!(rect.width() > 0.0);
    h.get_by_role_and_label(Role::Button, "English");
}

#[test]
fn footer_buttons_sit_inside_the_window_with_centred_labels() {
    let h = harness(configured_cfg());
    let window = h.ctx.content_rect();
    for label in [tr("button.cancel"), tr("button.save")] {
        let button = h.get_by_role_and_label(Role::Button, &label).rect();
        assert!(window.contains_rect(button), "{label}: {button:?}");
        assert!(
            window.bottom() - button.bottom() >= 10.0,
            "{label}: {button:?}"
        );
        let text = find_text_shape(&h.output().shapes, &label)
            .map(|s| s.galley.rect.translate(s.pos.to_vec2()))
            .unwrap();
        assert!(
            (text.center().y - button.center().y).abs() <= 1.0,
            "{label}"
        );
    }
}

#[test]
fn about_name_and_version_are_centred_on_the_icon() {
    let mut h = harness(configured_cfg());
    h.state_mut().win.set_page(Page::About);
    run(&mut h);
    let icon = h
        .output()
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Mesh(m) => Some(m.calc_bounds()),
            _ => None,
        })
        .filter(|r| r.width() == 48.0)
        .expect("48 pt icon painted");
    let name = h
        .get_by_label(&wizard101_chat_translator::config::app_name())
        .rect();
    let version = format!("{} v{}", tr("about.version"), env!("CARGO_PKG_VERSION"));
    let version = h.get_by_label(&version).rect();
    let column = name.union(version);
    assert!(
        (column.center().y - icon.center().y).abs() <= 1.0,
        "text {column:?} vs icon {icon:?}"
    );
}

#[test]
fn nothing_but_the_custom_knob_is_painted_on_the_opacity_slider() {
    let mut h = harness(configured_cfg());
    h.state_mut().win.set_page(Page::Advanced);
    run(&mut h);
    let slider = h.get_by_role(Role::Slider).rect();
    for clipped in &h.output().shapes {
        match &clipped.shape {
            // 軌道與填色高 4；egui 預設的方形把手比軌道高
            egui::Shape::Rect(r) if slider.expand(1.0).contains_rect(r.rect) => {
                assert!(r.rect.height() <= 4.5, "extra handle {r:?}");
            }
            egui::Shape::Circle(c) if slider.contains(c.center) => {
                assert_ne!(c.stroke.color, theme::TEXT, "egui's own handle {c:?}");
            }
            _ => {}
        }
    }
}

/// 關於頁頭卡的名稱與版本兩行的位置。
fn hero_lines(h: &Harness<State>) -> (Rect, Rect) {
    let name = h
        .get_by_label(&wizard101_chat_translator::config::app_name())
        .rect();
    let version = format!("{} v{}", tr("about.version"), env!("CARGO_PKG_VERSION"));
    (name, h.get_by_label(&version).rect())
}

#[test]
fn an_update_result_does_not_move_the_name_and_version() {
    let results: [Result<Option<Release>, String>; 3] = [
        Ok(None),
        Err("HTTP 403: API rate limit exceeded for 203.0.113.7".into()),
        Ok(Some(release())),
    ];
    let texts = [
        tr("update.latest"),
        t(
            "update.failed",
            &[("error", "HTTP 403: API rate limit exceeded for 203.0.113.7")],
        ),
        t("update.available", &[("version", "9.9.9")]),
    ];
    for (result, text) in results.into_iter().zip(texts) {
        let mut h = harness(configured_cfg());
        go(&mut h, "settings.tab.about");
        let before = hero_lines(&h);
        check_update(&mut h, result);
        assert_eq!(hero_lines(&h), before, "{text}");
        let line = h.get_by_label(&text).rect();
        assert!(line.top() > before.1.bottom(), "{text}: below the header");
        assert!(line.left() >= before.0.left() - 1.0, "{text}: {line:?}");
    }
}

#[test]
fn the_log_folder_path_is_shown_in_full_beside_the_button() {
    let mut h = harness_with(
        configured_cfg(),
        FakeEnv::default(),
        egui::vec2(640.0, 360.0),
    );
    go(&mut h, "settings.tab.about");
    h.get_by_role_and_label(Role::Button, &tr("button.open_folder"))
        .scroll_to_me();
    run(&mut h);
    let path = app_dir().display().to_string();
    let (galley, pos) = find_text_shape(&h.output().shapes, &path)
        .map(|s| (s.galley.clone(), s.pos))
        .expect("the whole path is painted");
    let button = h
        .get_by_role_and_label(Role::Button, &tr("button.open_folder"))
        .rect();
    assert!(
        pos.x + galley.rect.right() < button.left(),
        "the path stays left of the button"
    );
}

#[test]
fn the_target_language_colors_what_was_typed() {
    let mut h = harness(configured_cfg());
    h.get_by_role_and_label(Role::TextInput, &tr("settings.target_language"))
        .click();
    run(&mut h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.event(egui::Event::Text("eng".into()));
    run(&mut h);
    let english = text_sections(&h, "English");
    assert!(
        english.contains(&("Eng".to_owned(), theme::ACCENT)),
        "{english:?}"
    );
}
