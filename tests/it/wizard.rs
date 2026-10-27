//! 首次設定精靈的內容（移植自 Python 的 test_wizard）。
//!
//! 換介面語言的測試在 `wizard_language.rs`；本檔一律用同一個介面語言。

use crate::common;

use eframe::egui::Key;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use common::kittest::{click, click_change, has_label, run, tr};
use common::wizard::*;
use wizard101_chat_translator::config::services::{ServiceSlot, new_service};
use wizard101_chat_translator::config::{Config, default_config};
use wizard101_chat_translator::i18n::{self, current_language, language_name};
use wizard101_chat_translator::runtime::updater::CROWDIN_URL;
use wizard101_chat_translator::ui::settings::env::SettingsEvent;
use wizard101_chat_translator::ui::settings::theme;
use wizard101_chat_translator::ui::settings::wizard::{Step, WizardAction, can_advance};
use wizard101_chat_translator::ui::text_select::opened_urls;

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
fn wizard_nav_is_packed_before_the_body() {
    // 內容比視窗高的第二步，導覽鈕仍整顆停在視窗底部，內容區才是捲動的那塊
    let mut h = on_service_step(blank_cfg());
    pick_provider(&mut h, "provider.custom");
    let window = theme::WIZARD_SIZE;
    let tail = h
        .get_by_role_and_label(Role::Link, &tr("wizard.skip_test"))
        .rect();
    let nav_top = h
        .get_by_role_and_label(Role::Button, &tr("button.next"))
        .rect()
        .top();
    assert!(tail.bottom() > nav_top, "body should overflow: {tail:?}");
    for key in ["button.back", "button.next"] {
        let button = h.get_by_role_and_label(Role::Button, &tr(key)).rect();
        assert!(button.bottom() <= window.y, "{key}: {button:?}");
        assert!(button.top() >= 0.0, "{key}: {button:?}");
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
    assert_eq!(opened_urls(&h.output().platform_output), [CROWDIN_URL]);
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
fn a_bad_base_url_keeps_next_disabled_even_after_skipping() {
    let mut h = on_service_step(blank_cfg());
    pick_provider(&mut h, "provider.custom");
    type_into(&mut h, &tr("field.base_url"), "http://127.0.0.1:8000/v1");
    type_into(&mut h, &tr("field.model"), "gemma");
    assert!(!has_label(&h, &tr("error.base_url_v1")));
    skip(&mut h);
    assert!(!next_enabled(&h));
    assert!(has_label(&h, &tr("dialog.incomplete_title")));
    assert!(has_label(&h, &tr("error.base_url_v1")));
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
    assert!(!next_enabled(&h));
    // 欄位補齊後仍要擋：先前的略過隨換服務商作廢，不是只因為欄位被清空
    type_into(&mut h, &tr("field.api_key"), "sk-1");
    type_into(&mut h, &tr("field.model"), "gpt-x");
    assert!(!next_enabled(&h));
    skip(&mut h);
    assert!(next_enabled(&h));
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
fn finish_rejects_a_ctrl_v_hotkey_while_paste_is_on() {
    // 手改過的設定也要認得：比對的是正規化後的熱鍵名稱
    let mut cfg = configured();
    cfg.insert("hotkey".into(), json!("v+ctrl"));
    let mut h = to_prefs(cfg);
    click(&mut h, Role::Button, &tr("button.finish"));
    assert!(finished(&h).is_none());
    assert!(has_label(&h, &tr("error.hotkey_is_paste")));
    record(&mut h, 0, "ctrl+q");
    assert!(!has_label(&h, &tr("error.hotkey_is_paste")));
    assert_eq!(walk_to_finish(&mut h)["hotkey"], json!("ctrl+q"));
}

#[test]
fn finish_rejects_an_uppercase_ctrl_v_hotkey() {
    let mut cfg = configured();
    cfg.insert("hotkey".into(), json!("ctrl+V"));
    let mut h = to_prefs(cfg);
    click(&mut h, Role::Button, &tr("button.finish"));
    assert!(finished(&h).is_none());
    assert!(has_label(&h, &tr("error.hotkey_is_paste")));
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

#[test]
fn wizard_service_step_auto_loads() {
    let mut h = on_service_step(blank_cfg());
    pick_provider(&mut h, "provider.claude");
    assert!(h.state().env.lists.borrow().is_empty());
    type_into(&mut h, &tr("field.api_key"), "sk-ant");
    type_into(&mut h, &tr("field.model"), "claude-x");
    assert_eq!(h.state().env.lists.borrow().len(), 1);
}

// --- 本地模型一鍵安裝 ---

mod local_install {
    use super::*;
    use wizard101_chat_translator::local::catalog::RECOMMENDED;
    use wizard101_chat_translator::local::engine::{Asset, Build, EngineInstall, Variant};
    use wizard101_chat_translator::local::fit::{Fit, Need, Pick, Rating};
    use wizard101_chat_translator::local::gguf::ModelMeta;
    use wizard101_chat_translator::local::hardware::{Gpu, GpuKind, Hardware};
    use wizard101_chat_translator::local::hf::{Catalog, Choice, Quant, RepoFile};
    use wizard101_chat_translator::local::store::{Installed, Manifest, model_id};
    use wizard101_chat_translator::local::{Command, EngineState, RepoInfo, RepoState, Snapshot};

    const G: u64 = 1 << 30;
    const QUANT: &str = "Q4_K_M";

    fn repo_info() -> RepoInfo {
        let choice = Choice {
            quant: Quant {
                label: QUANT.into(),
                bits: Some(4),
            },
            files: vec![RepoFile {
                path: "m.gguf".into(),
                size: 2 * G,
                sha256: None,
            }],
        };
        RepoInfo {
            catalog: Catalog {
                choices: vec![choice],
                mtp: vec![],
            },
            meta: ModelMeta::default(),
            fits: vec![vec![Rating {
                need: Need::estimate(3 * G),
                fit: Some(Fit::Smooth),
            }]],
            auto: Some(Pick {
                choice: 0,
                mtp: None,
                fit: Fit::Smooth,
                required: 3 * G,
            }),
        }
    }

    /// 六個推薦都查好、硬體量得到；引擎已就緒。
    fn ready_snapshot() -> Snapshot {
        let mut s = Snapshot {
            engine: EngineState::Ready(EngineInstall {
                build: "b1".into(),
                variant: Variant::Vulkan,
            }),
            hardware: Hardware {
                gpu: Some(Gpu {
                    name: "t".into(),
                    vendor: 0,
                    kind: GpuKind::Discrete,
                    nvidia_driver: None,
                    device: 0,
                }),
                vram_total: 24 * G,
                vram_free: 24 * G,
                ram_total: 32 * G,
                ram_free: 32 * G,
                measured: true,
                own: Default::default(),
            },
            ..Default::default()
        };
        for r in RECOMMENDED {
            s.repos.insert(r.repo.into(), RepoState::Ready(repo_info()));
        }
        s
    }

    fn set_snapshot(h: &mut WizardHarness, s: Snapshot) {
        *h.state().env.snapshot.borrow_mut() = s;
        run(h);
    }

    fn on_card(s: Snapshot) -> WizardHarness {
        let mut h = on_service_step(blank_cfg());
        *h.state().env.snapshot.borrow_mut() = s;
        pick_provider(&mut h, "provider.local");
        h
    }

    fn default_id() -> String {
        model_id("google/gemma-4-12B-it-qat-q4_0-gguf", QUANT)
    }

    fn done_model() -> Installed {
        Installed {
            id: default_id(),
            manifest: Manifest {
                repo: "google/gemma-4-12B-it-qat-q4_0-gguf".into(),
                quant: QUANT.into(),
                files: vec![],
                mtp: None,
                moe: false,
                has_builtin_mtp: false,
                required: 0,
                measured: None,
                layers: 0,
            },
            complete: true,
        }
    }

    fn sent(h: &WizardHarness) -> Vec<String> {
        let commands = h.state().env.commands.borrow();
        commands.iter().map(|c| format!("{c:?}")).collect()
    }

    #[test]
    fn picking_local_shows_the_confirmation_card_with_sizes() {
        let mut s = ready_snapshot();
        s.engine = EngineState::Missing;
        s.builds = Some(Ok(vec![Build {
            tag: "b1".into(),
            published: String::new(),
            variants: vec![(
                Variant::Vulkan,
                vec![Asset {
                    name: "a.zip".into(),
                    url: String::new(),
                    size: G / 2,
                }],
            )],
        }]));
        let h = on_card(s);
        assert!(has_label(&h, "Gemma 4 12B QAT · Q4_K_M"));
        assert!(has_label(&h, "Model: 2.0 GB"));
        assert!(has_label(&h, "Engine (vulkan): 0.5 GB"));
        assert!(has_label(&h, "Total download: 2.5 GB"));
        let sent = sent(&h);
        assert!(sent.iter().any(|c| c.contains("InstallEngine")));
        assert!(!enabled(&h, &tr("local.wizard.download")));
    }

    #[test]
    fn next_is_disabled_until_the_download_completes() {
        let mut h = on_card(ready_snapshot());
        assert!(enabled(&h, &tr("local.wizard.download")));
        click(&mut h, Role::Button, &tr("local.wizard.download"));
        let sent = sent(&h);
        assert!(
            sent.iter()
                .any(|c| c.starts_with("Download") && c.contains("gemma-4-12B"))
        );
        assert!(!next_enabled(&h));

        let mut s = ready_snapshot();
        s.models.push(done_model());
        set_snapshot(&mut h, s);
        assert!(next_enabled(&h));
        next(&mut h);
        click(&mut h, Role::Button, &tr("button.finish"));
        let cfg = finished(&h).expect("wizard finished");
        assert_eq!(cfg["services"][0]["model"], json!(default_id()));
    }

    #[test]
    fn change_model_opens_the_shared_dialog_in_select_mode() {
        let mut h = on_card(ready_snapshot());
        click(&mut h, Role::Button, &tr("local.wizard.change"));
        assert!(
            h.query_all_by_role_and_label(Role::Button, &tr("local.dialog.select"))
                .next()
                .is_some()
        );
    }

    #[test]
    fn cancel_download_keeps_progress_for_later() {
        let mut h = on_card(ready_snapshot());
        click(&mut h, Role::Button, &tr("local.wizard.download"));
        click(&mut h, Role::Button, &tr("local.wizard.cancel"));
        let want = format!("{:?}", Command::CancelDownload(default_id()));
        assert!(sent(&h).contains(&want));
        assert!(has_label(&h, &tr("local.wizard.cancelled_hint")));
        assert!(enabled(&h, &tr("local.wizard.download")));
    }

    #[test]
    fn a_ready_engine_allows_download_even_when_vram_is_unmeasured() {
        let mut s = ready_snapshot();
        for repo in s.repos.values_mut() {
            if let RepoState::Ready(info) = repo {
                info.fits[0][0].fit = None;
            }
        }
        let h = on_card(s);
        assert!(enabled(&h, &tr("local.wizard.download")));
        assert!(has_label(&h, &tr("local.fit.unknown")));
    }

    fn download_button_shown(h: &WizardHarness) -> bool {
        h.query_by_role_and_label(Role::Button, &tr("local.wizard.download"))
            .is_some()
    }

    /// 硬體偵測中、或還有推薦沒查完：卡片先畫佔位，不先挑一個之後再換。
    #[test]
    fn the_card_waits_for_the_hardware_and_every_recommendation() {
        let mut s = ready_snapshot();
        s.detecting = true;
        let h = on_card(s);
        assert!(has_label(&h, &tr("local.loading")));
        assert!(!download_button_shown(&h));

        let mut s = ready_snapshot();
        s.repos
            .insert(RECOMMENDED[0].repo.into(), RepoState::Loading);
        let mut h = on_card(s);
        assert!(!download_button_shown(&h));
        set_snapshot(&mut h, ready_snapshot());
        assert!(download_button_shown(&h));
        assert!(!has_label(&h, &tr("local.loading")));

        // 引擎還在裝、VRAM 還沒量：推薦之後會依量到的換，先畫佔位
        let mut s = ready_snapshot();
        s.engine = EngineState::Missing;
        s.builds = Some(Ok(vec![]));
        s.hardware.measured = false;
        let h = on_card(s);
        assert!(has_label(&h, &tr("local.loading")));
        assert!(has_label(&h, &tr("local.wizard.waiting_vram")));
        assert!(!download_button_shown(&h));
    }

    /// 進來時引擎正在移除：移除完變成未安裝就自動裝一次，不會停在佔位。
    #[test]
    fn an_engine_removed_while_entering_is_installed_once_it_is_missing() {
        let installs = |h: &WizardHarness| {
            sent(h)
                .iter()
                .filter(|c| c.contains("InstallEngine"))
                .count()
        };
        let mut s = ready_snapshot();
        s.engine = EngineState::Removing;
        let mut h = on_card(s.clone());
        assert_eq!(installs(&h), 0);
        s.engine = EngineState::Missing;
        set_snapshot(&mut h, s.clone());
        set_snapshot(&mut h, s);
        assert_eq!(installs(&h), 1);
    }

    /// 推薦全部查詢失敗：顯示原文與〔重試〕，重試會重新查詢失敗的 repo。
    #[test]
    fn failed_recommendations_show_the_error_and_retry() {
        let mut s = ready_snapshot();
        for r in RECOMMENDED {
            s.repos
                .insert(r.repo.into(), RepoState::Failed("HTTP 403".into()));
        }
        let mut h = on_card(s);
        assert!(has_label(&h, "HTTP 403"));
        assert!(!has_label(&h, &tr("local.loading")));
        let before = sent(&h).len();
        click(&mut h, Role::Button, &tr("local.dialog.retry"));
        let queries = sent(&h)[before..]
            .iter()
            .filter(|c| c.starts_with("QueryRepo"))
            .count();
        assert_eq!(queries, RECOMMENDED.iter().filter(|r| r.wizard).count());
    }

    #[test]
    fn a_failed_engine_shows_the_error_and_retry_reinstalls() {
        let mut s = ready_snapshot();
        s.engine = EngineState::Failed("boom".into());
        let mut h = on_card(s);
        assert!(has_label(&h, "boom"));
        assert!(!enabled(&h, &tr("local.wizard.download")));
        click(&mut h, Role::Button, &tr("local.dialog.retry"));
        let installs = sent(&h)
            .iter()
            .filter(|c| c.contains("InstallEngine"))
            .count();
        assert_eq!(installs, 1);
    }

    fn download_count(h: &WizardHarness) -> usize {
        sent(h).iter().filter(|c| c.starts_with("Download")).count()
    }

    #[test]
    fn an_existing_complete_model_skips_the_download() {
        let mut service = new_service("local", &[]);
        service["model"] = json!(default_id());
        let cfg = cfg_with(vec![service.clone()], Some(&service));
        let mut s = ready_snapshot();
        s.models.push(done_model());
        let mut h = open(cfg);
        *h.state().env.snapshot.borrow_mut() = s;
        next(&mut h);
        assert!(next_enabled(&h));
        assert_eq!(download_count(&h), 0);
        next(&mut h);
        click(&mut h, Role::Button, &tr("button.finish"));
        let cfg = finished(&h).expect("wizard finished");
        assert_eq!(cfg["services"][0]["model"], json!(default_id()));
    }

    #[test]
    fn going_back_then_choosing_a_cloud_provider_leaves_nothing_local() {
        let mut h = on_card(ready_snapshot());
        back(&mut h);
        pick_provider(&mut h, "provider.claude");
        type_into(&mut h, &tr("field.api_key"), "sk-ant");
        type_into(&mut h, &tr("field.model"), "claude-x");
        let cfg = walk_to_finish(&mut h);
        let services = cfg["services"].as_array().unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0]["provider"], json!("claude"));
    }

    #[test]
    fn re_entering_does_not_repeat_entry_commands_or_the_download() {
        let mut s = ready_snapshot();
        s.repos.remove(RECOMMENDED[5].repo);
        let mut h = on_card(s);
        click(&mut h, Role::Button, &tr("local.wizard.download"));
        let mut s = ready_snapshot();
        s.downloads.insert(
            default_id(),
            wizard101_chat_translator::local::DownloadState::Running {
                done: G,
                total: 2 * G,
            },
        );
        set_snapshot(&mut h, s);
        back(&mut h);
        pick_provider(&mut h, "provider.local");
        assert!(has(&h, Role::Button, &tr("local.wizard.cancel")));
        let queries = sent(&h).iter().filter(|c| c.contains("QueryRepo")).count();
        assert_eq!(queries, 1);
        assert_eq!(download_count(&h), 1);
    }

    /// hub 收到下載就把舊的失敗蓋成下載中；之後馬上又失敗也立刻顯示。
    #[test]
    fn a_new_failure_right_after_download_is_shown() {
        use wizard101_chat_translator::local::DownloadState;
        let failed = |error: &str| {
            let mut s = ready_snapshot();
            s.downloads
                .insert(default_id(), DownloadState::Failed(error.into()));
            s
        };
        let mut h = on_card(failed("old"));
        click(&mut h, Role::Button, &tr("local.wizard.download"));
        let mut s = ready_snapshot();
        s.downloads
            .insert(default_id(), DownloadState::Running { done: 0, total: 0 });
        set_snapshot(&mut h, s);
        assert!(!has_label(&h, "old"));
        assert!(has(&h, Role::Button, &tr("local.wizard.cancel")));
        set_snapshot(&mut h, failed("new"));
        assert!(has_label(&h, "new"));
    }

    /// 表單原有的模型沒下載完：直接顯示〔繼續下載〕，按了送繼續下載。
    #[test]
    fn an_incomplete_existing_model_offers_to_continue() {
        let mut service = new_service("local", &[]);
        service["model"] = json!(default_id());
        let cfg = cfg_with(vec![service.clone()], Some(&service));
        let mut s = ready_snapshot();
        let mut model = done_model();
        model.complete = false;
        s.models.push(model);
        let mut h = open(cfg);
        *h.state().env.snapshot.borrow_mut() = s;
        next(&mut h);
        assert!(!next_enabled(&h));
        click(&mut h, Role::Button, &tr("local.continue_download"));
        let want = format!("{:?}", Command::Resume(default_id()));
        assert!(sent(&h).contains(&want), "{:?}", sent(&h));
        assert_eq!(download_count(&h), 0);
    }

    /// 取消後返回再進來：取消提示不留，暫停的下載重新接上並可繼續。
    #[test]
    fn re_entering_after_cancel_resumes_the_paused_download() {
        use wizard101_chat_translator::local::DownloadState;
        let mut h = on_card(ready_snapshot());
        click(&mut h, Role::Button, &tr("local.wizard.download"));
        click(&mut h, Role::Button, &tr("local.wizard.cancel"));
        let mut s = ready_snapshot();
        let mut model = done_model();
        model.complete = false;
        s.models.push(model);
        s.downloads.insert(
            default_id(),
            DownloadState::Paused {
                done: 1,
                total: 2 * G,
            },
        );
        set_snapshot(&mut h, s);
        back(&mut h);
        pick_provider(&mut h, "provider.local");
        assert!(!has_label(&h, &tr("local.wizard.cancelled_hint")));
        click(&mut h, Role::Button, &tr("local.continue_download"));
        let want = format!("{:?}", Command::Resume(default_id()));
        assert!(sent(&h).contains(&want), "{:?}", sent(&h));
    }

    #[test]
    fn a_custom_repo_selection_is_tracked_with_its_own_id() {
        let mut s = ready_snapshot();
        s.repos
            .insert("org/custom".into(), RepoState::Ready(repo_info()));
        let mut h = on_card(s);
        click(&mut h, Role::Button, &tr("local.wizard.change"));
        click(&mut h, Role::Button, &tr("local.dialog.tab_custom"));
        type_into(&mut h, &tr("local.dialog.search_hint"), "org/custom");
        click(&mut h, Role::Button, &tr("local.dialog.search"));
        click(&mut h, Role::Button, &tr("local.dialog.select"));
        click(&mut h, Role::Button, &tr("local.wizard.download"));
        let want = model_id("org/custom", QUANT);
        let sent = sent(&h);
        assert!(
            sent.iter()
                .any(|c| c.starts_with("Download") && c.contains("org/custom")),
            "{sent:?}"
        );
        assert!(!next_enabled(&h));
        let mut s = ready_snapshot();
        let mut m = done_model();
        m.id = want;
        s.models.push(m);
        set_snapshot(&mut h, s);
        assert!(next_enabled(&h));
    }
}
