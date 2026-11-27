//! overlay 的開啟遊戲入口：等待遊戲時的中央按鈕、標題列 ▶ 與兩者共用的帳號勾選選單。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use crate::common::kittest::tr;
use wizard101_chat_translator::i18n::t;
use wizard101_chat_translator::launcher::login::LoginError;
use wizard101_chat_translator::launcher::orchestrate::{Failure, LaunchStatus};
use wizard101_chat_translator::launcher::{Account, AccountKind};
use wizard101_chat_translator::ui::app::{App, channel};
use wizard101_chat_translator::ui::events::{UiEvent, UiPoster};
use wizard101_chat_translator::ui::fonts_egui;
use wizard101_chat_translator::ui::overlay::frame::{BAR_HEIGHT, Frame};
use wizard101_chat_translator::ui::overlay::launch_popup::MenuAction;

type Actions = Rc<RefCell<Vec<MenuAction>>>;

fn account(id: &str, name: &str) -> Account {
    Account {
        id: id.into(),
        name: name.into(),
        kind: AccountKind::Official,
        password_dpapi: String::new(),
    }
}

fn three() -> Vec<Account> {
    vec![
        account("a", "Alice"),
        account("b", "Bob"),
        account("c", "Carol"),
    ]
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// 狀態為 `status` 的 overlay，帳號 `accounts`、上次勾選 `selection`、執行中 `running`；回傳收到的選單動作。
fn overlay(
    size: Vec2,
    status: &'static str,
    accounts: Vec<Account>,
    selection: &[&str],
    running: &[&str],
) -> (Harness<'static, App>, Actions) {
    let (h, actions, _) = overlay_with_poster(size, status, accounts, selection, running);
    (h, actions)
}

/// 同 [`overlay`]，另回傳 UI 事件的投遞端。
fn overlay_with_poster(
    size: Vec2,
    status: &'static str,
    accounts: Vec<Account>,
    selection: &[&str],
    running: &[&str],
) -> (Harness<'static, App>, Actions, UiPoster) {
    let (poster, inbox) = channel();
    poster.post(UiEvent::SetStatus(status));
    let actions = Actions::default();
    let sink = Rc::clone(&actions);
    let (selection, running) = (ids(selection), ids(running));
    let mut h = Harness::builder().with_size(size).build_eframe(move |cc| {
        fonts_egui::install(&cc.egui_ctx, "en-US");
        let mut app = App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}));
        let menu = app.launch_menu_mut();
        menu.set_accounts(accounts, &selection);
        menu.set_running(running);
        app.set_on_launch(move |action| sink.borrow_mut().push(action));
        app
    });
    h.run_steps(3);
    (h, actions, poster)
}

fn click(h: &mut Harness<App>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    h.run_steps(3);
}

fn center_button() -> String {
    t("launch.open_game", &[])
}

fn open_count(n: usize) -> String {
    t("launch.open_count", &[("count", &n.to_string())])
}

fn big() -> Vec2 {
    Vec2::new(640.0, 420.0)
}

#[test]
fn waiting_shows_center_button() {
    let (h, _) = overlay(big(), "waiting_game", three(), &[], &[]);
    let button = h
        .get_by_role_and_label(Role::Button, &center_button())
        .rect();
    let status = h
        .query_all_by_label(&t("status.waiting_game", &[]))
        .map(|n| n.rect())
        .find(|r| r.min.y > BAR_HEIGHT)
        .expect("the centred status text");
    assert!(button.min.y > status.max.y, "{button:?} below {status:?}");
    assert!(
        (button.center().x - 320.0).abs() < 1.0,
        "{button:?} centred"
    );
    h.get_by_role_and_label(Role::Button, "▶");

    let (h, _) = overlay(big(), "listening", three(), &[], &[]);
    assert!(
        h.query_by_role_and_label(Role::Button, &center_button())
            .is_none()
    );
    h.get_by_role_and_label(Role::Button, "▶");
}

#[test]
fn menu_lists_accounts_with_running_badge() {
    let tall = Vec2::new(640.0, 640.0);
    let (mut h, _) = overlay(tall, "waiting_game", three(), &["a", "b", "c"], &["BOB"]);
    let button = h
        .get_by_role_and_label(Role::Button, &center_button())
        .rect();
    click(&mut h, Role::Button, &center_button());
    for (name, enabled) in [("Alice", true), ("Bob", false), ("Carol", true)] {
        let node = h.get_by_role_and_label(Role::CheckBox, name);
        assert_eq!(node.accesskit_node().is_disabled(), !enabled, "{name}");
    }
    let badge = h.get_by_label(&tr("accounts.running")).rect();
    let bob = h.get_by_role_and_label(Role::CheckBox, "Bob").rect();
    assert!(
        (badge.center().y - bob.center().y).abs() < 2.0,
        "badge on Bob's row"
    );
    h.get_by_role_and_label(Role::CheckBox, &tr("launch.select_all"));
    h.get_by_role_and_label(Role::Button, &open_count(2));
    let header = h
        .get_by_role_and_label(Role::CheckBox, &tr("launch.select_all"))
        .rect();
    assert!(
        header.min.y > button.max.y,
        "menu hangs below the centre button"
    );
}

#[test]
fn open_button_launches_only_the_non_running_checked_accounts() {
    let (mut h, actions) = overlay(big(), "waiting_game", three(), &["a", "b"], &["bob"]);
    click(&mut h, Role::Button, &center_button());
    click(&mut h, Role::Button, &open_count(1));
    assert_eq!(*actions.borrow(), [MenuAction::Launch(ids(&["a"]))]);
    h.get_by_label(&tr("launch.logging_in"));
    let none = open_count(0);
    let disabled = h.get_by_role_and_label(Role::Button, &none);
    assert!(
        disabled.accesskit_node().is_disabled(),
        "a second press must not relaunch"
    );
}

#[test]
fn checking_an_account_reports_the_new_selection() {
    let (mut h, actions) = overlay(big(), "waiting_game", three(), &["a"], &["bob"]);
    click(&mut h, Role::Button, &center_button());
    click(&mut h, Role::CheckBox, "Carol");
    assert_eq!(
        *actions.borrow(),
        [MenuAction::SelectionChanged(ids(&["a", "c"]))]
    );
    // 能勾的都勾了：全選先全取消，再按一次只勾回沒在執行的
    click(&mut h, Role::CheckBox, &tr("launch.select_all"));
    click(&mut h, Role::CheckBox, &tr("launch.select_all"));
    assert_eq!(
        actions.borrow()[1..],
        [
            MenuAction::SelectionChanged(Vec::new()),
            MenuAction::SelectionChanged(ids(&["a", "c"])),
        ]
    );
}

#[test]
fn title_bar_button_opens_the_menu_under_the_title_bar() {
    let (mut h, _) = overlay(big(), "listening", three(), &["a"], &[]);
    click(&mut h, Role::Button, "▶");
    let header = h
        .get_by_role_and_label(Role::CheckBox, &tr("launch.select_all"))
        .rect();
    assert!(
        header.min.y > BAR_HEIGHT && header.min.y < BAR_HEIGHT + 20.0,
        "{header:?}"
    );
    click(&mut h, Role::Button, "▶");
    assert!(h.query_by_label(&tr("launch.select_all")).is_none());
}

#[test]
fn pressing_outside_the_menu_closes_it() {
    let (mut h, _) = overlay(big(), "listening", three(), &["a"], &[]);
    click(&mut h, Role::Button, "▶");
    h.get_by_role_and_label(Role::CheckBox, &tr("launch.select_all"));
    let away = egui::pos2(40.0, 380.0);
    h.hover_at(away);
    h.drag_at(away);
    h.drop_at(away);
    h.run_steps(3);
    assert!(h.query_by_label(&tr("launch.select_all")).is_none());
}

#[test]
fn a_short_window_shows_the_compact_menu() {
    let (mut h, actions) = overlay(
        Vec2::new(400.0, 160.0),
        "waiting_game",
        three(),
        &["a", "c"],
        &[],
    );
    click(&mut h, Role::Button, &center_button());
    assert!(h.query_by_label(&tr("launch.select_all")).is_none());
    h.get_by_label(&tr("launch.taller_hint"));
    click(
        &mut h,
        Role::Button,
        &t("launch.open_last", &[("count", "2")]),
    );
    assert_eq!(*actions.borrow(), [MenuAction::Launch(ids(&["a", "c"]))]);
}

#[test]
fn the_compact_button_stays_inside_a_minimum_size_window() {
    let (mut h, _) = overlay(Vec2::new(200.0, 90.0), "listening", three(), &["a"], &[]);
    click(&mut h, Role::Button, "▶");
    let label = t("launch.open_last", &[("count", "1")]);
    let button = h.get_by_role_and_label(Role::Button, &label).rect();
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(200.0, 90.0));
    assert!(window.contains_rect(button), "{button:?}");
    assert!(button.min.y >= BAR_HEIGHT, "{button:?}");
}

#[test]
fn a_long_list_scrolls_with_the_wheel() {
    let many: Vec<Account> = (0..20)
        .map(|i| account(&i.to_string(), &format!("Account{i:02}")))
        .collect();
    let (mut h, _) = overlay(Vec2::new(400.0, 320.0), "listening", many, &[], &[]);
    click(&mut h, Role::Button, "▶");
    assert!(
        h.query_by_role_and_label(Role::CheckBox, "Account19")
            .is_none()
    );
    let first = h.get_by_role_and_label(Role::CheckBox, "Account00").rect();
    h.hover_at(first.center());
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Page,
        delta: egui::vec2(0.0, -10.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
    h.get_by_role_and_label(Role::CheckBox, "Account19");
    h.get_by_role_and_label(Role::CheckBox, &tr("launch.select_all"));
}

#[test]
fn a_rejected_login_shows_only_the_server_text() {
    let (mut h, _) = overlay(big(), "waiting_game", three(), &["a"], &[]);
    let raw = "Your client is out of date (code 7)".to_owned();
    let failed = LaunchStatus::Failed(Failure::Login(LoginError::Rejected { raw: raw.clone() }));
    h.state_mut()
        .launch_menu_mut()
        .set_launch_status("a", failed);
    click(&mut h, Role::Button, &center_button());
    h.get_by_label(&raw);
    assert!(h.query_by_label(&tr("launch.update_hint")).is_none());
}

#[test]
fn no_accounts_points_to_the_settings_page() {
    let (mut h, actions) = overlay(big(), "waiting_game", Vec::new(), &[], &[]);
    click(&mut h, Role::Button, &center_button());
    h.get_by_label(&tr("launch.no_accounts"));
    click(&mut h, Role::Button, &tr("launch.open_settings"));
    assert!(actions.borrow().is_empty(), "the app opens settings itself");
    assert!(h.query_by_label(&tr("launch.no_accounts")).is_none());
}

#[test]
fn launch_progress_and_running_scans_reach_the_menu() {
    let (mut h, _, poster) = overlay_with_poster(big(), "waiting_game", three(), &["a"], &[]);
    poster.post(UiEvent::LaunchStatus {
        id: "a".into(),
        status: LaunchStatus::Opened,
    });
    poster.post(UiEvent::RunningAccounts(ids(&["carol"])));
    click(&mut h, Role::Button, &center_button());
    h.get_by_label(&tr("launch.opened"));
    let carol = h.get_by_role_and_label(Role::CheckBox, "Carol");
    assert!(carol.accesskit_node().is_disabled(), "Carol is running");
}

#[test]
fn the_open_menu_rescans_running_accounts_every_few_seconds_not_every_frame() {
    let (mut h, _) = overlay(big(), "waiting_game", three(), &[], &[]);
    let scans = Rc::new(Cell::new(0));
    let counter = Rc::clone(&scans);
    h.state_mut()
        .set_on_scan_running(move || counter.set(counter.get() + 1));
    h.run_steps(5);
    assert_eq!(scans.get(), 0, "a closed menu does not scan");
    // kittest 每步前進 0.25 秒
    click(&mut h, Role::Button, &center_button());
    h.run_steps(4);
    assert_eq!(scans.get(), 1, "one scan on open");
    h.run_steps(8);
    assert_eq!(scans.get(), 2, "another after the interval");
    click(&mut h, Role::Button, &center_button());
    click(&mut h, Role::Button, &center_button());
    assert_eq!(scans.get(), 3, "reopening scans again");
}
