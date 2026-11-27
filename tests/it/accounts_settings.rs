//! 設定視窗的「帳號」分頁：帳號清單、執行中標記、新增／編輯對話框的驗證與遊戲位置。

use crate::common;

use eframe::egui;
use eframe::egui::accesskit::Role;
use eframe::egui::text_edit::TextEditState;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use common::kittest::{click, click_in_dialog, click_nth, has_label, run, text_sections, tr};
use common::settings_env::FakeEnv;
use common::settings_window::{State, custom_cfg, harness as window_harness};
use wizard101_chat_translator::config::Config;
use wizard101_chat_translator::game::process::PROCESS_NAME;
use wizard101_chat_translator::i18n;
use wizard101_chat_translator::launcher::accounts::{decrypt_password, store, stored};
use wizard101_chat_translator::launcher::{Account, AccountKind};
use wizard101_chat_translator::ui::settings::env::SettingsEvent;
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

fn open(accounts: &[Account], env: FakeEnv) -> Harness<'static, State> {
    i18n::set_language(LANGUAGE);
    let mut cfg: Config = custom_cfg();
    store(&mut cfg, accounts);
    let mut h = window_harness(&cfg, env, egui::vec2(840.0, 660.0), LANGUAGE);
    h.state_mut().win.set_page(Page::Accounts);
    run(&mut h);
    h
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

/// 由上往下數第 `index` 顆〔開啟〕是否停用。
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
    assert_eq!(decrypt_password(&carol.password_dpapi).unwrap(), "s3cret");
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
    let running = [tr("accounts.running"), tr("accounts.kind_official")];
    assert!(has_label(&h, &card_label("alice", &running)));
    assert!(has_label(
        &h,
        &card_label("bob", &[tr("accounts.kind_steam")])
    ));
    assert!(launch_disabled(&h, 0));
    assert!(!launch_disabled(&h, 1));
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
    assert!(h.state().win.has_unsaved());
}
