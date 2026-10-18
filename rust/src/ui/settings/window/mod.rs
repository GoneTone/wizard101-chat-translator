//! 設定視窗的內容：側欄四個分頁、各頁可捲動的內容與固定的底部按鈕列；視窗本身由 App 托管。
//!
//! 介面語言改了就先預覽（呼叫 `set_language`），按儲存才寫進設定，取消則還原成開窗時的語言。

mod about;
mod advanced;
mod basic;

use eframe::egui::style::ScrollStyle;
use eframe::egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Label, Layout, Margin, Rect, RichText,
    Role, ScrollArea, Sense, Stroke, Ui, UiBuilder, WidgetInfo, pos2, vec2,
};
use serde_json::Value;

use super::env::{SettingsEnv, SettingsEvent};
use super::hotkeys::{HotkeyEditor, HotkeyKind};
use super::service_list::{ServiceDraft, ServicePane};
use super::theme::{self, icon};
use super::widgets::{
    callout, focus_ring, primary_button, right_aligned, right_aligned_width, secondary_button,
};
use super::{flag_setting, setting, text_setting};
use crate::config::services::{ServiceSlot, find, validate_service};
use crate::config::{AdvancedText, Config, default_config, parse_advanced, py};
use crate::i18n::{current_language, set_language, t};
use crate::logs::log;
use crate::py::pystr;
use crate::runtime::updater::Release;
use crate::ui::overlay::scrollbar;

/// 設定視窗的分頁，依側欄順序。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Basic,
    Services,
    Advanced,
    About,
}

impl Page {
    const ALL: [Page; 4] = [Self::Basic, Self::Services, Self::Advanced, Self::About];

    fn title_key(self) -> &'static str {
        match self {
            Self::Basic => "settings.tab.basic",
            Self::Services => "settings.tab.services",
            Self::Advanced => "settings.tab.advanced",
            Self::About => "settings.tab.about",
        }
    }

    fn icon(self) -> char {
        match self {
            Self::Basic => icon::BASIC,
            Self::Services => icon::SERVICES,
            Self::Advanced => icon::ADVANCED,
            Self::About => icon::ABOUT,
        }
    }
}

/// 設定視窗要 App 做的事。
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsAction {
    /// 預覽 overlay 與泡泡的透明度（尚未儲存）。
    PreviewAlpha(f32),
    /// 視窗已呼叫 `set_language`，App 只需 relabel 常駐介面。
    PreviewLanguage,
    /// 手動檢查查到新版。
    UpdateFound(Release),
    /// 這個視窗管的鍵（服務三鍵、目標語言、熱鍵與開關、進階數值、`ui_language`）；不是完整設定，
    /// App 要併進目前共用的設定（`cfg.extend`），其他鍵（如 overlay 位置）不可被開窗時的舊值蓋掉。視窗該關了。
    Save(Config),
    /// 取消；預覽中的透明度與語言已在這之前的動作還原，視窗該關了。
    Cancel,
}

/// 設定視窗的內容與編輯中的草稿；`ui` 每幀畫一次並回傳這一幀要 App 做的事。
pub struct SettingsWindow {
    /// 開窗時的 `service_slots`；寫回時保留其中認不得的鍵。
    slots_at_open: Option<Value>,
    page: Page,
    language_at_open: String,
    alpha_at_open: f64,
    target_language: String,
    hotkeys: HotkeyEditor,
    auto_show_input: bool,
    paste_hotkey: bool,
    services: ServicePane,
    advanced: AdvancedText,
    translate_system: bool,
    /// 上次按儲存被擋下的原因（錯誤文案 key）；顯示後隨編輯即時更新。
    errors: Vec<String>,
    about: about::AboutState,
    pending: Vec<SettingsAction>,
    /// 下一幀要呼叫 `env.cancel_recording()`（收下錄製結果或換頁時手上沒有 env）。
    release_recorder: bool,
}

impl SettingsWindow {
    /// 從 `cfg` 取一份草稿，記下開窗時的介面語言與透明度。
    pub fn new(cfg: &Config) -> Self {
        let alpha = py::number(&setting(cfg, "overlay_alpha"))
            .or_else(|| py::number(&default_config()["overlay_alpha"]))
            .unwrap_or_default();
        let number = |key: &str| py::display(&setting(cfg, key));
        Self {
            slots_at_open: cfg.get("service_slots").cloned(),
            page: Page::Basic,
            language_at_open: current_language(),
            alpha_at_open: alpha,
            target_language: text_setting(cfg, "target_language"),
            hotkeys: HotkeyEditor::new(
                &text_setting(cfg, HotkeyKind::Input.config_key()),
                &text_setting(cfg, HotkeyKind::Region.config_key()),
            ),
            auto_show_input: flag_setting(cfg, "auto_show_input"),
            paste_hotkey: flag_setting(cfg, "paste_hotkey"),
            services: ServicePane::new(ServiceDraft::from_config(cfg)),
            advanced: AdvancedText {
                fade_seconds: number("fade_seconds"),
                max_messages: number("max_messages"),
                max_parallel_translations: number("max_parallel_translations"),
                type_delay: number("type_delay"),
                overlay_alpha: alpha,
            },
            translate_system: flag_setting(cfg, "translate_system_messages"),
            errors: Vec::new(),
            about: about::AboutState::default(),
            pending: Vec::new(),
            release_recorder: false,
        }
    }

    pub fn page(&self) -> Page {
        self.page
    }

    /// 切到 `page`；正在錄熱鍵就放棄錄製。
    pub fn set_page(&mut self, page: Page) {
        if page != self.page {
            self.release_recorder |= self.hotkeys.stop();
            self.page = page;
        }
    }

    fn switch_page(&mut self, page: Page, env: &dyn SettingsEnv) {
        self.set_page(page);
        self.release_recorder(env);
    }

    fn release_recorder(&mut self, env: &dyn SettingsEnv) {
        if std::mem::take(&mut self.release_recorder) {
            env.cancel_recording();
        }
    }

    /// 背景工作的結果：測試連線與模型清單轉給服務分頁，其餘由視窗自己收。
    pub fn apply(&mut self, event: &SettingsEvent) {
        match event {
            SettingsEvent::TestDone { .. } | SettingsEvent::ModelsDone { .. } => {
                self.services.apply(event);
            }
            SettingsEvent::UpdateChecked { round, result } => self.apply_update(*round, result),
            SettingsEvent::HotkeyRecorded { .. } => {
                self.release_recorder |= self.hotkeys.apply(event);
            }
        }
    }

    /// 畫整個視窗內容；回傳這一幀要 App 做的事。
    pub fn ui(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) -> Vec<SettingsAction> {
        self.release_recorder(env);
        let full = ui.available_rect_before_wrap();
        let (side, content) = full.split_left_right_at_x(full.left() + theme::SIDEBAR_WIDTH);
        ui.painter().rect_filled(side, 0, theme::SIDEBAR_BG);
        ui.painter().rect_filled(content, 0, theme::WINDOW_BG);
        let picked = ui
            .scope_builder(UiBuilder::new().max_rect(side), |ui| self.sidebar(ui))
            .inner;
        if let Some(page) = picked {
            self.switch_page(page, env);
        }
        self.content(ui, env, content);
        ui.advance_cursor_after_rect(full);
        std::mem::take(&mut self.pending)
    }

    /// 取消（〔取消〕、視窗 ✕ 或 Alt+F4）：停掉錄製，還原預覽中的透明度與介面語言，最後送 `Cancel`。
    pub fn cancel(&mut self, env: &dyn SettingsEnv) -> Vec<SettingsAction> {
        self.release_recorder |= self.hotkeys.stop();
        self.release_recorder(env);
        let mut actions = std::mem::take(&mut self.pending);
        if self.advanced.overlay_alpha != self.alpha_at_open {
            self.advanced.overlay_alpha = self.alpha_at_open;
            actions.push(SettingsAction::PreviewAlpha(self.alpha_at_open as f32));
        }
        if current_language() != self.language_at_open {
            log(format!(
                "[ui] settings language preview reverted to {}",
                self.language_at_open
            ));
            set_language(&self.language_at_open);
            actions.push(SettingsAction::PreviewLanguage);
        }
        actions.push(SettingsAction::Cancel);
        actions
    }

    /// 介面語言換了：立刻以新語言預覽，不寫設定（按儲存才算數）。
    fn preview_language(&mut self, code: &str) {
        log(format!("[ui] settings previewing language {code}"));
        set_language(code);
        self.pending.push(SettingsAction::PreviewLanguage);
    }

    /// 草稿寫成這個視窗管的那些鍵，連同擋下儲存的原因（錯誤文案 key，依序：服務、熱鍵、目標語言、進階數值）。
    fn collect(&self) -> (Config, Vec<String>) {
        let mut cfg = Config::new();
        if let Some(slots) = &self.slots_at_open {
            cfg.insert("service_slots".into(), slots.clone());
        }
        self.services.draft().write_into(&mut cfg);
        let target = pystr::strip(&self.target_language);
        let (input, region) = (
            self.hotkeys.value(HotkeyKind::Input),
            self.hotkeys.value(HotkeyKind::Region),
        );
        let values = [
            ("target_language", Value::from(target)),
            (HotkeyKind::Input.config_key(), input.into()),
            (HotkeyKind::Region.config_key(), region.into()),
            ("auto_show_input", self.auto_show_input.into()),
            ("paste_hotkey", self.paste_hotkey.into()),
            ("translate_system_messages", self.translate_system.into()),
        ];
        for (key, value) in values {
            cfg.insert(key.into(), value);
        }
        let mut errors = service_errors(&cfg);
        if input == region {
            errors.push("error.hotkeys_same".into());
        }
        if target.is_empty() {
            errors.push("error.need_target_language".into());
        }
        match parse_advanced(&self.advanced) {
            Ok(values) => cfg.extend(values),
            Err(key) => errors.push(key.into()),
        }
        cfg.insert("ui_language".into(), current_language().into());
        (cfg, errors)
    }

    fn save(&mut self, env: &dyn SettingsEnv) {
        let (cfg, errors) = self.collect();
        if !errors.is_empty() {
            log(format!("[settings] save blocked: {}", errors.join(", ")));
            self.errors = errors;
            return;
        }
        self.release_recorder |= self.hotkeys.stop();
        self.release_recorder(env);
        self.pending.push(SettingsAction::Save(cfg));
    }

    /// 側欄：四個導覽項，底部是版本號；回傳這一幀點下的分頁。
    fn sidebar(&self, ui: &mut Ui) -> Option<Page> {
        let margin = Margin {
            left: 8,
            right: 8,
            top: 8,
            bottom: 14,
        };
        Frame::new()
            .inner_margin(margin)
            .show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                ui.spacing_mut().item_spacing.y = 2.0;
                let mut picked = None;
                for page in Page::ALL {
                    if nav_item(ui, page, page == self.page).clicked() {
                        picked = Some(page);
                    }
                }
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
                        ui.add(Label::new(
                            RichText::new(version).size(12.0).color(theme::TEXT_MUTED),
                        ));
                    });
                });
                picked
            })
            .inner
    }

    /// 內容區：上方是目前分頁（可捲動），下方是固定的按鈕列。
    fn content(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, rect: Rect) {
        let id = Id::unique("settings_footer");
        let footer_height: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(theme::FOOTER_HEIGHT);
        let split = (rect.bottom() - footer_height).max(rect.top());
        let page_rect = Rect::from_min_max(rect.min, pos2(rect.right(), split));
        let footer_rect = Rect::from_min_max(pos2(rect.left(), split), rect.max);

        ui.scope_builder(UiBuilder::new().max_rect(page_rect), |ui| {
            ui.set_clip_rect(page_rect);
            self.page_ui(ui, env);
        });

        ui.painter().hline(
            footer_rect.x_range(),
            footer_rect.top(),
            Stroke::new(1.0, theme::CARD_BORDER),
        );
        let (pad_x, pad_y) = (24.0, 14.0);
        let row = footer_rect.shrink2(vec2(pad_x, pad_y));
        let used = ui
            .scope_builder(
                UiBuilder::new()
                    .max_rect(row)
                    .layout(Layout::left_to_right(Align::Max)),
                |ui| self.footer(ui, env, id.with("buttons")),
            )
            .response
            .rect;
        let needed = (used.height() + 2.0 * pad_y).max(theme::FOOTER_HEIGHT);
        if (needed - footer_height).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(id, needed));
            ui.ctx().request_discard("settings footer resized");
        }
    }

    /// 按鈕列：被擋下的儲存提示在左，〔取消〕〔儲存〕靠右。
    fn footer(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, buttons: Id) {
        ui.spacing_mut().item_spacing.x = 8.0;
        if !self.errors.is_empty() {
            self.errors = self.collect().1;
        }
        if !self.errors.is_empty() {
            let room = ui.available_width() - right_aligned_width(ui, buttons) - 16.0;
            let lines: Vec<String> = self.errors.iter().map(|e| t(e, &[])).collect();
            ui.scope(|ui| {
                ui.set_max_width(room.max(120.0));
                callout(ui, &t("dialog.incomplete_title", &[]), &lines);
            });
        }
        let (cancel, save) = right_aligned(ui, buttons, |ui| {
            let cancel = secondary_button(ui, &t("button.cancel", &[])).clicked();
            let save = primary_button(ui, &t("button.save", &[])).clicked();
            (cancel, save)
        });
        if save {
            self.save(env);
        } else if cancel {
            let actions = self.cancel(env);
            self.pending.extend(actions);
        }
    }

    /// 目前分頁：頁標題加內容，各頁各自記住捲動位置。
    fn page_ui(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) {
        let style = ui.style().clone();
        thin_scrollbar(ui);
        let (top, right, bottom, left) = theme::CONTENT_MARGIN;
        let margin = Margin {
            left: left as i8,
            right: right as i8,
            top: top as i8,
            bottom: bottom as i8,
        };
        ScrollArea::vertical()
            .id_salt(("settings_page", self.page))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_style(style);
                Frame::new().inner_margin(margin).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = theme::CARD_GAP;
                    let title = RichText::new(t(self.page.title_key(), &[]))
                        .font(theme::bold(theme::TITLE_SIZE));
                    ui.add(Label::new(title));
                    ui.add_space(6.0);
                    match self.page {
                        Page::Basic => self.basic_page(ui, env),
                        Page::Services => {
                            let target = pystr::strip(&self.target_language).to_owned();
                            self.services.ui(ui, env, &target);
                        }
                        Page::Advanced => self.advanced_page(ui),
                        Page::About => self.about_page(ui, env),
                    }
                });
            });
    }
}

/// 預設服務與三個用途實際指到的服務各自缺哪些必填欄位（錯誤文案 key，去重）；連預設服務都沒有就只回 `error.need_service`。
fn service_errors(cfg: &Config) -> Vec<String> {
    let default = cfg.get("default_service").and_then(Value::as_str);
    if find(cfg, default).is_none() {
        return vec!["error.need_service".into()];
    }
    let slots = cfg.get("service_slots");
    let slot_ids = ServiceSlot::ALL.iter().map(|slot| {
        slots
            .and_then(|s| s.get(slot.key()))
            .and_then(Value::as_str)
    });
    let mut errors: Vec<String> = Vec::new();
    for service in std::iter::once(default)
        .chain(slot_ids)
        .filter_map(|id| find(cfg, id))
    {
        for error in validate_service(service) {
            if !errors.contains(&error) {
                errors.push(error);
            }
        }
    }
    errors
}

/// 捲軸改成 overlay 那種細滑塊、透明軌道；內容要在捲動區裡把樣式換回來。
pub(super) fn thin_scrollbar(ui: &mut Ui) {
    let style = ui.style_mut();
    style.spacing.scroll = ScrollStyle {
        bar_width: scrollbar::WIDTH,
        handle_min_length: scrollbar::MIN_THUMB as f32,
        bar_inner_margin: 2.0,
        bar_outer_margin: 2.0,
        ..ScrollStyle::solid()
    };
    let visuals = &mut style.visuals;
    visuals.extreme_bg_color = Color32::TRANSPARENT;
    let widgets = &mut visuals.widgets;
    for (state, color) in [
        (&mut widgets.inactive, scrollbar::THUMB),
        (&mut widgets.hovered, scrollbar::THUMB_HOVER),
        (&mut widgets.active, scrollbar::THUMB_HOVER),
    ] {
        state.bg_fill = color;
        state.corner_radius = CornerRadius::same(4);
    }
}

/// 側欄的一個導覽項：圖示加名稱；選中的有底色與左側 `ACCENT` 指示條。
fn nav_item(ui: &mut Ui, page: Page, selected: bool) -> eframe::egui::Response {
    let label = t(page.title_key(), &[]);
    let size = vec2(ui.available_width(), theme::NAV_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Tab, ui.is_enabled(), selected, &label));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = theme::CONTROL_RADIUS;
        if selected {
            painter.rect_filled(rect, radius, theme::NAV_SELECTED);
            let bar = Rect::from_min_max(
                pos2(rect.left(), rect.top() + 12.0),
                pos2(rect.left() + 3.0, rect.bottom() - 12.0),
            );
            painter.rect_filled(bar, 2, theme::ACCENT);
        } else if response.hovered() {
            painter.rect_filled(rect, radius, theme::NAV_HOVER);
        }
        let color = if selected || response.hovered() {
            theme::TEXT
        } else {
            theme::NAV_TEXT
        };
        let x = rect.left() + 14.0;
        painter.text(
            pos2(x + 8.0, rect.center().y),
            Align2::CENTER_CENTER,
            page.icon(),
            theme::icons(16.0),
            color,
        );
        painter.text(
            pos2(x + 16.0 + 12.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(theme::TEXT_SIZE),
            color,
        );
    }
    focus_ring(ui, &response, theme::CONTROL_RADIUS);
    response
}
