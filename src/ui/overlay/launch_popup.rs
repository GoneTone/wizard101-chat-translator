//! overlay 的開啟遊戲入口：等待遊戲時內容區中央的大按鈕，以及它與標題列 ▶ 共用的帳號勾選選單。

use std::sync::Arc;

use eframe::egui::{
    Align, Align2, Area, Checkbox, Color32, FontId, Galley, Id, Label, Layout, Order, Pos2, Rect,
    Sense, Stroke, StrokeKind, TextStyle, TextWrapMode, Ui, UiBuilder, Vec2, WidgetText,
};

use crate::i18n::t;
use crate::launcher::Account;
use crate::launcher::launch::LaunchError;
use crate::launcher::login::LoginError;
use crate::launcher::orchestrate::{Failure, LaunchStatus};
use crate::launcher::steam::SteamError;
use crate::logs::log;
use crate::ui::geometry::EDGE;
use crate::ui::overlay::frame::GRIP_SIZE;
use crate::ui::overlay::launch_menu::{Anchor, MenuState, SelectAll, menu_rect};
use crate::ui::overlay::scrollbar::{self, Scrollbar};
use crate::ui::palette::{BAR, GRIP};
use crate::ui::settings::theme::{self, icon};
use crate::ui::settings::widgets::{
    pill, primary_button, primary_leading_icon_button, secondary_button,
};
use crate::ui::text_select::wheel_delta;

/// 選單想要的寬度（DIP）；內容區更窄時縮到放得下。
const MENU_WIDTH: f32 = 280.0;
const PAD: f32 = 8.0;
const GAP: f32 = 6.0;
/// 中央按鈕與上方狀態大字的距離。
const BUTTON_GAP: f32 = 8.0;

/// 選單要 App 做的事。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// 開啟這些帳號（id）；執行中與登入中的已排除。
    Launch(Vec<String>),
    /// 勾選改了：要記住的帳號 id（寫回 `launch_selection`）。
    SelectionChanged(Vec<String>),
    /// 沒有帳號時按了「開啟設定」。
    OpenSettings,
}

/// 單一帳號開啟進度的顯示文字；伺服器拒絕登入時直接是伺服器原文。
pub fn status_text(status: &LaunchStatus) -> String {
    match status {
        LaunchStatus::LoggingIn => t("launch.logging_in", &[]),
        LaunchStatus::Opened => t("launch.opened", &[]),
        LaunchStatus::SkippedRunning => t("launch.already_running", &[]),
        LaunchStatus::Failed(failure) => failure_text(failure),
    }
}

/// 開啟失敗的顯示文字。
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
        Failure::Launch(LaunchError::NoSteamInstall) => t("launch.failed_no_steam_install", &[]),
        Failure::Launch(LaunchError::ClientMissing) => t("launch.failed_client_missing", &[]),
        Failure::Launch(LaunchError::Spawn(e)) => with_error("launch.failed_spawn", e),
    }
}

/// 開啟進度文字的顏色：開好綠、失敗紅、其餘說明色。
pub(crate) fn status_color(status: &LaunchStatus) -> Color32 {
    match status {
        LaunchStatus::Opened => theme::OK,
        LaunchStatus::Failed(_) => theme::ERROR,
        LaunchStatus::LoggingIn | LaunchStatus::SkippedRunning => theme::TEXT_SECONDARY,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Opener {
    Button,
    TitleBar,
}

/// 選單的三種樣子。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Variant {
    Full,
    /// 內容區放不下「全選＋2 列＋按鈕」。
    Compact,
    NoAccounts,
}

/// 開啟遊戲的中央大按鈕與帳號選單；每幀由 [`LaunchMenu::show`] 畫出。
#[derive(Default)]
pub struct LaunchMenu {
    state: MenuState,
    open: Option<Opener>,
    variant: Option<Variant>,
    /// 這一幀中央按鈕的範圍；沒顯示是 `None`。
    button: Option<Rect>,
    /// 上一幀選單的範圍，判斷按在選單外用。
    rect: Option<Rect>,
    scroll: f32,
    scrollbar: Scrollbar,
}

impl LaunchMenu {
    /// 換上帳號清單與上次的勾選（不存在的 id 丟掉）；執行中名單與進度保留。
    pub fn set_accounts(&mut self, accounts: Vec<Account>, selection: &[String]) {
        self.state.set_accounts(accounts, selection);
    }

    /// 換上執行中的帳號名稱；執行中的帳號停用勾選、不計入開啟數。
    pub fn set_running(&mut self, names: Vec<String>) {
        self.state.set_running(names);
    }

    /// 某帳號的開啟進度有變。
    pub fn set_launch_status(&mut self, id: &str, status: LaunchStatus) {
        self.state.set_status(id, status);
    }

    /// 選單是否開著（開著時執行中名單要保持新鮮）。
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// 標題列 ▶：開著就關，否則貼著標題列打開。
    pub fn toggle(&mut self) {
        self.toggle_from(Opener::TitleBar);
    }

    /// 收起選單；原本開著才記 log。
    pub fn close(&mut self) {
        self.rect = None;
        if self.open.take().is_some() {
            log("[ui] launch menu closed");
        }
    }

    fn toggle_from(&mut self, opener: Opener) {
        if self.open == Some(opener) {
            self.close();
            return;
        }
        self.open = Some(opener);
        self.scroll = 0.0;
        log(format!(
            "[ui] launch menu opened from={opener:?} accounts={} launchable={}",
            self.state.accounts().len(),
            self.state.open_count()
        ));
    }

    /// 畫出中央按鈕（`waiting` 是按鈕要掛在其下的點，沒在等遊戲時為 `None`）與開著的選單。
    ///
    /// `body` 是標題列以下的整塊內容區；選單最多佔滿它，左右與底部讓出縮放帶與抓取紋。
    pub fn show(&mut self, ui: &Ui, body: Rect, waiting: Option<Pos2>) -> Option<MenuAction> {
        let avail = Rect::from_min_max(
            Pos2::new(body.min.x + EDGE as f32, body.min.y),
            Pos2::new(body.max.x - EDGE as f32, body.max.y - GRIP_SIZE),
        );
        let clicked = waiting.is_some_and(|at| self.center_button(ui, at, avail));
        if waiting.is_none() {
            self.button = None;
            // 遊戲開起來後中央按鈕消失，選單改掛標題列，進度照樣看得到
            if self.open == Some(Opener::Button) {
                self.open = Some(Opener::TitleBar);
            }
        }
        let pressed = ui.input(|i| {
            let p = &i.pointer;
            p.primary_pressed().then(|| p.press_origin()).flatten()
        });
        // 標題列上的按壓不收：▶ 自己會切換，拖曳視窗時選單留著
        if let (Some(p), Some(menu)) = (pressed, self.rect)
            && !menu.contains(p)
            && p.y >= body.min.y
            && !self.button.is_some_and(|b| b.contains(p))
        {
            self.close();
        }
        if clicked {
            self.toggle_from(Opener::Button);
        }
        let opener = self.open?;
        let anchor = match (opener, self.button) {
            (Opener::Button, Some(button)) => Anchor::Button(button),
            _ => Anchor::TitleBar,
        };
        let action = self.menu(ui, anchor, avail);
        if action == Some(MenuAction::OpenSettings) {
            self.close();
        }
        action
    }

    fn center_button(&mut self, ui: &Ui, at: Pos2, avail: Rect) -> bool {
        let top = (at.y + BUTTON_GAP)
            .min(avail.max.y - theme::CONTROL_HEIGHT)
            .max(avail.min.y);
        let response = Area::new(Id::unique("launch_button"))
            .order(Order::Middle)
            .constrain(false)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(Pos2::new(at.x, top))
            .show(ui.ctx(), |ui| {
                theme::apply(ui.style_mut());
                primary_leading_icon_button(ui, icon::PLAY, &t("launch.open_game", &[]))
            })
            .inner;
        self.button = Some(response.rect);
        response.clicked()
    }

    fn menu(&mut self, ui: &Ui, anchor: Anchor, avail: Rect) -> Option<MenuAction> {
        let width = MENU_WIDTH.min(avail.width());
        let inner_w = width - 2.0 * PAD;
        let painter = ui.painter();
        let hint = |key: &str, args: &[(&str, &str)]| {
            let font = FontId::proportional(theme::HINT_SIZE);
            painter.layout(t(key, args), font, theme::TEXT_SECONDARY, inner_w)
        };
        let line = theme::CONTROL_HEIGHT;
        let accounts = self.state.accounts().to_vec();
        // 名稱前的勾選框寬度：進度字對齊名稱
        let indent = ui.spacing().icon_width + ui.spacing().icon_spacing;
        let statuses: Vec<Option<Arc<Galley>>> = accounts
            .iter()
            .map(|a| {
                self.state.status(&a.id).map(|s| {
                    let font = FontId::proportional(theme::HINT_SIZE);
                    let wrap = inner_w - indent - scrollbar::WIDTH;
                    painter.layout(status_text(s), font, status_color(s), wrap)
                })
            })
            .collect();
        let footer = line;
        let rows: Vec<f32> = statuses
            .iter()
            .map(|s| line + s.as_ref().map_or(0.0, |g| g.size().y + GAP / 2.0))
            .collect();
        let frame = |list: f32| 2.0 * PAD + line + GAP + list + GAP + footer;

        let taller = hint("launch.taller_hint", &[]);
        let empty = hint("launch.no_accounts", &[]);
        let (variant, height) = if accounts.is_empty() {
            (Variant::NoAccounts, 2.0 * PAD + empty.size().y + GAP + line)
        } else if avail.height() < frame(2.0 * line) {
            (Variant::Compact, 2.0 * PAD + line + GAP + taller.size().y)
        } else {
            (Variant::Full, frame(rows.iter().sum()))
        };
        if self.variant != Some(variant) {
            self.variant = Some(variant);
            log(format!(
                "[ui] launch menu layout={variant:?} content_h={:.0} full_min_h={:.0}",
                avail.height(),
                frame(2.0 * line)
            ));
        }
        let rect = menu_rect(anchor, Vec2::new(width, height), avail);
        self.rect = Some(rect);

        Area::new(Id::unique("launch_menu"))
            .order(Order::Foreground)
            .constrain(false)
            .fixed_pos(rect.min)
            .show(ui.ctx(), |ui| {
                theme::apply(ui.style_mut());
                ui.set_clip_rect(rect);
                let (frame, _) = ui.allocate_exact_size(rect.size(), Sense::click());
                ui.painter()
                    .rect(frame, 0.0, BAR, Stroke::new(1.0, GRIP), StrokeKind::Inside);
                let inner = frame.shrink(PAD);
                match variant {
                    Variant::NoAccounts => no_accounts(ui, inner, empty),
                    Variant::Compact => self.compact(ui, inner, taller),
                    Variant::Full => self.full(ui, inner, &accounts, statuses, &rows),
                }
            })
            .inner
    }

    fn full(
        &mut self,
        ui: &mut Ui,
        inner: Rect,
        accounts: &[Account],
        statuses: Vec<Option<Arc<Galley>>>,
        rows: &[f32],
    ) -> Option<MenuAction> {
        let line = theme::CONTROL_HEIGHT;
        let mut action = None;

        let header = Rect::from_min_size(inner.min, Vec2::new(inner.width(), line));
        let all = self.state.select_all_state();
        let any = accounts.iter().any(|a| self.state.checkable(a));
        let mut on = all == SelectAll::All;
        let check = Checkbox::new(&mut on, t("launch.select_all", &[]))
            .indeterminate(all == SelectAll::Some);
        if put_left(ui, header, |ui| ui.add_enabled(any, check)).changed() {
            self.state.toggle_all();
            action = Some(self.selection_changed());
        }

        let button = Rect::from_min_max(Pos2::new(inner.min.x, inner.max.y - line), inner.max);
        let list_bottom = button.min.y - GAP;
        let list = Rect::from_min_max(
            Pos2::new(inner.min.x, header.max.y + GAP),
            Pos2::new(inner.max.x, list_bottom),
        );
        if self.rows(ui, list, accounts, statuses, rows) {
            action = Some(self.selection_changed());
        }

        let count = self.state.open_count();
        let text = t("launch.open_count", &[("count", &count.to_string())]);
        let open = ui.scope_builder(
            UiBuilder::new()
                .max_rect(button)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                ui.add_enabled_ui(count > 0, |ui| primary_button(ui, &text))
                    .inner
            },
        );
        if open.inner.clicked() {
            action = Some(self.launch());
        }
        action
    }

    /// 中間可捲動的帳號列；有勾選變動回 `true`。
    fn rows(
        &mut self,
        ui: &mut Ui,
        list: Rect,
        accounts: &[Account],
        statuses: Vec<Option<Arc<Galley>>>,
        heights: &[f32],
    ) -> bool {
        let content: f32 = heights.iter().sum();
        let track = Rect::from_min_max(
            Pos2::new(list.max.x - scrollbar::WIDTH, list.min.y),
            list.max,
        );
        if ui.rect_contains_pointer(list) {
            self.scroll -= wheel_delta(ui, list.height());
        }
        if let Some(fraction) = self.scrollbar.drag(ui, track) {
            self.scroll = fraction as f32 * content;
        }
        self.scroll = self.scroll.clamp(0.0, (content - list.height()).max(0.0));

        let mut rows = ui.new_child(UiBuilder::new().max_rect(list));
        rows.set_clip_rect(list);
        let width = list.width() - scrollbar::WIDTH;
        let mut top = list.min.y - self.scroll;
        let mut changed = false;
        for ((account, status), height) in accounts.iter().zip(statuses).zip(heights) {
            let row = Rect::from_min_size(Pos2::new(list.min.x, top), Vec2::new(width, *height));
            top += height;
            if row.intersects(list) {
                changed |= self.row(&mut rows, row, account, status);
            }
        }
        if content > 0.0 {
            let first = f64::from(self.scroll / content);
            let last = f64::from((self.scroll + list.height()) / content);
            self.scrollbar.paint(ui, track, first, last);
        }
        changed
    }

    fn row(
        &mut self,
        ui: &mut Ui,
        row: Rect,
        account: &Account,
        status: Option<Arc<Galley>>,
    ) -> bool {
        let line = Rect::from_min_size(row.min, Vec2::new(row.width(), theme::CONTROL_HEIGHT));
        let running = self.state.is_running(account);
        let enabled = self.state.checkable(account);
        let mut on = self.state.is_checked(&account.id);
        let indent = ui.spacing().icon_width + ui.spacing().icon_spacing;
        let changed = ui
            .scope_builder(
                UiBuilder::new()
                    .max_rect(line)
                    .layout(Layout::right_to_left(Align::Center)),
                |ui| {
                    if running {
                        pill(ui, &t("accounts.running", &[]));
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let room = ui.available_width() - indent;
                        let name = WidgetText::from(account.name.as_str()).into_galley(
                            ui,
                            Some(TextWrapMode::Truncate),
                            room,
                            TextStyle::Button,
                        );
                        ui.add_enabled(enabled, Checkbox::new(&mut on, name))
                            .changed()
                    })
                    .inner
                },
            )
            .inner;
        if let Some(status) = status {
            let at = Pos2::new(row.min.x + indent, line.max.y);
            ui.put(
                Rect::from_min_size(at, status.size()),
                Label::new(status).selectable(false),
            );
        }
        changed && self.state.set_checked(&account.id, on)
    }

    fn compact(&mut self, ui: &mut Ui, inner: Rect, hint: Arc<Galley>) -> Option<MenuAction> {
        let line = theme::CONTROL_HEIGHT;
        let button = Rect::from_min_size(inner.min, Vec2::new(inner.width(), line));
        let count = self.state.open_count();
        let text = t("launch.open_last", &[("count", &count.to_string())]);
        let open = put_left(ui, button, |ui| {
            ui.add_enabled_ui(count > 0, |ui| primary_button(ui, &text))
                .inner
        });
        put_hint(ui, Pos2::new(inner.min.x, button.max.y + GAP), hint, inner);
        open.clicked().then(|| self.launch())
    }

    fn launch(&mut self) -> MenuAction {
        let ids = self.state.begin_launch();
        log(format!(
            "[launch] launch requested from overlay: {} account(s) {ids:?}",
            ids.len()
        ));
        MenuAction::Launch(ids)
    }

    fn selection_changed(&self) -> MenuAction {
        let ids = self.state.selection();
        log(format!(
            "[ui] launch selection changed: {} checked",
            ids.len()
        ));
        MenuAction::SelectionChanged(ids)
    }
}

fn no_accounts(ui: &mut Ui, inner: Rect, hint: Arc<Galley>) -> Option<MenuAction> {
    let below = hint.size().y + GAP;
    let top = if put_hint(
        ui,
        inner.min,
        hint,
        inner.with_max_y(inner.max.y - theme::CONTROL_HEIGHT - GAP),
    ) {
        inner.min.y + below
    } else {
        inner.min.y
    };
    let button = Rect::from_min_size(
        Pos2::new(inner.min.x, top),
        Vec2::new(inner.width(), theme::CONTROL_HEIGHT),
    );
    let open = put_left(ui, button, |ui| {
        secondary_button(ui, &t("launch.open_settings", &[]))
    });
    open.clicked().then_some(MenuAction::OpenSettings)
}

/// 提示整段放得進 `room` 才畫在 `at`：只露半行比不畫難看，選單外也不能多佔位置（Area 會被推開）。
fn put_hint(ui: &mut Ui, at: Pos2, hint: Arc<Galley>, room: Rect) -> bool {
    let rect = Rect::from_min_size(at, hint.size());
    let fits = rect.max.y <= room.max.y;
    if fits {
        ui.put(rect, Label::new(hint).selectable(false));
    }
    fits
}

/// 在 `rect` 內由左往右、垂直置中排 `add`。
fn put_left<R>(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
        add,
    )
    .inner
}
