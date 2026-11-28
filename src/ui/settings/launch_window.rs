//! 啟動遊戲帳號視窗的畫面：上方「全選」、中間可捲動的帳號列（勾選、標記、單獨〔啟動〕與進度），底部自動關窗選項與批次啟動。

use eframe::egui::accesskit::Role;
use eframe::egui::{Checkbox, Color32, Frame, Id, Margin, Panel, Ui, WidgetInfo};

use super::account_dialog::kind_label;
use super::launch_list::{LaunchList, SelectAll};
use super::patch_card::{PatchBoard, board_card};
use super::release_card::{FOOTER_MARGIN, HEADER_MARGIN, LIST_PADDING};
use super::theme;
use super::widgets::{
    hint, list_card, primary_button, right_aligned, scroll_page, secondary_button,
};
use crate::i18n::t;
use crate::launcher::Account;
use crate::launcher::install::InstallSource;
use crate::launcher::launch::LaunchError;
use crate::launcher::login::LoginError;
use crate::launcher::orchestrate::{Failure, LaunchStatus};
use crate::launcher::steam::SteamError;
use crate::logs::log;

/// 視窗要 App 做的事。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchAction {
    /// 啟動這些帳號（id）；執行中與等檢查、登入中的已排除。
    Launch(Vec<String>),
    /// 勾選改了：要記住的帳號 id（寫回 `launch_selection`）。
    SelectionChanged(Vec<String>),
    /// 「啟動後自動關閉」改了：立刻寫回設定。
    AutoCloseChanged(bool),
    /// 沒有帳號時按了「前往新增帳號」：開設定視窗的遊戲帳號頁。
    AddAccount,
}

/// 單一帳號啟動進度的顯示文字；伺服器拒絕登入時直接是伺服器原文。
pub fn status_text(status: &LaunchStatus) -> String {
    match status {
        LaunchStatus::Waiting(InstallSource::Official) => t("launch.waiting_update", &[]),
        LaunchStatus::Waiting(InstallSource::Steam) => t("launch.waiting_check", &[]),
        LaunchStatus::LoggingIn => t("launch.logging_in", &[]),
        LaunchStatus::Opened => t("launch.opened", &[]),
        LaunchStatus::SkippedRunning => t("launch.already_running", &[]),
        LaunchStatus::Failed(failure) => failure_text(failure),
    }
}

/// 啟動失敗的顯示文字。
pub fn failure_text(failure: &Failure) -> String {
    let with_error = |key, error: &str| t(key, &[("error", error)]);
    match failure {
        Failure::Password => t("launch.failed_password", &[]),
        Failure::Steam(SteamError::NotRunning) => t("launch.failed_steam_not_running", &[]),
        Failure::Steam(SteamError::InterfaceUnavailable) => {
            t("launch.failed_steam_unavailable", &[])
        }
        Failure::Steam(SteamError::InitFailed(code)) => {
            t("launch.failed_steam_init", &[("code", &code.to_string())])
        }
        Failure::Steam(SteamError::NoSession) => t("launch.failed_steam_no_session", &[]),
        Failure::Steam(SteamError::Timeout) => t("launch.failed_steam_timeout", &[]),
        Failure::Login(LoginError::Connect(e)) => with_error("launch.failed_connect", e),
        Failure::Login(LoginError::Rejected { raw }) => raw.clone(),
        Failure::Login(LoginError::Protocol(e)) => with_error("launch.failed_protocol", e),
        Failure::Launch(LaunchError::ClientMissing) => t("launch.failed_client_missing", &[]),
        Failure::Launch(LaunchError::Spawn(e)) => with_error("launch.failed_spawn", e),
        Failure::Patch => t("launch.not_started", &[]),
    }
}

/// 啟動進度文字的顏色：已啟動綠、失敗紅（檢查沒通過除外，錯誤已顯示在安裝狀態）、其餘說明色。
pub(crate) fn status_color(status: &LaunchStatus) -> Color32 {
    match status {
        LaunchStatus::Opened => theme::OK,
        LaunchStatus::Failed(Failure::Patch) => theme::TEXT_SECONDARY,
        LaunchStatus::Failed(_) => theme::ERROR,
        LaunchStatus::Waiting(_) | LaunchStatus::LoggingIn | LaunchStatus::SkippedRunning => {
            theme::TEXT_SECONDARY
        }
    }
}

/// 帳號卡片下方的啟動進度；貼近上面那張卡片，才看得出是誰的。
pub(super) fn status_line(ui: &mut Ui, status: &LaunchStatus) {
    ui.add_space(4.0 - ui.spacing().item_spacing.y);
    hint(ui, &status_text(status), status_color(status));
}

/// 帳號列的標記：類型在前、位置固定；窄視窗截掉的是後面的執行中，〔啟動〕停用仍看得出。
pub(super) fn account_badges(account: &Account, running: bool) -> Vec<String> {
    let mut badges = vec![kind_label(account.kind)];
    badges.extend(running.then(|| t("accounts.running", &[])));
    badges
}

/// 畫一幀；`patch` 是各安裝的更新狀態，`auto_close` 是「啟動後自動關閉」的勾選，改了就回 `AutoCloseChanged`。
pub fn show(
    ui: &mut Ui,
    list: &mut LaunchList,
    patch: &PatchBoard,
    auto_close: &mut bool,
) -> Vec<LaunchAction> {
    let mut actions = Vec::new();
    if list.accounts().is_empty() {
        no_accounts(ui, &mut actions);
        return actions;
    }
    Panel::bottom("launch_window_footer")
        .frame(
            Frame::new()
                .fill(theme::MODAL_FOOT)
                .inner_margin(FOOTER_MARGIN),
        )
        .show(ui, |ui| footer(ui, list, auto_close, &mut actions));
    Panel::top("launch_window_header")
        .frame(Frame::new().inner_margin(HEADER_MARGIN))
        .show(ui, |ui| header(ui, list, patch, &mut actions));
    let margin = Margin::symmetric(20, LIST_PADDING);
    scroll_page(ui, "launch_window_list", margin, theme::CARD_GAP, |ui| {
        for account in list.accounts().to_vec() {
            row(ui, list, &account, &mut actions);
        }
    });
    actions
}

fn no_accounts(ui: &mut Ui, actions: &mut Vec<LaunchAction>) {
    Frame::new().inner_margin(Margin::same(20)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        let top = ui.available_height() * 0.3;
        ui.vertical_centered(|ui| {
            ui.add_space(top);
            hint(ui, &t("launch.no_accounts", &[]), theme::TEXT_SECONDARY);
            if primary_button(ui, &t("launch.go_add_account", &[])).clicked() {
                log("[ui] launch window: add account clicked; opening the settings accounts page");
                actions.push(LaunchAction::AddAccount);
            }
        });
    });
}

fn header(ui: &mut Ui, list: &mut LaunchList, patch: &PatchBoard, actions: &mut Vec<LaunchAction>) {
    let all = list.select_all_state();
    let any = list.accounts().iter().any(|a| list.checkable(a));
    let mut on = all == SelectAll::All;
    let check =
        Checkbox::new(&mut on, t("launch.select_all", &[])).indeterminate(all == SelectAll::Some);
    if ui.add_enabled(any, check).changed() {
        list.toggle_all();
        actions.push(selection_changed(list));
    }
    if !patch.is_empty() {
        ui.add_space(12.0 - ui.spacing().item_spacing.y);
        board_card(ui, patch);
    }
}

fn row(ui: &mut Ui, list: &mut LaunchList, account: &Account, actions: &mut Vec<LaunchAction>) {
    let running = list.is_running(account);
    let enabled = list.checkable(account);
    let mut on = list.is_checked(&account.id);
    let mut toggled = false;
    let leading = |ui: &mut Ui| {
        let response = ui.add_enabled(enabled, Checkbox::without_text(&mut on));
        // 名稱畫在卡片的名稱列，勾選框自己沒有字，無障礙名稱要另外給
        response.widget_info(|| WidgetInfo::selected(Role::CheckBox, enabled, on, &account.name));
        toggled = response.changed();
    };
    let launch = list_card(
        ui,
        Id::unique(("launch_window_row", &account.id)),
        leading,
        &account.name,
        &account_badges(account, running),
        "",
        |ui| {
            ui.add_enabled_ui(enabled, |ui| secondary_button(ui, &t("button.launch", &[])))
                .inner
                .clicked()
        },
    );
    if let Some(status) = list.status(&account.id) {
        status_line(ui, status);
    }
    if toggled && list.set_checked(&account.id, on) {
        actions.push(selection_changed(list));
    }
    if launch && let Some(id) = list.begin_launch_one(&account.id) {
        log(format!(
            "[launch] launch requested from launch window: {id}"
        ));
        actions.push(LaunchAction::Launch(vec![id]));
    }
}

fn footer(
    ui: &mut Ui,
    list: &mut LaunchList,
    auto_close: &mut bool,
    actions: &mut Vec<LaunchAction>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if ui
            .checkbox(auto_close, t("launch.auto_close", &[]))
            .changed()
        {
            log(format!("[ui] launch auto-close set to {auto_close}"));
            actions.push(LaunchAction::AutoCloseChanged(*auto_close));
        }
        right_aligned(ui, Id::unique("launch_window_buttons"), |ui| {
            let count = list.open_count();
            let text = t("launch.open_count", &[("count", &count.to_string())]);
            let open = ui.add_enabled_ui(count > 0, |ui| primary_button(ui, &text));
            if open.inner.clicked() {
                let ids = list.begin_launch();
                log(format!(
                    "[launch] launch requested from launch window: {} account(s) {ids:?}",
                    ids.len()
                ));
                actions.push(LaunchAction::Launch(ids));
            }
        });
    });
}

fn selection_changed(list: &LaunchList) -> LaunchAction {
    let ids = list.selection();
    log(format!(
        "[ui] launch selection changed: {} checked",
        ids.len()
    ));
    LaunchAction::SelectionChanged(ids)
}
