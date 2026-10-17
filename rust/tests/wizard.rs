//! 首次設定精靈的內容（移植自 Python 的 test_wizard）。
//!
//! 換介面語言的測試在 `wizard_language.rs`；本檔一律用同一個介面語言。

mod common;

use eframe::egui::accesskit::Role;
use eframe::egui::{Key, OutputCommand};
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use common::wizard::*;
use wizard101_chat_translator::config::services::{ServiceSlot, new_service};
use wizard101_chat_translator::config::{Config, default_config};
use wizard101_chat_translator::i18n::{self, current_language, language_name};
use wizard101_chat_translator::runtime::updater::CROWDIN_URL;
use wizard101_chat_translator::ui::settings::env::SettingsEvent;
use wizard101_chat_translator::ui::settings::theme;
use wizard101_chat_translator::ui::settings::wizard::{Step, WizardAction, can_advance};

const LANGUAGE: &str = "en-US";

fn blank_cfg() -> Config {
    i18n::set_language(LANGUAGE);
    default_config()
}

fn open(cfg: Config) -> WizardHarness {
    i18n::set_language(LANGUAGE);
    harness(cfg)
}

/// 停在第二步的精靈。
fn on_service_step(cfg: Config) -> WizardHarness {
    let mut h = open(cfg);
    next(&mut h);
    assert_eq!(step(&h), Step::Service);
    h
}

/// 停在第二步、Claude 的欄位都填好的精靈。
fn with_a_filled_form() -> WizardHarness {
    let mut h = on_service_step(blank_cfg());
    pick_provider(&mut h, "provider.claude");
    type_into(&mut h, &tr("field.api_key"), "sk-ant");
    type_into(&mut h, &tr("field.model"), "claude-x");
    h
}

fn to_prefs(cfg: Config) -> WizardHarness {
    let mut h = on_service_step(cfg);
    skip(&mut h);
    next(&mut h);
    assert_eq!(step(&h), Step::Prefs);
    h
}

fn configured() -> Config {
    i18n::set_language(LANGUAGE);
    configured_cfg()
}

/// 三組填好的服務（openai／claude／custom）。
fn three_services() -> Vec<Value> {
    i18n::set_language(LANGUAGE);
    let mut services: Vec<Value> = Vec::new();
    for provider in ["openai", "claude", "custom"] {
        let service = filled(provider, &services);
        services.push(service);
    }
    services
}

fn opened_urls(h: &WizardHarness) -> Vec<String> {
    h.output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            OutputCommand::OpenUrl(open) => Some(open.url.clone()),
            _ => None,
        })
        .collect()
}

fn help_link() -> String {
    format!("{} ↗", tr("credit.help_translate"))
}

// --- can_advance ---

#[test]
fn step_service_requires_valid_form() {
    assert!(!can_advance(
        Step::Service,
        true,
        &["error.need_api_key".into()]
    ));
}

#[test]
fn step_service_requires_passed_or_skipped_test() {
    assert!(!can_advance(Step::Service, false, &[]));
    assert!(can_advance(Step::Service, true, &[]));
}

#[test]
fn step_prefs_always_advances() {
    assert!(can_advance(Step::Prefs, false, &[]));
}

#[test]
fn can_advance_allows_language_step() {
    // 語言頁沒有必填欄位，永遠可以往下一步
    assert!(can_advance(
        Step::Language,
        false,
        &["error.need_model".into()]
    ));
}

// --- 版面與導覽 ---

#[test]
fn the_step_bar_names_all_three_steps() {
    let h = open(blank_cfg());
    for key in [
        "wizard.step.language",
        "wizard.step.service",
        "wizard.step.prefs",
    ] {
        assert!(has_label(&h, &tr(key)), "{key}");
    }
}

#[test]
fn the_step_bar_fits_the_wizard_width() {
    // 英文的步驟名稱最長：三步加連接線仍要擠得進預設寬度
    let h = open(blank_cfg());
    let label = tr("wizard.step.prefs");
    let names: Vec<_> = h.query_all_by_label(&label).collect();
    assert_eq!(names.len(), 1);
    assert!(names[0].rect().right() <= theme::WIZARD_SIZE.x - 30.0);
}

#[test]
fn back_is_disabled_on_the_first_step_only() {
    let mut h = open(blank_cfg());
    assert!(!enabled(&h, &tr("button.back")));
    next(&mut h);
    assert!(enabled(&h, &tr("button.back")));
    back(&mut h);
    assert_eq!(step(&h), Step::Language);
}

#[test]
fn the_language_step_shows_the_app_and_the_language_combo() {
    let h = open(blank_cfg());
    assert!(has_label(&h, &tr("app.name")));
    assert!(has_label(&h, &tr("wizard.language_hint")));
    h.get_by_role_and_label(Role::ComboBox, &tr("field.ui_language"));
}

#[test]
fn language_step_shows_the_translators() {
    // en-US 有掛名的譯者；掛名與協助翻譯連結都在語言下拉底下
    let h = open(blank_cfg());
    h.get_by_role_and_label(Role::Link, "Claude Code");
    let combo = h
        .get_by_role_and_label(Role::ComboBox, &tr("field.ui_language"))
        .rect();
    let link = h.get_by_role_and_label(Role::Link, &help_link()).rect();
    assert!(link.top() >= combo.bottom());
}

#[test]
fn language_step_links_to_crowdin() {
    let mut h = open(blank_cfg());
    h.get_by_role_and_label(Role::Link, &help_link()).click();
    h.step();
    assert_eq!(opened_urls(&h), [CROWDIN_URL]);
}

#[test]
fn the_credits_belong_to_the_language_step_only() {
    let h = to_prefs(configured());
    assert!(!has(&h, Role::Link, &help_link()));
    assert!(!has(&h, Role::Link, "Claude Code"));
}

#[test]
fn the_prefs_step_shows_target_language_and_both_hotkeys() {
    let h = to_prefs(configured());
    assert!(has_label(&h, &tr("wizard.target_language")));
    assert!(has_label(&h, &tr("hint.language")));
    assert!(has_label(&h, &tr("settings.hotkey")));
    assert!(has_label(&h, &tr("settings.region_hotkey")));
    h.get_by_role_and_label(Role::Switch, &tr("field.auto_input"));
    assert!(has(&h, Role::Button, &tr("button.finish")));
    assert!(!has(&h, Role::Button, &tr("button.next")));
}

// --- 第二步 ---

#[test]
fn step_service_asks_for_a_provider_before_the_form() {
    // 沒有服務可編輯時第二步先選服務商：表單還不存在，〔下一步〕也就無從驗起
    let h = on_service_step(blank_cfg());
    for key in [
        "provider.openai",
        "provider.claude",
        "provider.gemini",
        "provider.custom",
    ] {
        assert!(has(&h, Role::Button, &tr(key)), "{key}");
    }
    assert!(!has(&h, Role::TextInput, &tr("field.service_name")));
    assert!(!next_enabled(&h));
}

#[test]
fn picking_a_provider_replaces_the_cards_with_the_form() {
    let mut h = on_service_step(blank_cfg());
    pick_provider(&mut h, "provider.claude");
    assert!(!has(&h, Role::Button, &tr("provider.openai")));
    assert_eq!(
        field_value(&h, &tr("field.service_name")).as_deref(),
        Some("Claude")
    );
}

#[test]
fn an_existing_service_skips_the_cards() {
    // 救援路徑：設定壞掉的老使用者被送回精靈時，服務商早就選過了，直接編輯那一筆
    let services = three_services();
    let cfg = cfg_with(services.clone(), Some(&services[1]));
    let h = on_service_step(cfg);
    assert!(!has(&h, Role::Button, &tr("provider.openai")));
    assert_eq!(
        field_value(&h, &tr("field.service_name")),
        services[1]["name"].as_str().map(str::to_owned)
    );
}

#[test]
fn the_form_instructions_wait_until_the_form_exists() {
    // 前半只有服務商卡片：叫人填設定、按測試連線是在講畫面上還沒有的東西
    let h = on_service_step(blank_cfg());
    assert!(has_label(&h, &tr("wizard.intro")));
    assert!(has_label(&h, &tr("wizard.pick_provider")));
    assert!(!has_label(&h, &tr("wizard.intro_form")));
}

#[test]
fn the_form_step_explains_what_to_do_with_the_form() {
    let mut h = on_service_step(blank_cfg());
    pick_provider(&mut h, "provider.claude");
    assert!(has_label(&h, &tr("wizard.intro")));
    assert!(has_label(&h, &tr("wizard.intro_form")));
    assert!(!has_label(&h, &tr("wizard.pick_provider")));
}

#[test]
fn next_is_disabled_until_tested_or_skipped() {
    let mut h = with_a_filled_form();
    assert!(!next_enabled(&h));
    pass_test(&mut h);
    assert!(next_enabled(&h));

    let mut h = with_a_filled_form();
    skip(&mut h);
    assert!(next_enabled(&h));
    next(&mut h);
    assert_eq!(step(&h), Step::Prefs);
}

#[test]
fn the_test_translates_into_the_interface_language() {
    let mut h = with_a_filled_form();
    pass_test(&mut h);
    let target = h.state().env.tests.borrow().last().unwrap().2.clone();
    assert_eq!(target, language_name(&current_language()));
}

#[test]
fn clearing_a_field_after_skipping_the_test_blocks_the_next_button() {
    // 略過測試後把模型清空，〔下一步〕必須跟著變灰 —— 否則會帶著 model="" 的服務走完
    let mut h = with_a_filled_form();
    skip(&mut h);
    assert!(next_enabled(&h));
    replace_text(&mut h, &tr("field.model"), "");
    assert!(!next_enabled(&h));
}

#[test]
fn editing_a_field_cancels_an_earlier_skip() {
    // 略過測試後又改了金鑰：先前的放行不算數，要重測或再次明示略過
    let mut h = with_a_filled_form();
    skip(&mut h);
    type_into(&mut h, &tr("field.api_key"), "-other");
    assert!(!next_enabled(&h));
    skip(&mut h);
    assert!(next_enabled(&h));
}

#[test]
fn completing_the_fields_after_an_early_skip_recomputes_the_nav() {
    // 欄位不全時就點過「略過測試」：補上模型時先前的略過作廢，再次明示略過才走得下去
    let mut h = on_service_step(blank_cfg());
    pick_provider(&mut h, "provider.claude");
    type_into(&mut h, &tr("field.api_key"), "sk-ant");
    skip(&mut h);
    assert!(!next_enabled(&h));
    type_into(&mut h, &tr("field.model"), "claude-x");
    assert!(!next_enabled(&h));
    skip(&mut h);
    assert!(next_enabled(&h));
}

#[test]
fn the_skip_test_link_is_keyboard_activatable() {
    // 純鍵盤使用者測不成連線時，「略過測試」是第二步唯一的出路
    for key in [Key::Enter, Key::Space] {
        let mut h = with_a_filled_form();
        h.get_by_role_and_label(Role::Link, &tr("wizard.skip_test"))
            .focus();
        run(&mut h);
        assert!(
            h.get_by_role_and_label(Role::Link, &tr("wizard.skip_test"))
                .is_focused()
        );
        h.key_press(key);
        run(&mut h);
        assert!(next_enabled(&h), "{key:?}");
    }
}

#[test]
fn switching_provider_in_the_wizard_dedupes_against_cfg_services() {
    // 精靈把設定裡的服務清單交給表單：編輯中的服務切到 Claude，要跟另一筆已叫 Claude 的去重
    i18n::set_language(LANGUAGE);
    let existing_claude = new_service("claude", &[]);
    let openai = new_service("openai", std::slice::from_ref(&existing_claude));
    let cfg = cfg_with(vec![existing_claude, openai.clone()], Some(&openai));
    let mut h = on_service_step(cfg);
    click(&mut h, Role::Button, &tr("button.change"));
    let dialog = tr("service.pick_provider");
    h.get_by_role_and_label(Role::Dialog, &dialog)
        .get_by_role_and_label(Role::Button, &tr("provider.claude"))
        .click();
    run(&mut h);
    assert!(!has(&h, Role::Dialog, &dialog));
    assert_eq!(
        field_value(&h, &tr("field.service_name")).as_deref(),
        Some("Claude (2)")
    );
}

#[test]
fn changing_the_provider_cancels_an_earlier_skip() {
    let mut h = with_a_filled_form();
    skip(&mut h);
    click(&mut h, Role::Button, &tr("button.change"));
    h.get_by_role_and_label(Role::Dialog, &tr("service.pick_provider"))
        .get_by_role_and_label(Role::Button, &tr("provider.openai"))
        .click();
    run(&mut h);
    type_into(&mut h, &tr("field.api_key"), "sk-1");
    type_into(&mut h, &tr("field.model"), "gpt-x");
    assert!(!next_enabled(&h));
}

// --- 完成 ---

#[test]
fn finishing_the_wizard_keeps_the_other_services() {
    // 精靈也是設定壞掉時的救援路徑 —— 按完成不能把其他服務與它們的金鑰刪掉
    let services = three_services();
    let ids: Vec<Value> = services.iter().map(|s| s["id"].clone()).collect();
    let mut cfg = cfg_with(services, None);
    cfg["default_service"] = ids[1].clone();
    cfg["service_slots"][ServiceSlot::Region.key()] = ids[2].clone();
    let mut h = on_service_step(cfg);
    replace_text(&mut h, &tr("field.model"), "claude-x");
    let cfg = walk_to_finish(&mut h);
    let services = cfg["services"].as_array().unwrap();
    let got: Vec<Value> = services.iter().map(|s| s["id"].clone()).collect();
    assert_eq!(got, ids);
    assert_eq!(services[0]["api_key"], json!("sk-openai"));
    assert_eq!(services[2]["base_url"], json!("http://x"));
    assert_eq!(services[1]["model"], json!("claude-x"));
    assert_eq!(cfg["default_service"], ids[1]);
    assert_eq!(cfg["service_slots"][ServiceSlot::Region.key()], ids[2]);
}

#[test]
fn finish_keeps_other_services_and_their_keys() {
    // 預設服務指到不存在的 id：編輯清單第一筆，完成後它成為預設，其他服務與金鑰原封不動
    let services = three_services();
    let mut cfg = cfg_with(services.clone(), None);
    cfg["default_service"] = json!("gone");
    let mut h = on_service_step(cfg);
    assert_eq!(
        field_value(&h, &tr("field.service_name")),
        services[0]["name"].as_str().map(str::to_owned)
    );
    let cfg = walk_to_finish(&mut h);
    let saved = cfg["services"].as_array().unwrap();
    assert_eq!(saved[1..], services[1..]);
    assert_eq!(saved[0]["id"], services[0]["id"]);
    assert_eq!(saved[0]["api_key"], json!("sk-openai"));
    assert_eq!(cfg["default_service"], services[0]["id"]);
}

#[test]
fn finish_dedupes_a_name_typed_to_match_another_service() {
    // 把名稱打成跟另一筆一樣：完成時補序號，清單不可留下兩筆同名（分派下拉靠名稱認服務）
    i18n::set_language(LANGUAGE);
    let mut other = filled("openai", &[]);
    other["name"] = json!("X");
    let edited = filled("claude", std::slice::from_ref(&other));
    let cfg = cfg_with(vec![other, edited.clone()], Some(&edited));
    let mut h = on_service_step(cfg);
    replace_text(&mut h, &tr("field.service_name"), "X");
    let cfg = walk_to_finish(&mut h);
    assert_eq!(names(&cfg), ["X", "X (2)"]);
}

#[test]
fn finish_writes_the_service_into_an_empty_list() {
    let mut h = on_service_step(blank_cfg());
    pick_provider(&mut h, "provider.claude");
    type_into(&mut h, &tr("field.api_key"), "sk-ant");
    type_into(&mut h, &tr("field.model"), "claude-x");
    let cfg = walk_to_finish(&mut h);
    let services = cfg["services"].as_array().unwrap();
    assert_eq!(services.len(), 1);
    assert_eq!(services[0]["provider"], json!("claude"));
    assert_eq!(services[0]["api_key"], json!("sk-ant"));
    assert_eq!(cfg["default_service"], services[0]["id"]);
    assert_eq!(cfg["ui_language"], json!(LANGUAGE));
    for slot in ServiceSlot::ALL {
        assert_eq!(cfg["service_slots"][slot.key()], Value::Null);
    }
}

#[test]
fn finish_stores_the_preferences() {
    let mut h = to_prefs(configured());
    replace_text(&mut h, &tr("wizard.target_language"), "日本語");
    // 收起清單，免得它蓋住底下的開關
    h.key_press(Key::Escape);
    run(&mut h);
    click(&mut h, Role::Switch, &tr("field.auto_input"));
    record(&mut h, 1, "ctrl+alt+r");
    let cfg = walk_to_finish(&mut h);
    assert_eq!(cfg["target_language"], json!("日本語"));
    assert_eq!(cfg["auto_show_input"], json!(false));
    assert_eq!(cfg["region_hotkey"], json!("ctrl+alt+r"));
    assert_eq!(cfg["hotkey"], json!("ctrl+space"));
}

#[test]
fn finish_rejects_identical_hotkeys() {
    let mut h = to_prefs(configured());
    record(&mut h, 1, "ctrl+space");
    click(&mut h, Role::Button, &tr("button.finish"));
    assert!(finished(&h).is_none());
    assert!(has_label(&h, &tr("dialog.incomplete_title")));
    assert!(has_label(&h, &tr("error.hotkeys_same")));
    // 改掉之後提示消失，就能完成
    record(&mut h, 1, "ctrl+alt+r");
    assert!(!has_label(&h, &tr("error.hotkeys_same")));
    let cfg = walk_to_finish(&mut h);
    assert_eq!(cfg["region_hotkey"], json!("ctrl+alt+r"));
}

#[test]
fn finish_sends_the_full_config() {
    // 精靈在 App 啟動前跑：送出的是整份設定，不是只有精靈管的那幾個鍵
    let mut cfg = configured();
    cfg.insert("overlay_alpha".into(), json!(0.5));
    cfg.insert("unknown_key".into(), json!("kept"));
    let mut h = open(cfg);
    let cfg = walk_to_finish(&mut h);
    assert_eq!(cfg["overlay_alpha"], json!(0.5));
    assert_eq!(cfg["unknown_key"], json!("kept"));
}

// --- 錄熱鍵與取消 ---

#[test]
fn cancel_stops_an_active_recording() {
    let mut h = to_prefs(configured());
    click_change(&mut h, 0);
    let State { wizard, env, .. } = h.state_mut();
    assert_eq!(wizard.cancel(env), WizardAction::Cancel);
    assert_eq!(env.cancels.get(), 1);
}

#[test]
fn cancel_without_recording_releases_nothing() {
    let mut h = open(blank_cfg());
    let State { wizard, env, .. } = h.state_mut();
    assert_eq!(wizard.cancel(env), WizardAction::Cancel);
    assert_eq!(env.cancels.get(), 0);
}

#[test]
fn leaving_the_prefs_step_stops_an_active_recording() {
    let mut h = to_prefs(configured());
    click_change(&mut h, 1);
    let round = *h.state().env.recordings.borrow().last().unwrap();
    back(&mut h);
    assert_eq!(h.state().env.cancels.get(), 1);
    // 之後才回來的結果丟掉
    h.state_mut().wizard.apply(&SettingsEvent::HotkeyRecorded {
        round,
        combo: "ctrl+alt+q".into(),
    });
    let cfg = walk_to_finish(&mut h);
    assert_eq!(cfg["region_hotkey"], json!("ctrl+shift+space"));
}

#[test]
fn a_recorded_hotkey_releases_the_recorder() {
    let mut h = to_prefs(configured());
    record(&mut h, 0, "ctrl+q");
    assert_eq!(h.state().env.cancels.get(), 1);
    assert!(h.query_all_by_label("Q").next().is_some());
}
