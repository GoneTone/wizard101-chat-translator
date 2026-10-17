//! 設定視窗換介面語言的預覽、儲存與取消；會切換全域介面語言，獨立成一個測試執行檔，檔內以鎖排隊。

mod common;

use std::sync::{Mutex, MutexGuard};

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Ui};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use common::log_capture::{capture_logs, log_lines_with};
use common::settings_env::FakeEnv;
use wizard101_chat_translator::config::services::new_service;
use wizard101_chat_translator::config::{Config, default_config};
use wizard101_chat_translator::i18n::{self, current_language, language_name, t};
use wizard101_chat_translator::ui::fonts_egui;
use wizard101_chat_translator::ui::settings::theme;
use wizard101_chat_translator::ui::settings::window::{Page, SettingsAction, SettingsWindow};

/// 全域介面語言同一時間只給一個測試用。
fn language_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn tr(key: &str) -> String {
    t(key, &[])
}

fn configured_cfg() -> Config {
    let mut cfg = default_config();
    let mut service = new_service("custom", &[]);
    service["base_url"] = json!("http://x");
    service["model"] = json!("m");
    cfg.insert("default_service".into(), service["id"].clone());
    cfg.insert("services".into(), json!([service]));
    cfg.insert("ui_language".into(), json!(current_language()));
    cfg
}

struct State {
    win: SettingsWindow,
    env: FakeEnv,
    actions: Vec<SettingsAction>,
}

/// 以 `language` 開窗。
fn harness(language: &str) -> Harness<'static, State> {
    i18n::set_language(language);
    let state = State {
        win: SettingsWindow::new(&configured_cfg()),
        env: FakeEnv::default(),
        actions: Vec::new(),
    };
    let mut h = Harness::builder()
        .with_size(egui::vec2(840.0, 660.0))
        .with_context_setup(|ctx| fonts_egui::install(ctx, "zh-TW"))
        .build_ui_state(
            |ui: &mut Ui, s: &mut State| {
                theme::apply(ui.style_mut());
                let actions = s.win.ui(ui, &s.env);
                s.actions.extend(actions);
            },
            state,
        );
    h.run_steps(3);
    h
}

fn run<S>(h: &mut Harness<S>) {
    h.run_steps(3);
}

fn click<S>(h: &mut Harness<S>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    run(h);
}

/// 從介面語言下拉選 `code`。
fn pick_language(h: &mut Harness<State>, code: &str) {
    click(h, Role::ComboBox, &tr("field.ui_language"));
    click(h, Role::Button, &language_name(code));
}

fn has_label<S>(h: &Harness<S>, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some()
}

fn saved(h: &Harness<State>) -> Option<Config> {
    h.state().actions.iter().find_map(|a| match a {
        SettingsAction::Save(cfg) => Some(cfg.clone()),
        _ => None,
    })
}

#[test]
fn changing_ui_language_previews_it_without_touching_config() {
    let _lock = language_lock();
    capture_logs();
    // 語言鎖讓同一時間只有本測試在記這行，前後筆數相減才準
    let logged = || log_lines_with("[ui] settings previewing language en-US").len();
    let before = logged();
    let mut h = harness("zh-TW");
    pick_language(&mut h, "en-US");
    assert_eq!(current_language(), "en-US");
    assert_eq!(h.state().actions, [SettingsAction::PreviewLanguage]);
    assert_eq!(logged(), before + 1);
    // 視窗以新語言重畫
    h.get_by_role_and_label(Role::Tab, "Basic");
    h.get_by_role_and_label(Role::Button, "Save");
}

#[test]
fn picking_the_current_language_previews_nothing() {
    let _lock = language_lock();
    let mut h = harness("en-US");
    pick_language(&mut h, "en-US");
    assert!(h.state().actions.is_empty());
}

#[test]
fn language_preview_keeps_the_draft_and_cancel_reverts() {
    let _lock = language_lock();
    capture_logs();
    let mut h = harness("zh-TW");
    h.get_by_role_and_label(Role::TextInput, &tr("settings.target_language"))
        .click();
    run(&mut h);
    h.event(egui::Event::Text("X".into()));
    run(&mut h);
    let typed = h
        .get_by_role_and_label(Role::TextInput, &tr("settings.target_language"))
        .value();
    assert!(
        typed.as_deref().is_some_and(|v| v.contains('X')),
        "{typed:?}"
    );
    pick_language(&mut h, "en-US");
    assert_eq!(h.state().win.page(), Page::Basic);
    let field = h.get_by_role_and_label(Role::TextInput, "Translate messages into");
    assert_eq!(field.value(), typed);

    h.state_mut().actions.clear();
    let reverted = || log_lines_with("[ui] settings language preview reverted to zh-TW").len();
    let before = reverted();
    click(&mut h, Role::Button, "Cancel");
    assert_eq!(current_language(), "zh-TW");
    assert_eq!(
        h.state().actions,
        [SettingsAction::PreviewLanguage, SettingsAction::Cancel]
    );
    assert_eq!(reverted(), before + 1);
}

#[test]
fn cancel_through_the_window_close_reverts_too() {
    let _lock = language_lock();
    let mut h = harness("zh-TW");
    pick_language(&mut h, "en-US");
    let State { win, env, .. } = h.state_mut();
    let actions = win.cancel(env);
    assert_eq!(
        actions,
        [SettingsAction::PreviewLanguage, SettingsAction::Cancel]
    );
    assert_eq!(current_language(), "zh-TW");
}

#[test]
fn save_keeps_the_previewed_language() {
    let _lock = language_lock();
    let mut h = harness("zh-TW");
    pick_language(&mut h, "en-US");
    click(&mut h, Role::Button, "Save");
    assert_eq!(current_language(), "en-US");
    assert_eq!(saved(&h).unwrap()["ui_language"], json!("en-US"));
}

#[test]
fn language_preview_keeps_a_half_added_service() {
    // 在服務分頁改了還沒儲存，就回基本分頁換介面語言：清單必須還帶著那筆修改
    let _lock = language_lock();
    let mut h = harness("zh-TW");
    click(&mut h, Role::Tab, &tr("settings.tab.services"));
    click(&mut h, Role::Button, &tr("button.edit"));
    h.get_by_label(&tr("field.service_name")).click();
    run(&mut h);
    h.event(egui::Event::Text(" half".into()));
    run(&mut h);
    h.get_by_role_and_label(Role::Dialog, &tr("service.dialog_title"))
        .get_by_role_and_label(Role::Button, &tr("button.ok"))
        .click();
    run(&mut h);
    click(&mut h, Role::Tab, &tr("settings.tab.basic"));
    pick_language(&mut h, "en-US");
    click(&mut h, Role::Button, "Save");
    let cfg = saved(&h).unwrap();
    assert!(
        cfg["services"][0]["name"]
            .as_str()
            .unwrap()
            .ends_with(" half")
    );
}

#[test]
fn translators_rows_are_hidden_when_the_language_credits_nobody() {
    let _lock = language_lock();
    let mut h = harness("zh-TW");
    assert!(!has_label(&h, &tr("credit.translators")));
    // 邀請協助翻譯的連結不看有沒有譯者：沒人翻的語言更需要
    h.get_by_role_and_label(Role::Link, &format!("{} ↗", tr("credit.help_translate")));
    click(&mut h, Role::Tab, &tr("settings.tab.about"));
    assert!(!has_label(&h, &tr("credit.translators")));
    assert!(!has_label(&h, "Claude Code"));
}

#[test]
fn translators_follow_the_previewed_language() {
    let _lock = language_lock();
    let mut h = harness("zh-TW");
    assert!(!has_label(&h, "Claude Code"));
    pick_language(&mut h, "en-US");
    h.get_by_role_and_label(Role::Link, "Claude Code");
}
