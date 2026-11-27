//! 設定視窗的「遊戲帳號」分頁：帳號清單、執行中標記、新增／編輯對話框的驗證、遊戲位置與自動關窗開關。

use crate::common;

use eframe::egui;
use eframe::egui::accesskit::{Role, Toggled};
use eframe::egui::text_edit::TextEditState;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use common::kittest::{click, click_in_dialog, click_nth, has_label, run, text_sections, tr};
use common::settings_env::FakeEnv;
use common::settings_window::{State, custom_cfg, harness as window_harness};
use wizard101_chat_translator::config::Config;
use wizard101_chat_translator::game::process::PROCESS_NAME;
use wizard101_chat_translator::i18n;
use wizard101_chat_translator::launcher::accounts::{
    decrypt_password, set_launch_auto_close, store, stored,
};
use wizard101_chat_translator::launcher::install::{Install, InstallSource};
use wizard101_chat_translator::launcher::login::LoginError;
use wizard101_chat_translator::launcher::orchestrate::{Failure, LaunchStatus};
use wizard101_chat_translator::launcher::{Account, AccountKind};
use wizard101_chat_translator::ui::settings::env::SettingsEvent;
use wizard101_chat_translator::ui::settings::launch_window::status_text;
use wizard101_chat_translator::ui::settings::theme;
use wizard101_chat_translator::ui::settings::widgets::card_label;
use wizard101_chat_translator::ui::settings::window::{Page, SettingsAction, SettingsWindow};

const LANGUAGE: &str = "en-US";

fn account(id: &str, name: &str, kind: AccountKind) -> Account {
    Account {
        id: id.into(),
        name: name.into(),
        kind,
        password_dpapi: "00".into(),
    }
}

fn two_accounts() -> Vec<Account> {
    vec![
        account("a", "alice", AccountKind::Official),
        account("b", "bob", AccountKind::Steam),
    ]
}

/// 打開帳號分頁，背景偵測與掃描還沒回來。
fn open_pending(accounts: &[Account], env: FakeEnv) -> Harness<'static, State> {
    i18n::set_language(LANGUAGE);
    let mut cfg: Config = custom_cfg();
    store(&mut cfg, accounts);
    let mut h = window_harness(&cfg, env, egui::vec2(840.0, 660.0), LANGUAGE);
    h.state_mut().win.set_page(Page::Accounts);
    run(&mut h);
    h
}

/// 打開帳號分頁，並把 `env` 的安裝位置與執行中帳號當成背景結果送回。
fn open(accounts: &[Account], env: FakeEnv) -> Harness<'static, State> {
    let mut h = open_pending(accounts, env);
    let installs = h.state().env.installs.clone();
    let names = h.state().env.running.clone();
    apply(&mut h, SettingsEvent::InstallsDetected { installs });
    apply(&mut h, SettingsEvent::RunningAccounts { names });
    h
}

fn apply(h: &mut Harness<State>, event: SettingsEvent) {
    h.state_mut().win.apply(&event);
    run(h);
}

fn launch_status(h: &mut Harness<State>, id: &str, status: LaunchStatus) {
    let id = id.into();
    apply(h, SettingsEvent::LaunchStatus { id, status });
}

/// 把 `label` 欄位整個換成 `text`。
fn replace_text(h: &mut Harness<State>, label: &str, text: &str) {
    h.get_by_label(label).click();
    run(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    if !text.is_empty() {
        h.event(egui::Event::Text(text.into()));
    }
    run(h);
}

fn dialog_save_disabled(h: &Harness<State>, title: &str) -> bool {
    let dialog = h.get_by_role_and_label(Role::Dialog, title);
    dialog
        .get_by_role_and_label(Role::Button, &tr("button.save"))
        .accesskit_node()
        .is_disabled()
}

/// 由上往下數第 `index` 顆〔啟動〕是否停用。
fn launch_disabled(h: &Harness<State>, index: usize) -> bool {
    let label = tr("button.launch");
    let mut nodes: Vec<_> = h
        .query_all_by_role_and_label(Role::Button, &label)
        .collect();
    nodes.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    nodes[index].accesskit_node().is_disabled()
}

fn commits(h: &Harness<State>) -> Vec<Config> {
    h.state()
        .actions
        .iter()
        .filter_map(|a| match a {
            SettingsAction::Commit(cfg) => Some(cfg.clone()),
            _ => None,
        })
        .collect()
}

/// 一個可用的安裝資料夾（有用戶端主程式與遊戲資料）。
fn fake_install() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("Data").join("GameData")).unwrap();
    std::fs::create_dir_all(dir.path().join("Bin")).unwrap();
    std::fs::write(dir.path().join("Bin").join(PROCESS_NAME), b"").unwrap();
    dir
}

fn deliver_folder(h: &mut Harness<State>, path: &std::path::Path) {
    let round = *h
        .state()
        .env
        .folder_picks
        .borrow()
        .last()
        .expect("picker opened");
    h.state_mut().win.apply(&SettingsEvent::GameFolderPicked {
        round,
        path: Some(path.to_path_buf()),
    });
    run(h);
}

#[test]
fn accounts_page_lists_saved_accounts() {
    let h = open(&two_accounts(), FakeEnv::default());
    assert!(has_label(
        &h,
        &card_label("alice", &[tr("accounts.kind_official")])
    ));
    assert!(has_label(
        &h,
        &card_label("bob", &[tr("accounts.kind_steam")])
    ));
    assert!(has_label(&h, &tr("accounts.steam_limit")));
    assert!(has_label(&h, &tr("launch.update_hint")));
}

#[test]
fn blank_name_blocks_save() {
    let mut h = open(&two_accounts(), FakeEnv::default());
    click_nth(&mut h, &tr("button.edit"), 0);
    let title = tr("accounts.edit_title");
    assert!(!dialog_save_disabled(&h, &title));
    replace_text(&mut h, &tr("accounts.field_name"), "   ");
    let error = tr("error.need_name");
    assert_eq!(text_sections(&h, &error)[0].1, theme::ERROR);
    assert!(dialog_save_disabled(&h, &title));
}

#[test]
fn name_with_a_space_or_unsendable_characters_blocks_save() {
    let mut h = open(&two_accounts(), FakeEnv::default());
    click_nth(&mut h, &tr("button.edit"), 0);
    let title = tr("accounts.edit_title");
    let error = tr("error.account_name_invalid");
    for bad in ["al ice", "alice王", "ali\tce"] {
        replace_text(&mut h, &tr("accounts.field_name"), bad);
        assert_eq!(text_sections(&h, &error)[0].1, theme::ERROR, "{bad:?}");
        assert!(dialog_save_disabled(&h, &title));
    }
    replace_text(&mut h, &tr("accounts.field_name"), " alicé.2 ");
    assert!(!has_label(&h, &error));
    assert!(!dialog_save_disabled(&h, &title));
}

#[test]
fn duplicate_name_blocks_save() {
    let mut h = open(&two_accounts(), FakeEnv::default());
    click_nth(&mut h, &tr("button.edit"), 1);
    let title = tr("accounts.edit_title");
    replace_text(&mut h, &tr("accounts.field_name"), " ALICE ");
    let error = tr("error.account_name_taken");
    assert_eq!(text_sections(&h, &error)[0].1, theme::ERROR);
    assert!(dialog_save_disabled(&h, &title));
    replace_text(&mut h, &tr("accounts.field_name"), "bob");
    assert!(!has_label(&h, &error));
    assert!(!dialog_save_disabled(&h, &title));
}

#[test]
fn new_account_needs_a_password() {
    let mut h = open(&[], FakeEnv::default());
    click(&mut h, Role::Button, &tr("button.add_account"));
    replace_text(&mut h, &tr("accounts.field_name"), "carol");
    assert!(has_label(&h, &tr("error.need_password")));
    assert!(dialog_save_disabled(&h, &tr("accounts.new_title")));
}

#[test]
fn saving_a_new_account_commits_it_with_an_encrypted_password() {
    let mut h = open(&two_accounts(), FakeEnv::default());
    click(&mut h, Role::Button, &tr("button.add_account"));
    click(&mut h, Role::Button, &tr("accounts.kind_steam"));
    replace_text(&mut h, &tr("accounts.field_name"), "  carol ");
    replace_text(&mut h, &tr("accounts.field_password"), "s3cret");
    click_in_dialog(&mut h, &tr("accounts.new_title"), &tr("button.save"));

    let commits = commits(&h);
    assert_eq!(commits.len(), 1);
    let saved = stored(&commits[0]);
    assert_eq!(saved.len(), 3);
    let carol = &saved[2];
    assert_eq!(
        (carol.name.as_str(), carol.kind),
        ("carol", AccountKind::Steam)
    );
    assert_ne!(carol.password_dpapi, "s3cret");
    assert_eq!(
        decrypt_password(&carol.password_dpapi).unwrap().as_str(),
        "s3cret"
    );
    assert!(!h.state().win.has_unsaved());
}

#[test]
fn editing_without_a_password_keeps_the_old_one() {
    let mut h = open(&two_accounts(), FakeEnv::default());
    click_nth(&mut h, &tr("button.edit"), 0);
    replace_text(&mut h, &tr("accounts.field_name"), "alice2");
    click_in_dialog(&mut h, &tr("accounts.edit_title"), &tr("button.save"));
    let saved = stored(&commits(&h)[0]);
    assert_eq!(saved[0].name, "alice2");
    assert_eq!(saved[0].password_dpapi, "00");
}

fn text_edit_states(h: &Harness<State>) -> usize {
    h.ctx.data(|d| d.count::<TextEditState>())
}

#[test]
fn password_edit_state_is_cleared_when_the_dialog_goes_away() {
    let mut h = open(&two_accounts(), FakeEnv::default());
    click(&mut h, Role::Button, &tr("button.add_account"));
    replace_text(&mut h, &tr("accounts.field_password"), "s3cret");
    let shown = text_edit_states(&h);
    h.key_press(egui::Key::Escape);
    run(&mut h);
    assert_eq!(text_edit_states(&h), shown - 1);

    click(&mut h, Role::Button, &tr("button.add_account"));
    replace_text(&mut h, &tr("accounts.field_password"), "s3cret");
    let shown = text_edit_states(&h);
    h.state_mut().win = SettingsWindow::new(&Config::new());
    assert_eq!(text_edit_states(&h), shown - 1);
}

#[test]
fn running_account_shows_badge_and_disables_open() {
    let env = FakeEnv {
        running: vec!["ALICE".into()],
        ..FakeEnv::default()
    };
    let h = open(&two_accounts(), env);
    let running = [tr("accounts.kind_official"), tr("accounts.running")];
    assert!(has_label(&h, &card_label("alice", &running)));
    assert!(has_label(
        &h,
        &card_label("bob", &[tr("accounts.kind_steam")])
    ));
    assert!(launch_disabled(&h, 0));
    assert!(!launch_disabled(&h, 1));
}

#[test]
fn entering_the_page_detects_in_the_background() {
    let mut h = open_pending(&[], FakeEnv::default());
    assert_eq!(h.state().env.install_detections.get(), 1);
    assert_eq!(h.state().env.running_scans.get(), 1);
    assert!(has_label(&h, &tr("accounts.detecting")));
    assert!(!has_label(&h, &tr("accounts.install_none")));
    run(&mut h);
    assert_eq!(
        h.state().env.install_detections.get(),
        1,
        "one detection in flight"
    );
    assert_eq!(h.state().env.running_scans.get(), 1, "one scan in flight");

    let installs = vec![Install {
        source: InstallSource::Official,
        root: r"D:\Games\Wizard101".into(),
    }];
    apply(&mut h, SettingsEvent::InstallsDetected { installs });
    assert!(!has_label(&h, &tr("accounts.detecting")));
    let line = i18n::t(
        "accounts.install_official",
        &[("path", r"D:\Games\Wizard101")],
    );
    assert!(has_label(&h, &line));
}

#[test]
fn logging_in_disables_launch_and_shows_the_status() {
    let mut h = open(&two_accounts(), FakeEnv::default());
    assert!(!launch_disabled(&h, 1));
    launch_status(&mut h, "b", LaunchStatus::LoggingIn);
    assert!(launch_disabled(&h, 1));
    assert!(!launch_disabled(&h, 0));
    assert!(has_label(&h, &status_text(&LaunchStatus::LoggingIn)));
}

#[test]
fn rejected_login_shows_the_server_text_verbatim() {
    let mut h = open(&two_accounts(), FakeEnv::default());
    let raw = "0xd: server says no".to_owned();
    let failed = LaunchStatus::Failed(Failure::Login(LoginError::Rejected { raw: raw.clone() }));
    launch_status(&mut h, "a", failed);
    assert!(has_label(&h, &raw));
    assert_eq!(text_sections(&h, &raw)[0].1, theme::ERROR);
    assert!(!launch_disabled(&h, 0));
}

#[test]
fn editing_an_account_clears_its_last_failure() {
    let mut h = open(&two_accounts(), FakeEnv::default());
    let raw = "server says no".to_owned();
    let failed = LaunchStatus::Failed(Failure::Login(LoginError::Rejected { raw: raw.clone() }));
    launch_status(&mut h, "a", failed);
    click_nth(&mut h, &tr("button.edit"), 0);
    replace_text(&mut h, &tr("accounts.field_password"), "fixed");
    click_in_dialog(&mut h, &tr("accounts.edit_title"), &tr("button.save"));
    assert!(!has_label(&h, &raw));
}

#[test]
fn launch_button_requests_that_account() {
    let mut h = open(&two_accounts(), FakeEnv::default());
    click_nth(&mut h, &tr("button.launch"), 1);
    assert!(
        h.state()
            .actions
            .contains(&SettingsAction::LaunchAccounts(vec!["b".into()]))
    );
}

#[test]
fn picked_game_folder_must_be_a_valid_install() {
    let mut h = open(&[], FakeEnv::default());
    assert!(has_label(&h, &tr("accounts.install_none")));

    let invalid = tempfile::tempdir().unwrap();
    click(&mut h, Role::Button, &tr("button.change"));
    deliver_folder(&mut h, invalid.path());
    assert!(has_label(&h, &tr("accounts.invalid_folder")));
    assert!(!h.state().win.has_unsaved());

    let valid = fake_install();
    click(&mut h, Role::Button, &tr("button.change"));
    deliver_folder(&mut h, valid.path());
    assert!(!has_label(&h, &tr("accounts.invalid_folder")));
    let path = valid.path().display().to_string();
    assert!(has_label(
        &h,
        &i18n::t("accounts.install_official", &[("path", &path)])
    ));
}

#[test]
fn picking_a_valid_folder_commits_game_path() {
    let mut h = open(&[], FakeEnv::default());
    let valid = fake_install();
    click(&mut h, Role::Button, &tr("button.change"));
    deliver_folder(&mut h, valid.path());
    let path = valid.path().display().to_string();
    let committed = commits(&h);
    assert_eq!(committed.len(), 1);
    assert_eq!(
        committed[0].get("game_path"),
        Some(&serde_json::json!(path))
    );
    assert!(!h.state().win.has_unsaved());
}

fn auto_close_on(h: &Harness<State>) -> bool {
    let label = tr("accounts.auto_close");
    let switch = h.get_by_role_and_label(Role::Switch, &label);
    switch.accesskit_node().toggled() == Some(Toggled::True)
}

fn saved_cfg(h: &mut Harness<State>) -> Config {
    click(h, Role::Button, &tr("button.save"));
    h.state()
        .actions
        .iter()
        .find_map(|a| match a {
            SettingsAction::Save(cfg) => Some(cfg.clone()),
            _ => None,
        })
        .expect("saved")
}

#[test]
fn auto_close_switch_shows_the_saved_value_and_saves_with_the_window() {
    let mut cfg: Config = custom_cfg();
    set_launch_auto_close(&mut cfg, false);
    i18n::set_language(LANGUAGE);
    let mut h = window_harness(&cfg, FakeEnv::default(), egui::vec2(840.0, 660.0), LANGUAGE);
    h.state_mut().win.set_page(Page::Accounts);
    run(&mut h);
    assert!(!auto_close_on(&h));
    click(&mut h, Role::Switch, &tr("accounts.auto_close"));
    assert!(auto_close_on(&h));
    assert!(h.state().win.has_unsaved());
    assert_eq!(saved_cfg(&mut h)["launch_auto_close"], json!(true));
}

#[test]
fn auto_close_changed_in_the_launch_window_shows_without_unsaved_changes() {
    let mut h = open(&two_accounts(), FakeEnv::default());
    assert!(auto_close_on(&h), "on when never set");
    apply(&mut h, SettingsEvent::LaunchAutoClose(false));
    assert!(!auto_close_on(&h));
    assert!(!h.state().win.has_unsaved());
    assert_eq!(saved_cfg(&mut h)["launch_auto_close"], json!(false));
}
