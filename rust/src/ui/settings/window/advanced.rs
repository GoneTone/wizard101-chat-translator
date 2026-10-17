//! 進階分頁：四個數值、透明度滑桿（拖動即預覽）與系統訊息開關。

use eframe::egui::{
    Align, CornerRadius, Label, Layout, RichText, Slider, Stroke, Ui, lerp, pos2, vec2,
};

use super::{SettingsAction, SettingsWindow};
use crate::config::{AdvancedText, advanced_limits, default_config, py};
use crate::i18n::t;
use crate::ui::settings::fields::number_field;
use crate::ui::settings::theme;
use crate::ui::settings::widgets::{card, focus_ring, name_by_row, setting_row, toggle};

/// 數值欄：設定鍵、名稱、說明與 −／＋ 的步長。
const NUMBERS: [(&str, &str, &str, f64); 4] = [
    ("fade_seconds", "settings.fade", "settings.fade_hint", 10.0),
    (
        "max_messages",
        "settings.max_messages",
        "settings.max_messages_hint",
        10.0,
    ),
    (
        "max_parallel_translations",
        "settings.parallel",
        "settings.parallel_hint",
        1.0,
    ),
    (
        "type_delay",
        "settings.type_delay",
        "settings.type_delay_hint",
        0.01,
    ),
];

fn field<'a>(text: &'a mut AdvancedText, key: &str) -> &'a mut String {
    match key {
        "fade_seconds" => &mut text.fade_seconds,
        "max_messages" => &mut text.max_messages,
        "max_parallel_translations" => &mut text.max_parallel_translations,
        _ => &mut text.type_delay,
    }
}

/// 範圍說明裡的上下限，寫法同 Python 的 ADVANCED_LIMITS：浮點數欄位寫成 `0.0`，整數欄位寫成 `0`。
fn bound(key: &str, x: f64) -> String {
    if default_config()[key].is_f64() {
        py::float_repr(x)
    } else {
        (x as i64).to_string()
    }
}

/// `settings.range_hint`：說明加上範圍與預設值。
fn range_hint(key: &str, hint_key: &str) -> String {
    let (lo, hi) = advanced_limits(key);
    t(
        "settings.range_hint",
        &[
            ("hint", &t(hint_key, &[])),
            ("lo", &bound(key, lo)),
            ("hi", &bound(key, hi)),
            ("default", &py::display(&default_config()[key])),
        ],
    )
}

impl SettingsWindow {
    pub(super) fn advanced_page(&mut self, ui: &mut Ui) {
        card(ui, |ui| {
            for (key, label_key, hint_key, step) in NUMBERS {
                let hint = range_hint(key, hint_key);
                setting_row(ui, &t(label_key, &[]), Some(&hint), |ui| {
                    number_field(
                        ui,
                        field(&mut self.advanced, key),
                        step,
                        advanced_limits(key),
                    );
                });
            }
        });
        let mut previewed = None;
        card(ui, |ui| {
            let default = py::display(&default_config()["overlay_alpha"]);
            let hint = t("settings.alpha_hint", &[("default", &default)]);
            setting_row(ui, &t("settings.alpha", &[]), Some(&hint), |ui| {
                if alpha_slider(ui, &mut self.advanced.overlay_alpha) {
                    previewed = Some(self.advanced.overlay_alpha);
                }
            });
        });
        if let Some(alpha) = previewed {
            self.pending
                .push(SettingsAction::PreviewAlpha(alpha as f32));
        }
        card(ui, |ui| {
            let label = t("field.translate_system", &[]);
            setting_row(ui, &label, None, |ui| {
                toggle(ui, &mut self.translate_system, &label);
            });
        });
    }
}

/// 透明度滑桿（寬 `SLIDER_WIDTH`、兩位小數）加右側的數值；回傳這一幀是否改了值。
fn alpha_slider(ui: &mut Ui, alpha: &mut f64) -> bool {
    let (lo, hi) = advanced_limits("overlay_alpha");
    ui.spacing_mut().item_spacing.x = 12.0;
    let response = ui
        .scope(|ui| {
            let style = ui.style_mut();
            style.spacing.slider_width = theme::SLIDER_WIDTH;
            style.spacing.slider_rail_height = 4.0;
            // 把手半徑是高度的 1/2.5：高 25 才是設計稿的 20 寬圓鈕
            style.spacing.interact_size.y = 25.0;
            style.visuals.selection.bg_fill = theme::ACCENT;
            style.visuals.widgets.inactive.bg_fill = theme::CONTROL_BORDER;
            style.visuals.widgets.inactive.corner_radius = CornerRadius::same(2);
            ui.add(
                Slider::new(alpha, lo..=hi)
                    .step_by(0.01)
                    .show_value(false)
                    .trailing_fill(true),
            )
        })
        .inner;
    let response = name_by_row(ui, response);
    let changed = response.changed();
    if changed {
        *alpha = (*alpha * 100.0).round() / 100.0;
    }
    // 把手照設計稿重畫：控制項底色的圓鈕，中間一點 ACCENT
    let rect = response.rect;
    let radius = rect.height() / 2.5;
    let t = ((*alpha - lo) / (hi - lo)) as f32;
    let center = pos2(
        lerp(rect.left() + radius..=rect.right() - radius, t),
        rect.center().y,
    );
    let ring = if response.hovered() || response.dragged() {
        theme::CONTROL_UNDERLINE
    } else {
        theme::CONTROL_BORDER
    };
    let painter = ui.painter();
    painter.circle(center, radius, theme::CONTROL_BG, Stroke::new(1.0, ring));
    painter.circle_filled(center, radius / 2.0, theme::ACCENT);
    focus_ring(ui, &response, theme::CONTROL_RADIUS);
    ui.allocate_ui_with_layout(
        vec2(36.0, theme::CONTROL_HEIGHT),
        Layout::right_to_left(Align::Center),
        |ui| {
            ui.add(Label::new(
                RichText::new(format!("{alpha:.2}")).size(theme::CONTROL_TEXT_SIZE),
            ))
        },
    );
    changed
}
