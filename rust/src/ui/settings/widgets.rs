//! 設定視窗與首次設定精靈共用的元件：卡片、設定列、開關、按鈕、收合區塊、按鍵帽、提示與結果；輸入欄位在 `fields`。
//!
//! 用到 `fonts_egui` 的粗體與圖示字族，畫之前要先 `fonts_egui::install`。

use eframe::egui::{
    Align, Align2, Button, Color32, CornerRadius, FontId, Frame, Id, Label, Layout, Margin, Rect,
    Response, RichText, Role, Sense, Stroke, StrokeKind, TextStyle, TextWrapMode, Tooltip, Ui,
    UiBuilder, UiStackInfo, Vec2, WidgetInfo, WidgetText, lerp, pos2, vec2,
};

use crate::py::pystr;
use crate::ui::richtext;
use crate::ui::settings::theme::{self, icon};

/// 聚焦時在元件外畫一圈 `ACCENT` 外框。
pub(super) fn focus_ring(ui: &Ui, response: &Response, radius: u8) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            CornerRadius::same(radius.saturating_add(2)),
            Stroke::new(2.0, theme::ACCENT),
            StrokeKind::Outside,
        );
    }
}

/// 畫一個圖示字（不產生無障礙節點，旁邊的文字就是它的意思）。
pub(super) fn glyph(ui: &mut Ui, glyph: char, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        theme::icons(size),
        color,
    );
}

const ROW_LABEL_TAG: &str = "settings_row_label";

/// 在 `setting_row` 的控制項裡時，用那一列的名稱當無障礙名稱；否則交給呼叫端命名。
pub(super) fn name_by_row(ui: &Ui, response: Response) -> Response {
    let row_label = ui
        .stack()
        .iter()
        .find_map(|frame| frame.tags().get_downcast::<Id>(ROW_LABEL_TAG).copied());
    match row_label {
        Some(label) => response.labelled_by(label),
        None => response,
    }
}

/// 用 `richtext::show` 顯示一段可含連結的文字，字級 `size`。
pub(super) fn rich(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    ui.scope(|ui| {
        ui.style_mut().override_font_id = Some(FontId::proportional(size));
        richtext::show(ui, text, color);
    });
}

/// 把 `add` 加入的元件推到這一列的右緣；元件依加入順序由左往右排，Tab 順序與畫面一致。
///
/// 寬度取上一幀以 `id` 量到的值（`right_aligned_width`）排版，變了就重排一次（`request_discard`）。
pub(super) fn right_aligned<R>(ui: &mut Ui, id: Id, add: impl FnOnce(&mut Ui) -> R) -> R {
    let width = right_aligned_width(ui, id);
    ui.add_space((ui.available_width() - width).max(0.0));
    let shown = ui.scope(add);
    let used = shown.response.rect.width();
    if (used - width).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(id, used));
        ui.ctx().request_discard("right-aligned row resized");
    }
    shown.inner
}

/// `right_aligned` 上一幀量到的寬度；還沒量過是 0。
pub(super) fn right_aligned_width(ui: &Ui, id: Id) -> f32 {
    ui.data(|d| d.get_temp(id)).unwrap_or(0.0)
}

/// 卡片：圓角深底加外框，內容占滿可用寬度，內部列與列之間不留空。
pub fn card<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(theme::CARD_BG)
        .stroke(Stroke::new(1.0, theme::CARD_BORDER))
        .corner_radius(theme::CARD_RADIUS)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add_contents(ui)
        })
        .inner
}

/// 卡片裡的一列：左邊名稱與說明（說明可含連結），右邊控制項，兩邊垂直置中；不是第一列時上方畫分隔線。
///
/// 開關與 `fields` 的下拉、數值欄放在這裡時以這一列的名稱當無障礙名稱。
/// 控制項的寬高與左欄高度取上一幀量到的值排版，變了就重排一次（`request_discard`）。
pub fn setting_row(ui: &mut Ui, label: &str, hint: Option<&str>, control: impl FnOnce(&mut Ui)) {
    let details = |ui: &mut Ui| {
        if let Some(hint) = hint {
            rich(ui, hint, theme::HINT_SIZE, theme::TEXT_SECONDARY);
        }
    };
    setting_row_with(ui, label, details, control);
}

/// 同 `setting_row`，但名稱下方的內容由 `details` 自己畫（例如路徑加說明兩行）。
pub fn setting_row_with(
    ui: &mut Ui,
    label: &str,
    details: impl FnOnce(&mut Ui),
    control: impl FnOnce(&mut Ui),
) {
    let id = ui.scope_id().with(("setting_row", label));
    let first = ui.cursor().top() <= ui.max_rect().top() + 0.5;
    let (pad_x, pad_y) = theme::ROW_PADDING;
    let inner_min = theme::ROW_MIN_HEIGHT - 2.0 * f32::from(pad_y);
    let row = Frame::new()
        .inner_margin(Margin::symmetric(pad_x, pad_y))
        .show(ui, |ui| {
            let width = ui.available_width();
            let (label_h, control_size): (f32, Vec2) = ui
                .data(|d| d.get_temp(id))
                .unwrap_or((inner_min, Vec2::ZERO));
            let height = label_h.max(control_size.y).max(inner_min);
            let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            let gap = f32::from(pad_x);
            let label_rect = Rect::from_min_size(
                pos2(rect.left(), rect.center().y - label_h / 2.0),
                vec2((width - control_size.x - gap).max(0.0), label_h),
            );
            let label_column = ui.scope_builder(
                UiBuilder::new()
                    .max_rect(label_rect)
                    .layout(Layout::top_down(Align::Min)),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    let label_id = ui.add(Label::new(label).wrap()).id;
                    details(ui);
                    label_id
                },
            );
            let label_used = label_column.response.rect;
            let control_rect = Rect::from_min_size(
                pos2(
                    rect.right() - control_size.x,
                    rect.center().y - control_size.y / 2.0,
                ),
                control_size,
            );
            let control_used = ui
                .scope_builder(
                    UiBuilder::new()
                        .max_rect(control_rect)
                        .layout(Layout::left_to_right(Align::Center))
                        .ui_stack_info(
                            UiStackInfo::default()
                                .with_tag_value(ROW_LABEL_TAG, label_column.inner),
                        ),
                    |ui| {
                        ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
                        control(ui);
                    },
                )
                .response
                .rect;
            let measured = (label_used.height(), control_used.size());
            let moved =
                (measured.0 - label_h).abs() > 0.5 || (measured.1 - control_size).length() > 0.5;
            if moved {
                ui.data_mut(|d| d.insert_temp(id, measured));
                ui.ctx().request_discard("setting row resized");
            }
        });
    if !first {
        let rect = row.response.rect;
        ui.painter().hline(
            rect.x_range(),
            rect.top(),
            Stroke::new(1.0, theme::CARD_BORDER),
        );
    }
}

/// 40×20 的開關；`label` 只當無障礙名稱（在 `setting_row` 裡改用該列名稱），畫面上的文字由 `setting_row` 畫。
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(theme::TOGGLE_SIZE, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::selected(Role::Switch, ui.is_enabled(), *on, label));
    let response = name_by_row(ui, response);
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let radius = CornerRadius::same((rect.height() / 2.0) as u8);
        let painter = ui.painter();
        painter.rect(
            rect,
            radius,
            Color32::TRANSPARENT.lerp_to_gamma(theme::ACCENT, t),
            Stroke::new(1.0, theme::TEXT_MUTED.lerp_to_gamma(theme::ACCENT, t)),
            StrokeKind::Inside,
        );
        let x = lerp(rect.left() + 10.0..=rect.right() - 10.0, t);
        painter.circle_filled(
            pos2(x, rect.center().y),
            6.0,
            theme::NAV_TEXT.lerp_to_gamma(theme::ON_ACCENT, t),
        );
    }
    focus_ring(ui, &response, 10);
    response
}

fn styled_button(ui: &mut Ui, text: &str, fill: Option<(Color32, Color32)>) -> Response {
    let button = match fill {
        Some((fill, ink)) => Button::new(
            RichText::new(text)
                .color(ink)
                .font(theme::bold(theme::CONTROL_TEXT_SIZE)),
        )
        .fill(fill)
        .stroke(Stroke::new(1.0, fill)),
        None => Button::new(text),
    };
    let response = ui.add(
        button
            .corner_radius(theme::CONTROL_RADIUS)
            .min_size(vec2(0.0, theme::CONTROL_HEIGHT)),
    );
    focus_ring(ui, &response, theme::CONTROL_RADIUS);
    response
}

/// 主要按鈕：`ACCENT` 底、粗體深色字。
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    styled_button(ui, text, Some((theme::ACCENT, theme::ON_ACCENT)))
}

/// 次要按鈕：控制項底色加外框。
pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    styled_button(ui, text, None)
}

/// 次要按鈕，文字前有一個圖示；無障礙名稱只取文字。
pub fn secondary_icon_button(ui: &mut Ui, glyph: char, text: &str) -> Response {
    let atoms = (RichText::new(glyph).font(theme::icons(12.0)), text);
    let response = ui.add(
        Button::new(atoms)
            .corner_radius(theme::CONTROL_RADIUS)
            .min_size(vec2(0.0, theme::CONTROL_HEIGHT)),
    );
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), text));
    focus_ring(ui, &response, theme::CONTROL_RADIUS);
    response
}

/// 危險操作（刪除確認）按鈕：`DANGER` 底。
pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    styled_button(ui, text, Some((theme::DANGER, theme::ON_DANGER)))
}

/// 只有圖示的方形按鈕；`tooltip` 同時是無障礙名稱。
pub fn icon_button(ui: &mut Ui, glyph: char, tooltip: &str) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::splat(theme::CONTROL_HEIGHT), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), tooltip));
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered() && ui.is_enabled();
        if hovered {
            ui.painter()
                .rect_filled(rect, theme::CONTROL_RADIUS, theme::CONTROL_BG);
        }
        let color = if hovered {
            theme::TEXT
        } else {
            theme::TEXT_SECONDARY
        };
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            glyph,
            theme::icons(12.0),
            color,
        );
    }
    focus_ring(ui, &response, theme::CONTROL_RADIUS);
    // 不用 on_hover_text：提示框的 Ui 從全域樣式建立，要帶上設定視窗自己的樣式
    let mut tip = Tooltip::for_enabled(&response);
    tip.popup = tip.popup.style((**ui.style()).clone());
    tip.show(|ui| {
        ui.set_max_width(ui.spacing().tooltip_width);
        ui.label(tooltip);
    });
    response
}

/// 可收合的區塊：整列標題（含說明與箭頭）可點，也能用 Tab 走到、Enter／Space 切換；展開狀態依 `id` 記住。
pub fn collapsible(
    ui: &mut Ui,
    id: Id,
    title: &str,
    hint: Option<&str>,
    default_open: bool,
    body: impl FnOnce(&mut Ui),
) {
    let mut open = ui.data_mut(|d| *d.get_temp_mut_or(id, default_open));
    let (pad_x, pad_y) = theme::ROW_PADDING;
    let (pad_x, pad_y) = (f32::from(pad_x), f32::from(pad_y));
    let chevron = 12.0;
    let width = ui.available_width();
    let text_width = (width - 3.0 * pad_x - chevron).max(0.0);
    let layout = |text: RichText, style: TextStyle| {
        WidgetText::from(text).into_galley(ui, Some(TextWrapMode::Wrap), text_width, style)
    };
    let title_galley = layout(RichText::new(title).color(theme::TEXT), TextStyle::Body);
    let hint_galley = hint.map(|h| {
        layout(
            RichText::new(h).color(theme::TEXT_SECONDARY),
            TextStyle::Small,
        )
    });
    let text_height =
        title_galley.size().y + hint_galley.as_ref().map_or(0.0, |g| 3.0 + g.size().y);
    let height = (text_height + 2.0 * pad_y).max(theme::ROW_MIN_HEIGHT);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let mut response = ui.interact(rect, id.with("header"), Sense::click());
    if response.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::labeled(Role::DisclosureTriangle, ui.is_enabled(), title));
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_expanded(open));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let mut pos = pos2(rect.left() + pad_x, rect.center().y - text_height / 2.0);
        let title_height = title_galley.size().y;
        painter.galley(pos, title_galley, theme::TEXT);
        if let Some(hint_galley) = hint_galley {
            pos.y += title_height + 3.0;
            painter.galley(pos, hint_galley, theme::TEXT_SECONDARY);
        }
        painter.text(
            pos2(rect.right() - pad_x - chevron / 2.0, rect.center().y),
            Align2::CENTER_CENTER,
            if open { icon::COLLAPSE } else { icon::EXPAND },
            theme::icons(chevron),
            theme::TEXT_SECONDARY,
        );
        if response.has_focus() {
            painter.rect_stroke(
                rect.shrink(1.0),
                theme::CARD_RADIUS,
                Stroke::new(2.0, theme::ACCENT),
                StrokeKind::Inside,
            );
        }
    }
    if open {
        body(ui);
    }
}

/// 把熱鍵字串拆成「段 → 鍵」的按鍵帽文字：以 `, ` 分段、以 `+` 分鍵，每個詞首字母大寫。
pub fn keycaps(hotkey: &str) -> Vec<Vec<String>> {
    let capitalize = |word: &str| {
        let mut chars = word.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect::<String>()
        })
    };
    hotkey
        .split(", ")
        .filter(|step| !step.trim().is_empty())
        .map(|step| {
            step.split('+')
                .map(|key| {
                    key.trim()
                        .split(' ')
                        .map(capitalize)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect()
        })
        .collect()
}

/// 以按鍵帽顯示熱鍵，鍵之間是 `+`、段之間是 `,`。
pub fn keycap_row(ui: &mut Ui, hotkey: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let separator = |ui: &mut Ui, text: &str| {
            ui.add(
                Label::new(RichText::new(text).size(12.0).color(theme::TEXT_MUTED))
                    .selectable(false),
            );
        };
        for (i, step) in keycaps(hotkey).iter().enumerate() {
            if i > 0 {
                separator(ui, ",");
            }
            for (j, key) in step.iter().enumerate() {
                if j > 0 {
                    separator(ui, "+");
                }
                keycap(ui, key);
            }
        }
    });
}

/// 一個按鍵帽：高 26、左右內距 9，外框 1 寬、底邊 2 寬。
fn keycap(ui: &mut Ui, key: &str) {
    let text = RichText::new(key).size(theme::HINT_SIZE).color(theme::TEXT);
    let galley = WidgetText::from(text.clone()).into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Body,
    );
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 18.0, 26.0), Sense::hover());
    let face = Rect::from_min_max(rect.min + vec2(1.0, 1.0), rect.max - vec2(1.0, 2.0));
    let painter = ui.painter();
    painter.rect_filled(rect, 5, theme::KEYCAP_BORDER);
    painter.rect_filled(face, 4, theme::KEYCAP_BG);
    ui.put(face, Label::new(text).selectable(false));
}

/// 依關鍵字篩選模型清單：不分大小寫的子字串比對；關鍵字空白時回傳全部。
pub fn filter_models(models: &[String], query: &str) -> Vec<String> {
    let keyword = pystr::strip(query).to_lowercase();
    models
        .iter()
        .filter(|m| m.to_lowercase().contains(&keyword))
        .cloned()
        .collect()
}

/// 儲存被擋下的提示：警告圖示、粗體標題，下面每個問題一行。
pub fn callout(ui: &mut Ui, title: &str, lines: &[String]) {
    Frame::new()
        .fill(theme::WARN_BG)
        .stroke(Stroke::new(1.0, theme::WARN_BORDER))
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                glyph(ui, icon::WARNING, 14.0, theme::WARN_TEXT);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(Label::new(
                        RichText::new(title)
                            .font(theme::bold(13.0))
                            .color(theme::WARN_TEXT),
                    ));
                    for line in lines {
                        ui.add(Label::new(
                            RichText::new(line).size(13.0).color(theme::WARN_TEXT),
                        ));
                    }
                });
            });
        });
}

/// 一次操作的結果：成功綠字打勾、失敗紅字打叉；文字裡的網址可點。
pub fn outcome(ui: &mut Ui, ok: bool, text: &str) {
    let (mark, color) = if ok {
        (icon::SUCCESS, theme::OK)
    } else {
        (icon::FAILURE, theme::ERROR)
    };
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        glyph(ui, mark, 13.0, color);
        ui.vertical(|ui| rich(ui, text, 13.0, color));
    });
}
