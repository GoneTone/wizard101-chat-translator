//! 首次設定精靈換介面語言：同一個精靈以新語言繼續，狀態照 Python 重建精靈後的結果。
//!
//! 會切換全域介面語言，獨立成一個測試執行檔，檔內以鎖排隊。

mod common;

use std::sync::{Mutex, MutexGuard};

use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use common::log_capture::{capture_logs, log_lines_with};
use common::wizard::*;
use wizard101_chat_translator::config::{Config, bootstrap_language_with, default_config};
use wizard101_chat_translator::i18n::{self, current_language, language_name};
use wizard101_chat_translator::ui::settings::wizard::{Step, WizardAction};

/// 全域介面語言同一時間只給一個測試用。
fn language_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 以 `language` 開精靈；`cfg` 在設好語言之後才建，服務的自動名稱才是這個語言的。
fn open(language: &str, cfg: impl FnOnce() -> Config) -> WizardHarness {
    i18n::set_language(language);
    harness(cfg())
}

/// 從介面語言下拉選 `code`。
fn pick_language(h: &mut WizardHarness, code: &str) {
    click(h, Role::ComboBox, &tr("field.ui_language"));
    click(h, Role::Button, &language_name(code));
}

/// 從第二步退回第一步換語言，再回到第二步。
fn change_language_from_service_step(h: &mut WizardHarness, code: &str) {
    back(h);
    pick_language(h, code);
    next(h);
}

fn service_name(h: &WizardHarness) -> Option<String> {
    field_value(h, &tr("field.service_name"))
}

fn blank() -> Config {
    default_config()
}

#[test]
fn language_change_relabels_the_same_wizard() {
    let _lock = language_lock();
    capture_logs();
    // 語言鎖讓同一時間只有本測試在記這行，前後筆數相減才準
    let logged = || log_lines_with("[ui] wizard restarting with language en-US").len();
    let before = logged();
    let mut h = open("zh-TW", blank);
    pick_language(&mut h, "en-US");
    assert_eq!(current_language(), "en-US");
    assert_eq!(h.state().actions, [WizardAction::PreviewLanguage]);
    assert_eq!(logged(), before + 1);
    assert_eq!(step(&h), Step::Language);
    h.get_by_role_and_label(Role::Button, "Next");
    assert!(has_label(&h, "Create a translation service"));
    let cfg = {
        h.state_mut().actions.clear();
        next(&mut h);
        pick_provider(&mut h, "provider.claude");
        type_into(&mut h, "API key", "sk");
        type_into(&mut h, "Model", "m");
        walk_to_finish(&mut h)
    };
    assert_eq!(cfg["ui_language"], json!("en-US"));
}

#[test]
fn picking_the_current_language_does_nothing() {
    let _lock = language_lock();
    let mut h = open("en-US", blank);
    pick_language(&mut h, "en-US");
    assert!(h.state().actions.is_empty());
}

#[test]
fn language_change_updates_untouched_target_language() {
    let _lock = language_lock();
    let mut h = open("zh-TW", || {
        let mut cfg = configured_cfg();
        cfg["target_language"] = json!("繁體中文（台灣）");
        cfg
    });
    pick_language(&mut h, "en-US");
    let cfg = walk_to_finish(&mut h);
    assert_eq!(cfg["target_language"], json!("English"));
}

#[test]
fn language_change_keeps_customised_target_language() {
    let _lock = language_lock();
    let mut h = open("zh-TW", || {
        let mut cfg = configured_cfg();
        cfg["target_language"] = json!("日本語");
        cfg
    });
    pick_language(&mut h, "en-US");
    let cfg = walk_to_finish(&mut h);
    assert_eq!(cfg["target_language"], json!("日本語"));
}

#[test]
fn a_target_language_typed_before_the_change_is_kept() {
    let _lock = language_lock();
    let mut h = open("zh-TW", configured_cfg);
    next(&mut h);
    skip(&mut h);
    next(&mut h);
    replace_text(&mut h, &tr("wizard.target_language"), "Deutsch");
    back(&mut h);
    back(&mut h);
    pick_language(&mut h, "en-US");
    let cfg = walk_to_finish(&mut h);
    assert_eq!(cfg["target_language"], json!("Deutsch"));
}

#[test]
fn language_change_from_en_bootstrap_follows_to_zh_cn() {
    // 英文系統首次啟動時 target_language 是介面語言的自稱，切到 zh-CN 時才比對得到
    let _lock = language_lock();
    let mut h = open("en-US", || {
        let mut cfg = default_config();
        let language = bootstrap_language_with(&mut cfg, false, || "en-US".into());
        assert_eq!(language, "en-US");
        assert_eq!(cfg["target_language"], json!(language_name("en-US")));
        let service = filled("claude", &[]);
        cfg["services"] = json!([service.clone()]);
        cfg["default_service"] = service["id"].clone();
        cfg
    });
    pick_language(&mut h, "zh-CN");
    let cfg = walk_to_finish(&mut h);
    assert_eq!(cfg["target_language"], json!("简体中文（中国）"));
}

#[test]
fn language_change_keeps_the_half_filled_service() {
    // 在第二步貼好金鑰後退回第一步換語言：表單必須還帶著剛才填的值，不能逼使用者重貼金鑰
    let _lock = language_lock();
    let mut h = open("zh-TW", blank);
    next(&mut h);
    pick_provider(&mut h, "provider.claude");
    type_into(&mut h, &tr("field.api_key"), "sk-ant-secret");
    type_into(&mut h, &tr("field.model"), "claude-x");
    change_language_from_service_step(&mut h, "en-US");
    assert!(!has(&h, Role::Button, "OpenAI"));
    assert_eq!(field_value(&h, "Model").as_deref(), Some("claude-x"));
    let cfg = walk_to_finish(&mut h);
    let service = &cfg["services"][0];
    assert_eq!(service["provider"], json!("claude"));
    assert_eq!(service["model"], json!("claude-x"));
    assert_eq!(service["api_key"], json!("sk-ant-secret"));
}

#[test]
fn language_change_retitles_an_untouched_service() {
    // 自動取的名字就是服務商短名：使用者沒改過就跟著介面語言走
    let _lock = language_lock();
    let mut h = open("zh-TW", blank);
    next(&mut h);
    pick_provider(&mut h, "provider.custom");
    change_language_from_service_step(&mut h, "en-US");
    assert_eq!(service_name(&h).as_deref(), Some("Custom endpoint"));
}

#[test]
fn language_change_retitles_a_suffixed_untouched_service() {
    // 撞名帶序號的自動名稱（「自訂端點 (2)」）仍算沒改過，換語言時照樣跟著換
    let _lock = language_lock();
    let mut h = open("zh-TW", || {
        let bystander = filled("custom", &[]);
        let edited = filled("custom", std::slice::from_ref(&bystander));
        assert_eq!(edited["name"], json!("自訂端點 (2)"));
        cfg_with(vec![bystander, edited.clone()], Some(&edited))
    });
    pick_language(&mut h, "en-US");
    let cfg = walk_to_finish(&mut h);
    assert_eq!(names(&cfg), ["自訂端點", "Custom endpoint"]);
}

#[test]
fn language_change_only_retitles_the_service_being_edited() {
    // 兩筆都停在自動名稱：改名不能靠位置認人，索引 0 那筆與精靈無關、必須原封不動
    let _lock = language_lock();
    let mut h = open("zh-TW", || {
        let bystander = filled("custom", &[]);
        let mut edited = filled("custom", std::slice::from_ref(&bystander));
        edited["name"] = bystander["name"].clone();
        cfg_with(vec![bystander, edited.clone()], Some(&edited))
    });
    pick_language(&mut h, "en-US");
    let cfg = walk_to_finish(&mut h);
    assert_eq!(names(&cfg), ["自訂端點", "Custom endpoint"]);
}

#[test]
fn language_change_keeps_a_service_name_the_user_typed() {
    let _lock = language_lock();
    let mut h = open("zh-TW", blank);
    next(&mut h);
    pick_provider(&mut h, "provider.custom");
    replace_text(&mut h, &tr("field.service_name"), "戰鬥用");
    change_language_from_service_step(&mut h, "en-US");
    assert_eq!(service_name(&h).as_deref(), Some("戰鬥用"));
}

#[test]
fn finish_does_not_collide_a_service_with_itself() {
    // 編輯的服務本來就在清單裡，去重不能拿它跟自己比，否則每次換語言都多疊一次序號
    let _lock = language_lock();
    let mut h = open("zh-TW", || {
        let mut service = filled("claude", &[]);
        service["name"] = json!("X");
        cfg_with(vec![service.clone()], Some(&service))
    });
    pick_language(&mut h, "en-US");
    pick_language(&mut h, "zh-TW");
    let cfg = walk_to_finish(&mut h);
    assert_eq!(names(&cfg), ["X"]);
}

#[test]
fn language_change_does_not_let_the_rename_collide() {
    // 清單裡已經有一筆叫新語言短名的服務：自動改名同樣要去重
    let _lock = language_lock();
    i18n::set_language("en-US");
    let other = filled("custom", &[]);
    let mut h = open("zh-TW", || {
        let edited = filled("custom", &[]);
        cfg_with(vec![edited.clone(), other.clone()], Some(&edited))
    });
    pick_language(&mut h, "en-US");
    let cfg = walk_to_finish(&mut h);
    assert_eq!(names(&cfg), ["Custom endpoint (2)", "Custom endpoint"]);
}

#[test]
fn language_change_while_picking_stays_on_the_cards() {
    // 還在選服務商時換語言：沒有服務可寫，回到第二步仍是卡片
    let _lock = language_lock();
    let mut h = open("zh-TW", blank);
    next(&mut h);
    change_language_from_service_step(&mut h, "en-US");
    assert!(has(&h, Role::Button, "OpenAI"));
    assert!(!has(&h, Role::TextInput, "Name"));
    assert!(!next_enabled(&h));
}

#[test]
fn language_change_after_picking_keeps_the_form() {
    let _lock = language_lock();
    let mut h = open("zh-TW", blank);
    next(&mut h);
    pick_provider(&mut h, "provider.claude");
    change_language_from_service_step(&mut h, "en-US");
    assert!(!has(&h, Role::Button, "OpenAI"));
    assert_eq!(service_name(&h).as_deref(), Some("Claude"));
}

#[test]
fn language_change_voids_the_test_result_and_the_skip() {
    let _lock = language_lock();
    let mut h = open("zh-TW", configured_cfg);
    next(&mut h);
    pass_test(&mut h);
    assert!(next_enabled(&h));
    change_language_from_service_step(&mut h, "en-US");
    assert!(!next_enabled(&h));
    skip(&mut h);
    change_language_from_service_step(&mut h, "zh-TW");
    assert!(!next_enabled(&h));
}

#[test]
fn language_change_keeps_the_hotkeys() {
    let _lock = language_lock();
    let mut h = open("zh-TW", configured_cfg);
    next(&mut h);
    skip(&mut h);
    next(&mut h);
    record(&mut h, 1, "ctrl+alt+r");
    back(&mut h);
    back(&mut h);
    pick_language(&mut h, "en-US");
    let cfg = walk_to_finish(&mut h);
    assert_eq!(cfg["region_hotkey"], json!("ctrl+alt+r"));
}

#[test]
fn translators_follow_the_language() {
    let _lock = language_lock();
    let mut h = open("zh-TW", blank);
    assert!(!has(&h, Role::Link, "Claude Code"));
    // 邀請協助翻譯的連結不看有沒有譯者：沒人翻的語言更需要
    assert!(has(
        &h,
        Role::Link,
        &format!("{} ↗", tr("credit.help_translate"))
    ));
    pick_language(&mut h, "en-US");
    assert!(has(&h, Role::Link, "Claude Code"));
}

#[test]
fn picking_a_provider_is_logged() {
    let _lock = language_lock();
    capture_logs();
    let logged = || log_lines_with("[ui] wizard provider picked: gemini").len();
    let before = logged();
    let mut h = open("en-US", blank);
    next(&mut h);
    pick_provider(&mut h, "provider.gemini");
    assert_eq!(logged(), before + 1);
}

#[test]
fn a_language_change_does_not_log_a_settings_service_add() {
    // 精靈寫回自己的草稿不是設定視窗的「新增第一筆服務」，那行 log 會誤導讀 app.log 的人
    let _lock = language_lock();
    capture_logs();
    let logged = || log_lines_with("[settings] first service added").len();
    let before = logged();
    let mut h = open("zh-TW", blank);
    next(&mut h);
    pick_provider(&mut h, "provider.claude");
    change_language_from_service_step(&mut h, "en-US");
    assert_eq!(logged(), before);
}
