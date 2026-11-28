//! 啟動遊戲帳號視窗：帳號列（勾選、標記、單獨〔啟動〕與進度）、全選與批次啟動、自動關窗，以及 overlay 的 ▶ 與中央按鈕開窗。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use crate::common::frames::Frames;
use crate::common::kittest::{click, click_nth, has_label, run, themed_harness, tr};
use wizard101_chat_translator::i18n::{self, t};
use wizard101_chat_translator::launcher::install::InstallSource;
use wizard101_chat_translator::launcher::login::LoginError;
use wizard101_chat_translator::launcher::orchestrate::{Failure, LaunchStatus};
use wizard101_chat_translator::launcher::patch::{PatchFailure, PatchStatus};
use wizard101_chat_translator::launcher::{Account, AccountKind};
use wizard101_chat_translator::ui::app::{App, channel};
use wizard101_chat_translator::ui::events::{UiEvent, UiPoster};
use wizard101_chat_translator::ui::fonts_egui;
use wizard101_chat_translator::ui::launch_host::LaunchHost;
use wizard101_chat_translator::ui::overlay::frame::{BAR_HEIGHT, Frame};
use wizard101_chat_translator::ui::settings::launch_list::LaunchList;
use wizard101_chat_translator::ui::settings::launch_window::{self, LaunchAction};
use wizard101_chat_translator::ui::settings::patch_card::{PatchBoard, install_name, status_text};
use wizard101_chat_translator::ui::settings::theme;
use wizard101_chat_translator::ui::settings_host::WindowOps;

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

fn open_count(n: usize) -> String {
    t("launch.open_count", &[("count", &n.to_string())])
}

fn rejected(raw: &str) -> LaunchStatus {
    let raw = raw.to_owned();
    LaunchStatus::Failed(Failure::Login(LoginError::Rejected { raw }))
}

// --- 視窗畫面 ---

struct Window {
    list: LaunchList,
    patch: PatchBoard,
    auto_close: bool,
    actions: Vec<LaunchAction>,
}

fn window_sized(
    size: Vec2,
    accounts: Vec<Account>,
    selection: &[&str],
    running: &[&str],
) -> Harness<'static, Window> {
    i18n::set_language("en-US");
    let mut list = LaunchList::new(accounts, &ids(selection));
    list.set_running(ids(running));
    let window = Window {
        list,
        patch: PatchBoard::default(),
        auto_close: true,
        actions: Vec::new(),
    };
    themed_harness(
        size,
        "en-US",
        |ui, w: &mut Window| {
            let actions = launch_window::show(ui, &mut w.list, &w.patch, &mut w.auto_close);
            w.actions.extend(actions);
        },
        window,
    )
}

fn window(
    accounts: Vec<Account>,
    selection: &[&str],
    running: &[&str],
) -> Harness<'static, Window> {
    window_sized(theme::LAUNCH_SIZE, accounts, selection, running)
}

/// 由上往下數第 `index` 顆〔啟動〕是否停用。
fn launch_disabled(h: &Harness<Window>, index: usize) -> bool {
    let label = tr("button.launch");
    let mut nodes: Vec<_> = h
        .query_all_by_role_and_label(Role::Button, &label)
        .collect();
    nodes.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    nodes[index].accesskit_node().is_disabled()
}

#[test]
fn each_row_has_a_checkbox_badges_and_its_own_launch_button() {
    let h = window(three(), &["a", "b", "c"], &["BOB"]);
    for (name, enabled) in [("Alice", true), ("Bob", false), ("Carol", true)] {
        let node = h.get_by_role_and_label(Role::CheckBox, name);
        assert_eq!(node.accesskit_node().is_disabled(), !enabled, "{name}");
    }
    assert_eq!(
        h.query_all_by_role_and_label(Role::Button, &tr("button.launch"))
            .count(),
        3
    );
    assert!(!launch_disabled(&h, 0));
    assert!(launch_disabled(&h, 1), "Bob is running");
    assert!(has_label(&h, &tr("accounts.running")));
    assert!(has_label(&h, &tr("accounts.kind_official")));
    h.get_by_role_and_label(Role::CheckBox, &tr("launch.select_all"));
    h.get_by_role_and_label(Role::Button, &open_count(2));
}

#[test]
fn a_row_button_launches_only_that_account() {
    let mut h = window(three(), &["b"], &[]);
    click_nth(&mut h, &tr("button.launch"), 0);
    assert_eq!(h.state().actions, [LaunchAction::Launch(ids(&["a"]))]);
    h.get_by_label(&tr("launch.logging_in"));
    assert!(launch_disabled(&h, 0), "a second press must not relaunch");
    let alice = h.get_by_role_and_label(Role::CheckBox, "Alice");
    assert!(alice.accesskit_node().is_disabled());
    h.get_by_role_and_label(Role::Button, &open_count(1));
}

#[test]
fn the_batch_button_launches_only_the_non_running_checked_accounts() {
    let mut h = window(three(), &["a", "b"], &["bob"]);
    click(&mut h, Role::Button, &open_count(1));
    assert_eq!(h.state().actions, [LaunchAction::Launch(ids(&["a"]))]);
    let none = open_count(0);
    let disabled = h.get_by_role_and_label(Role::Button, &none);
    assert!(disabled.accesskit_node().is_disabled());
}

#[test]
fn checking_an_account_reports_the_new_selection() {
    let mut h = window(three(), &["a"], &["bob"]);
    click(&mut h, Role::CheckBox, "Carol");
    assert_eq!(
        h.state().actions,
        [LaunchAction::SelectionChanged(ids(&["a", "c"]))]
    );
    // 能勾的都勾了：全選先全取消，再按一次只勾回沒在執行的
    click(&mut h, Role::CheckBox, &tr("launch.select_all"));
    click(&mut h, Role::CheckBox, &tr("launch.select_all"));
    assert_eq!(
        h.state().actions[1..],
        [
            LaunchAction::SelectionChanged(Vec::new()),
            LaunchAction::SelectionChanged(ids(&["a", "c"])),
        ]
    );
}

#[test]
fn toggling_auto_close_reports_the_new_value() {
    let mut h = window(three(), &[], &[]);
    click(&mut h, Role::CheckBox, &tr("launch.auto_close"));
    click(&mut h, Role::CheckBox, &tr("launch.auto_close"));
    assert_eq!(
        h.state().actions,
        [
            LaunchAction::AutoCloseChanged(false),
            LaunchAction::AutoCloseChanged(true),
        ]
    );
}

#[test]
fn a_rejected_login_shows_only_the_server_text() {
    let mut h = window(three(), &["a"], &[]);
    let raw = "Your client is out of date (code 7)";
    h.state_mut().list.set_status("a", rejected(raw));
    run(&mut h);
    h.get_by_label(raw);
}

fn patch(h: &mut Harness<Window>, source: InstallSource, status: Option<PatchStatus>) {
    h.state_mut().patch.apply(source, status, Instant::now());
    run(h);
}

#[test]
fn patch_card_shows_progress_and_waiting_rows() {
    let mut h = window(three(), &["a", "b"], &[]);
    let downloading = PatchStatus::Downloading {
        done: 1 << 30,
        total: 3 << 30,
    };
    patch(&mut h, InstallSource::Official, Some(downloading.clone()));
    for id in ["a", "b"] {
        let waiting = LaunchStatus::Waiting(InstallSource::Official);
        h.state_mut().list.set_status(id, waiting);
    }
    run(&mut h);
    h.get_by_label(&install_name(InstallSource::Official));
    h.get_by_role(Role::ProgressIndicator);
    h.get_by_label(&status_text(&downloading, None).0);
    let waiting = h.query_all_by_label(&tr("launch.waiting_update")).count();
    assert_eq!(waiting, 2);
    assert!(launch_disabled(&h, 0) && launch_disabled(&h, 1));
    assert!(!launch_disabled(&h, 2), "Carol is not waiting");
}

#[test]
fn steam_outdated_shows_reason_once_and_rows_not_started() {
    let mut h = window(three(), &["a", "b"], &[]);
    let outdated = PatchStatus::Failed(PatchFailure::SteamOutdated);
    patch(&mut h, InstallSource::Steam, Some(outdated.clone()));
    for id in ["a", "b"] {
        h.state_mut()
            .list
            .set_status(id, LaunchStatus::Failed(Failure::Patch));
    }
    run(&mut h);
    let (reason, _) = status_text(&outdated, None);
    assert_eq!(h.query_all_by_label(&reason).count(), 1);
    assert_eq!(h.query_all_by_label(&tr("launch.not_started")).count(), 2);
    h.get_by_label(&install_name(InstallSource::Steam));
    assert!(h.query_by_role(Role::ProgressIndicator).is_none());
    assert!(!launch_disabled(&h, 0), "a failed row can be retried");
}

#[test]
fn patch_card_hidden_when_board_empty() {
    let mut h = window(three(), &[], &[]);
    let top = |h: &Harness<Window>| {
        h.get_by_role_and_label(Role::CheckBox, "Alice")
            .rect()
            .top()
    };
    let bare = top(&h);
    let name = install_name(InstallSource::Official);
    assert!(!has_label(&h, &name));
    patch(&mut h, InstallSource::Official, Some(PatchStatus::UpToDate));
    h.get_by_label(&tr("patch.up_to_date"));
    assert!(top(&h) > bare, "the card pushes the list down");
    patch(&mut h, InstallSource::Official, None);
    assert!(!has_label(&h, &name));
    assert!(!has_label(&h, &tr("patch.up_to_date")));
    assert_eq!(top(&h), bare, "the list moves back up");
}

#[test]
fn the_patch_card_scrolls_with_the_list_at_min_size() {
    let size = theme::LAUNCH_MIN_SIZE;
    let mut h = window_sized(size, three(), &["c"], &[]);
    let downloading = PatchStatus::Downloading {
        done: 1 << 30,
        total: 3 << 30,
    };
    let checking = PatchStatus::Checking {
        done: 2310,
        total: 3735,
    };
    patch(&mut h, InstallSource::Official, Some(downloading));
    patch(&mut h, InstallSource::Steam, Some(checking));
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let header = h
        .get_by_role_and_label(Role::CheckBox, &tr("launch.select_all"))
        .rect();
    let open = h.get_by_role_and_label(Role::Button, &open_count(1)).rect();
    let auto_close = h
        .get_by_role_and_label(Role::CheckBox, &tr("launch.auto_close"))
        .rect();
    for rect in [header, open, auto_close] {
        assert!(window.contains_rect(rect), "{rect:?}");
    }
    let bar = |h: &Harness<Window>| {
        h.query_all_by_role(Role::ProgressIndicator)
            .map(|n| n.rect())
            .min_by(|a, b| a.top().total_cmp(&b.top()))
            .expect("a progress bar")
    };
    assert!(
        bar(&h).min.y >= header.max.y,
        "the card starts under the header"
    );
    let last = |h: &Harness<Window>| h.get_by_role_and_label(Role::CheckBox, "Carol").rect();
    assert!(last(&h).min.y > open.min.y, "Carol starts out of view");

    h.hover_at(bar(&h).center());
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Page,
        delta: egui::vec2(0.0, -10.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    });
    run(&mut h);
    let scrolled = last(&h);
    assert!(
        scrolled.max.y <= open.min.y && scrolled.min.y >= header.max.y,
        "{scrolled:?}"
    );
    assert!(
        bar(&h).max.y < header.max.y,
        "the card scrolled away with the list"
    );
    click(&mut h, Role::Button, &open_count(1));
    assert_eq!(h.state().actions, [LaunchAction::Launch(ids(&["c"]))]);
}

#[test]
fn no_accounts_offers_to_add_one() {
    let mut h = window(Vec::new(), &[], &[]);
    h.get_by_label(&tr("launch.no_accounts"));
    assert!(!has_label(&h, &tr("launch.select_all")));
    click(&mut h, Role::Button, &tr("launch.go_add_account"));
    assert_eq!(h.state().actions, [LaunchAction::AddAccount]);
}

#[test]
fn a_long_list_scrolls_between_a_fixed_header_and_footer() {
    let many: Vec<Account> = (0..20)
        .map(|i| account(&i.to_string(), &format!("Account{i:02}")))
        .collect();
    let size = theme::LAUNCH_MIN_SIZE;
    let mut h = window_sized(size, many, &[], &[]);
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let header = h
        .get_by_role_and_label(Role::CheckBox, &tr("launch.select_all"))
        .rect();
    let open = h.get_by_role_and_label(Role::Button, &open_count(0)).rect();
    let auto_close = h
        .get_by_role_and_label(Role::CheckBox, &tr("launch.auto_close"))
        .rect();
    for rect in [header, open, auto_close] {
        assert!(window.contains_rect(rect), "{rect:?}");
    }
    assert!(auto_close.max.x <= open.min.x, "{auto_close:?} {open:?}");
    let last = |h: &Harness<Window>| h.get_by_role_and_label(Role::CheckBox, "Account19").rect();
    assert!(last(&h).min.y > open.min.y, "starts out of view");
    let first = h.get_by_role_and_label(Role::CheckBox, "Account00").rect();
    h.hover_at(first.center());
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Page,
        delta: egui::vec2(0.0, -10.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    });
    run(&mut h);
    let scrolled = last(&h);
    assert!(
        scrolled.max.y <= open.min.y && scrolled.min.y >= header.max.y,
        "{scrolled:?}"
    );
}

// --- overlay 的入口與 App 的接線 ---

type Actions = Rc<RefCell<Vec<LaunchAction>>>;

/// 狀態為 `status` 的 overlay，帳號 `accounts`、上次勾選 `selection`；回傳收到的動作與 UI 事件的投遞端。
fn overlay(
    status: &'static str,
    accounts: Vec<Account>,
    selection: &[&str],
) -> (Harness<'static, App>, Actions, UiPoster) {
    i18n::set_language("en-US");
    let (poster, inbox) = channel();
    poster.post(UiEvent::SetStatus(status));
    let actions = Actions::default();
    let sink = Rc::clone(&actions);
    let selection = ids(selection);
    let mut h = Harness::builder()
        .with_size(Vec2::new(1280.0, 900.0))
        .build_eframe(move |cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            let mut app = App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}));
            let launch = app.launch_mut();
            launch.set_window_ops(WindowOps::offscreen());
            launch.set_accounts(accounts, &selection);
            app.set_on_launch(move |action| sink.borrow_mut().push(action));
            app
        });
    h.run_steps(3);
    (h, actions, poster)
}

fn center_button() -> String {
    t("launch.open_game", &[])
}

fn status(poster: &UiPoster, id: &str, status: LaunchStatus) {
    let id = id.into();
    poster.post(UiEvent::LaunchStatus { id, status });
}

#[test]
fn waiting_shows_center_button() {
    let (h, _, _) = overlay("waiting_game", three(), &[]);
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
        (button.center().x - 640.0).abs() < 1.0,
        "{button:?} centred"
    );
    h.get_by_role_and_label(Role::Button, "▶");

    let (h, _, _) = overlay("listening", three(), &[]);
    assert!(
        h.query_by_role_and_label(Role::Button, &center_button())
            .is_none()
    );
    h.get_by_role_and_label(Role::Button, "▶");
}

#[test]
fn the_title_bar_button_and_the_centre_button_open_the_window() {
    let (mut h, _, _) = overlay("waiting_game", three(), &[]);
    assert!(!h.state_mut().launch_mut().is_open());
    click(&mut h, Role::Button, "▶");
    assert!(h.state_mut().launch_mut().is_open());
    h.get_by_role_and_label(Role::CheckBox, &tr("launch.select_all"));
    click(&mut h, Role::Button, "▶");
    assert!(h.state_mut().launch_mut().is_open(), "▶ does not toggle");

    h.state_mut().launch_mut().close();
    run(&mut h);
    assert!(!has_label(&h, &tr("launch.select_all")));
    click(&mut h, Role::Button, &center_button());
    assert!(h.state_mut().launch_mut().is_open());
}

/// 開窗、按批次啟動 Alice 與 Carol。
fn launch_two(auto_close: bool) -> (Harness<'static, App>, Actions, UiPoster) {
    let (mut h, actions, poster) = overlay("waiting_game", three(), &["a", "c"]);
    h.state_mut().launch_mut().set_auto_close(auto_close);
    click(&mut h, Role::Button, "▶");
    click(&mut h, Role::Button, &open_count(2));
    assert_eq!(*actions.borrow(), [LaunchAction::Launch(ids(&["a", "c"]))]);
    (h, actions, poster)
}

#[test]
fn the_window_closes_itself_once_every_account_opened() {
    let (mut h, _, poster) = launch_two(true);
    status(&poster, "a", LaunchStatus::Opened);
    run(&mut h);
    assert!(
        h.state_mut().launch_mut().is_open(),
        "Carol is still signing in"
    );
    status(&poster, "c", LaunchStatus::SkippedRunning);
    run(&mut h);
    assert!(!h.state_mut().launch_mut().is_open());
}

#[test]
fn a_failure_keeps_the_window_open_with_the_error() {
    let (mut h, _, poster) = launch_two(true);
    status(&poster, "a", LaunchStatus::Opened);
    status(&poster, "c", rejected("Invalid password"));
    run(&mut h);
    assert!(h.state_mut().launch_mut().is_open());
    h.get_by_label("Invalid password");
}

#[test]
fn with_auto_close_off_the_window_stays_open() {
    let (mut h, _, poster) = launch_two(false);
    status(&poster, "a", LaunchStatus::Opened);
    status(&poster, "c", LaunchStatus::Opened);
    run(&mut h);
    assert!(h.state_mut().launch_mut().is_open());
}

#[test]
fn launch_progress_and_running_scans_reach_the_window() {
    let (mut h, _, poster) = overlay("waiting_game", three(), &["a"]);
    status(&poster, "a", LaunchStatus::Opened);
    poster.post(UiEvent::RunningAccounts(ids(&["carol"])));
    click(&mut h, Role::Button, &center_button());
    h.get_by_label(&tr("launch.opened"));
    let carol = h.get_by_role_and_label(Role::CheckBox, "Carol");
    assert!(carol.accesskit_node().is_disabled(), "Carol is running");
}

#[test]
fn the_open_window_rescans_running_accounts_every_few_seconds_not_every_frame() {
    let (mut h, _, _) = overlay("waiting_game", three(), &[]);
    let scans = Rc::new(Cell::new(0));
    let counter = Rc::clone(&scans);
    h.state_mut()
        .set_on_scan_running(move || counter.set(counter.get() + 1));
    h.run_steps(5);
    assert_eq!(scans.get(), 0, "a closed window does not scan");
    // kittest 每步前進 0.25 秒
    click(&mut h, Role::Button, &center_button());
    h.run_steps(4);
    assert_eq!(scans.get(), 1, "one scan on open");
    h.run_steps(8);
    assert_eq!(scans.get(), 2, "another after the interval");
    h.state_mut().launch_mut().close();
    run(&mut h);
    click(&mut h, Role::Button, &center_button());
    assert_eq!(scans.get(), 3, "reopening scans again");
}

#[test]
fn the_close_button_closes_the_window() {
    let mut host = LaunchHost::default();
    host.set_window_ops(WindowOps::offscreen());
    host.set_accounts(three(), &[]);
    host.open();
    let mut frames = Frames::new();
    for _ in 0..3 {
        frames.run(false, |ctx| drop(host.show(ctx, &[])));
    }
    assert!(host.is_open());
    frames.run(true, |ctx| drop(host.show(ctx, &[])));
    assert!(!host.is_open());
}
