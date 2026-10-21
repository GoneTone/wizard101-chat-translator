//! 精靈的步驟條與三個步驟的內容。

use eframe::egui::{
    Align, Align2, Color32, CursorIcon, FontId, Frame, Id, Label, Layout, Margin, Pos2, Rect,
    Response, RichText, Role, Sense, Stroke, TextStyle, Ui, Vec2, WidgetInfo, pos2,
};

use super::{Step, Wizard, WizardAction};
use crate::config::app_name;
use crate::i18n::{current_language, language_name, t};
use crate::logs::log;
use crate::ui::icon::app_icon_texture;
use crate::ui::settings::env::SettingsEnv;
use crate::ui::settings::fields::editable_combo;
use crate::ui::settings::language::{common_languages, ui_language_combo, ui_language_hint};
use crate::ui::settings::provider_picker::{CardLayout, provider_cards};
use crate::ui::settings::theme::{self, icon};
use crate::ui::settings::widgets::{
    callout, card, extended_galley, focus_ring, named_by, rich, scroll_page,
};

/// 步驟條的左右內距、圓點直徑與步驟內容的內距。
const STEPS_PADDING_X: f32 = 36.0;
const DOT_SIZE: f32 = 24.0;
const BODY_MARGIN: Margin = Margin {
    left: 36,
    right: 36,
    top: 24,
    bottom: 24,
};

impl Wizard {
    /// 步驟條：編號圓點加名稱，之間是連接線；走過的打勾，目前的是外框加粗體。
    pub(super) fn step_bar(&self, ui: &mut Ui, rect: Rect) {
        ui.painter().rect_filled(rect, 0, theme::MODAL_FOOT);
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            Stroke::new(1.0, theme::CARD_BORDER),
        );
        let current = self.step.index();
        let names: Vec<RichText> = Step::ALL
            .iter()
            .map(|step| {
                let text = RichText::new(t(step.title_key(), &[]));
                match step.index() {
                    i if i == current => text.font(theme::bold(13.0)).color(theme::TEXT),
                    _ => text.size(13.0).color(theme::TEXT_MUTED),
                }
            })
            .collect();
        let widths: Vec<f32> = names
            .iter()
            .map(|name| extended_galley(ui, name.clone(), TextStyle::Body).size().x)
            .collect();
        let (gap, dot_gap) = (10.0, 8.0);
        let fixed: f32 = widths.iter().map(|w| DOT_SIZE + dot_gap + w).sum::<f32>() + 4.0 * gap;
        let room = rect.width() - 2.0 * STEPS_PADDING_X;
        let line = ((room - fixed) / 2.0).max(8.0);
        let y = rect.top() + 22.0 + DOT_SIZE / 2.0;
        let mut x = rect.left() + STEPS_PADDING_X;
        for (i, (name, width)) in names.into_iter().zip(widths).enumerate() {
            step_dot(ui, pos2(x + DOT_SIZE / 2.0, y), i, current);
            x += DOT_SIZE + dot_gap;
            let name_rect = Rect::from_min_max(pos2(x, y - 10.0), pos2(x + width, y + 10.0));
            ui.put(name_rect, Label::new(name).selectable(false));
            x += width;
            if i + 1 < Step::ALL.len() {
                let color = if i < current {
                    theme::ACCENT
                } else {
                    theme::CONTROL_BORDER
                };
                x += gap;
                let bar = Rect::from_min_max(pos2(x, y - 1.0), pos2(x + line, y + 1.0));
                ui.painter().rect_filled(bar, 1, color);
                x += line + gap;
            }
        }
    }

    pub(super) fn body(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) -> Option<WizardAction> {
        let salt = ("wizard_step", self.step);
        scroll_page(ui, salt, BODY_MARGIN, 14.0, |ui| match self.step {
            Step::Language => self.language_step(ui),
            Step::Service => {
                self.service_step(ui, env);
                None
            }
            Step::Prefs => {
                self.prefs_step(ui, env);
                None
            }
        })
    }

    fn language_step(&mut self, ui: &mut Ui) -> Option<WizardAction> {
        let texture = app_icon_texture(ui.ctx(), 48);
        let hero = Margin {
            top: 36,
            bottom: 28,
            ..Margin::ZERO
        };
        Frame::new().inner_margin(hero).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 12.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                ui.painter().image(texture, rect, uv, Color32::WHITE);
                ui.add(Label::new(RichText::new(app_name()).font(theme::bold(22.0))).wrap());
            });
        });
        let label = ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let label = heading(ui, &t("field.ui_language", &[]));
                let hint = t("wizard.language_hint", &[]);
                rich(ui, &hint, theme::HINT_SIZE, theme::TEXT_SECONDARY);
                label
            })
            .inner;
        let chosen = named_by(ui, label, |ui| {
            ui.spacing_mut().combo_width = ui.available_width();
            ui_language_combo(ui)
        });
        rich(ui, &ui_language_hint(), 13.0, theme::TEXT_SECONDARY);
        let code = chosen?;
        self.change_language(&code);
        Some(WizardAction::PreviewLanguage)
    }

    fn service_step(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) {
        ui.add(Label::new(RichText::new(t("wizard.intro", &[])).color(theme::TEXT)).wrap());
        let Some(form) = &mut self.form else {
            heading(ui, &t("wizard.pick_provider", &[]));
            if let Some(key) = provider_cards(ui, CardLayout::Grid) {
                self.pick_provider(key);
            }
            return;
        };
        // 填設定與測試連線要等表單真的畫出來才說得通，選服務商之前不提
        ui.add(
            Label::new(
                RichText::new(t("wizard.intro_form", &[]))
                    .size(theme::CONTROL_TEXT_SIZE)
                    .color(theme::TEXT_SECONDARY),
            )
            .wrap(),
        );
        let target = language_name(&current_language());
        let response = card(ui, |ui| {
            Frame::new()
                .inner_margin(Margin::symmetric(16, 14))
                .show(ui, |ui| form.ui(ui, env, &target))
                .inner
        });
        if response.changed {
            self.skip_test = false;
        }
        self.picking |= response.change_provider;
        if skip_link(ui).clicked() {
            self.skip_test = true;
            log("[ui] wizard connection test skipped");
        }
    }

    fn prefs_step(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) {
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let label = heading(ui, &t("wizard.target_language", &[]));
            ui.spacing_mut().combo_width = ui.available_width();
            let id = Id::unique("wizard_target_language");
            editable_combo(
                ui,
                id,
                &mut self.target_language,
                &common_languages(),
                false,
            )
            .labelled_by(label);
            rich(
                ui,
                &t("hint.language", &[]),
                theme::HINT_SIZE,
                theme::TEXT_SECONDARY,
            );
        });
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = theme::CARD_GAP;
        self.hotkeys.cards(ui, env, &mut self.auto_show_input);
        let errors = self.hotkey_errors();
        self.hotkeys_blocked &= !errors.is_empty();
        if self.hotkeys_blocked {
            let lines: Vec<String> = errors.iter().map(|e| t(e, &[])).collect();
            let shown = ui.scope(|ui| callout(ui, &t("dialog.incomplete_title", &[]), &lines));
            if std::mem::take(&mut self.reveal_callout) {
                shown.response.scroll_to_me(None);
            }
        }
    }
}

/// 一個步驟圓點：走過的 `ACCENT` 底加勾，目前的 `ACCENT` 外框加編號，還沒到的灰框加編號。
fn step_dot(ui: &Ui, center: Pos2, index: usize, current: usize) {
    let painter = ui.painter();
    let radius = DOT_SIZE / 2.0;
    let number = (index + 1).to_string();
    if index < current {
        painter.circle_filled(center, radius, theme::ACCENT);
        painter.text(
            center,
            Align2::CENTER_CENTER,
            icon::SUCCESS,
            theme::icons(11.0),
            theme::ON_ACCENT,
        );
        return;
    }
    let (ring, ink) = if index == current {
        (theme::ACCENT, theme::ACCENT)
    } else {
        (theme::CONTROL_BORDER, theme::TEXT_MUTED)
    };
    painter.circle_stroke(center, radius - 0.75, Stroke::new(1.5, ring));
    let font = FontId::proportional(12.0);
    painter.text(center, Align2::CENTER_CENTER, number, font, ink);
}

/// 粗體的段落標題（會換行）；回傳它的 Id，給底下的欄位當無障礙名稱。
fn heading(ui: &mut Ui, text: &str) -> Id {
    let text = RichText::new(text).font(theme::bold(theme::TEXT_SIZE));
    ui.add(Label::new(text).wrap()).id
}

/// 「略過測試」：靠右的底線文字連結，可用 Tab 走到、Enter／Space 觸發。
fn skip_link(ui: &mut Ui) -> Response {
    let text = t("wizard.skip_test", &[]);
    ui.with_layout(Layout::top_down(Align::Max), |ui| {
        let rich = RichText::new(&text)
            .size(theme::HINT_SIZE)
            .color(theme::TEXT_SECONDARY)
            .underline();
        let galley = extended_galley(ui, rich, TextStyle::Body);
        let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::click());
        response.widget_info(|| WidgetInfo::labeled(Role::Link, ui.is_enabled(), &text));
        let color = if response.hovered() {
            theme::TEXT
        } else {
            theme::TEXT_SECONDARY
        };
        ui.painter()
            .galley_with_override_text_color(rect.min, galley, color);
        focus_ring(ui, &response, 4);
        response.on_hover_cursor(CursorIcon::PointingHand)
    })
    .inner
}
