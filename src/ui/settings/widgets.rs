//! 設定視窗與首次設定精靈共用的元件：卡片、設定列、開關、按鈕、收合區塊、按鍵帽、提示與結果；輸入欄位在 `fields`。
//!
//! 用到 `fonts_egui` 的粗體與圖示字族，畫之前要先 `fonts_egui::install`。

use std::sync::Arc;

use eframe::egui::style::{ScrollStyle, StyleModifier, WidgetVisuals};
use eframe::egui::{
    Align, Align2, AsIdSalt, Color32, ComboBox, CornerRadius, FontId, Frame, Galley, Id, Label,
    Layout, Margin, Pos2, Rect, Response, RichText, Role, ScrollArea, Sense, Shape, Stroke,
    StrokeKind, Style, TextStyle, TextWrapMode, Tooltip, Ui, UiBuilder, UiStackInfo, Vec2,
    WidgetInfo, WidgetText, lerp, pos2, vec2,
};

use crate::input::keys::split_hotkey;
use crate::py::pystr;
use crate::ui::overlay::scrollbar;
use crate::ui::richtext;
use crate::ui::settings::theme::{self, icon};

/// 一頁可捲動的內容：細捲軸、`margin` 內距、項目縱向間距 `gap`；`salt` 讓各頁記住各自的捲動位置。
pub(super) fn scroll_page<R>(
    ui: &mut Ui,
    salt: impl AsIdSalt,
    margin: Margin,
    gap: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let style = ui.style().clone();
    thin_scrollbar(ui);
    ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_style(style);
            Frame::new()
                .inner_margin(margin)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = gap;
                    add(ui)
                })
                .inner
        })
        .inner
}

/// 捲軸改成 overlay 那種細滑塊、透明軌道；內容要在捲動區裡把樣式換回來。
fn thin_scrollbar(ui: &mut Ui) {
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

/// 畫一個圖示字，置中在 `size` 寬、`text_font` 一行高的框裡（不產生無障礙節點，旁邊的文字就是它的意思）。
///
/// 放在一段文字前面、靠上對齊時，圖示就與文字的第一行垂直置中；圖示字型的行高與文字字型不同，不能用正方形的框。
pub(super) fn glyph(ui: &mut Ui, glyph: char, size: f32, color: Color32, text_font: &FontId) {
    let line = ui.fonts_mut(|f| f.row_height(text_font));
    let (rect, _) = ui.allocate_exact_size(vec2(size, line.max(size)), Sense::hover());
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

/// `add` 裡的開關、下拉以 `label` 這個元件當無障礙名稱（同 `setting_row` 的控制項）。
pub(super) fn named_by<R>(ui: &mut Ui, label: Id, add: impl FnOnce(&mut Ui) -> R) -> R {
    let info = UiStackInfo::default().with_tag_value(ROW_LABEL_TAG, label);
    ui.scope_builder(UiBuilder::new().ui_stack_info(info), add)
        .inner
}

/// 用 `richtext::show` 顯示一段可含連結、可反白複製的文字，字級 `size`。
pub(super) fn rich(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    ui.scope(|ui| {
        ui.style_mut().override_font_id = Some(FontId::proportional(size));
        // 換行排版的每一列至少 `interact_size.y` 高，設定介面把它拉到控制項高 32：
        // 不歸零的話一行字會置中在 32 高的列裡，比前面的圖示低、行距也被撐大
        ui.spacing_mut().interact_size.y = 0.0;
        richtext::show(ui, text, color, true);
    });
}

/// 下拉彈出層的樣式：設定介面的樣式，清單項目間距固定（不受下拉所在版面的 `item_spacing` 影響）。
pub(super) fn popup_style(ui: &Ui) -> Style {
    let mut style = (**ui.style()).clone();
    style.spacing.item_spacing.y = theme::POPUP_ITEM_GAP;
    style
}

/// 下拉右側的箭頭，畫在距控制項右緣 `CHEVRON_INSET` 處。
pub(super) fn paint_chevron(ui: &Ui, center: Pos2) {
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        icon::EXPAND,
        theme::icons(10.0),
        theme::TEXT_SECONDARY,
    );
}

/// 設定介面的唯讀下拉：彈出層用 `popup_style`，箭頭與可輸入的下拉（`fields::editable_combo`）相同。
pub(super) fn combo_box<'a>(ui: &Ui, id_salt: impl AsIdSalt) -> ComboBox<'a> {
    ComboBox::from_id_salt(id_salt)
        .popup_style(StyleModifier::from(popup_style(ui)))
        .icon(|ui: &Ui, rect: Rect, _: &WidgetVisuals, _| {
            // egui 把圖示放在右內距之內；換算回控制項右緣
            let right = rect.right() + ui.spacing().button_padding.x;
            paint_chevron(ui, pos2(right - theme::CHEVRON_INSET, rect.center().y));
        })
}

/// 在水平列裡放一欄由上往下的內容，整欄與列垂直置中；高度取上一幀以 `id` 量到的值，變了就重排一次。
pub(super) fn centered_column<R>(
    ui: &mut Ui,
    id: Id,
    width: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let height: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
    let shown = ui.allocate_ui_with_layout(vec2(width, height), Layout::top_down(Align::Min), add);
    let used = shown.response.rect.height();
    if (used - height).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(id, used));
        ui.ctx().request_discard("centered column resized");
    }
    shown.inner
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

/// 清單卡片的無障礙名稱：名稱後接標記，以 ` · ` 分隔。
pub fn card_label(name: &str, badges: &[String]) -> String {
    if badges.is_empty() {
        name.to_owned()
    } else {
        format!("{name}\u{3000}{}", badges.join(" · "))
    }
}

/// 清單卡片（服務、術語表）：`leading` 畫前置圖示，接著是名稱列（名稱後接 `badges`）與一行說明 `hint`，`buttons` 加入的按鈕靠右；整張以 `card_label` 為群組名。
///
/// `buttons_id` 是右側按鈕列量寬度用的 Id，每張卡片要不同。
pub(super) fn list_card<R>(
    ui: &mut Ui,
    buttons_id: Id,
    leading: impl FnOnce(&mut Ui),
    name: &str,
    badges: &[String],
    hint: &str,
    buttons: impl FnOnce(&mut Ui) -> R,
) -> R {
    card(ui, |ui| {
        let inner = Frame::new()
            .inner_margin(Margin::symmetric(16, 12))
            .show(ui, |ui| {
                let name_font = theme::bold(theme::TEXT_SIZE);
                let pill_font = FontId::proportional(11.5);
                let hint_font = FontId::proportional(theme::HINT_SIZE);
                let (name_h, pill_h, hint_h) = ui.fonts_mut(|f| {
                    (
                        f.row_height(&name_font),
                        f.row_height(&pill_font),
                        f.row_height(&hint_font),
                    )
                });
                let name_h = name_h.max(pill_h + 2.0);
                let column_h = name_h + 3.0 + hint_h;
                let size = vec2(ui.available_width(), column_h.max(theme::CONTROL_HEIGHT));
                ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    leading(ui);
                    let reserved = right_aligned_width(ui, buttons_id);
                    let text_width = (ui.available_width() - reserved - 16.0).max(24.0);
                    let (column, _) =
                        ui.allocate_exact_size(vec2(text_width, size.y), Sense::hover());
                    let layout = UiBuilder::new()
                        .max_rect(column)
                        .layout(Layout::top_down(Align::Min));
                    // 不經父層配置空間：標記太多時截掉，不可把按鈕擠出卡片
                    let mut text = ui.new_child(layout);
                    text.shrink_clip_rect(column);
                    text.spacing_mut().item_spacing.y = 3.0;
                    text.add_space((size.y - column_h) / 2.0);
                    name_line(&mut text, name, badges, name_font, name_h);
                    text.add(
                        Label::new(
                            RichText::new(hint)
                                .font(hint_font)
                                .color(theme::TEXT_SECONDARY),
                        )
                        .truncate(),
                    );
                    right_aligned(ui, buttons_id, |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        buttons(ui)
                    })
                })
                .inner
            })
            .inner;
        name_card(ui, card_label(name, badges));
        inner
    })
}

/// 整張卡片是一個以 `label` 為名的群組，螢幕報讀一次念完名稱與標記。
fn name_card(ui: &Ui, label: String) {
    let rect = ui.min_rect();
    ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
        node.set_role(Role::Group);
        node.set_label(label);
        node.set_bounds(eframe::egui::accesskit::Rect {
            x0: rect.min.x.into(),
            y0: rect.min.y.into(),
            x1: rect.max.x.into(),
            y1: rect.max.y.into(),
        });
    });
}

/// 清單卡片的名稱列：名稱（太長就截斷）後接標記。
fn name_line(ui: &mut Ui, name: &str, badges: &[String], font: FontId, height: f32) {
    let size = vec2(ui.available_width(), height);
    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let pill_font = FontId::proportional(11.5);
        let pills: f32 = badges
            .iter()
            .map(|b| {
                let galley =
                    ui.painter()
                        .layout_no_wrap(b.clone(), pill_font.clone(), theme::PILL_TEXT);
                galley.size().x + 16.0 + 8.0
            })
            .sum();
        let room = (ui.available_width() - pills).max(24.0);
        ui.scope(|ui| {
            ui.set_max_width(room);
            ui.add(Label::new(RichText::new(name).font(font)).truncate());
        });
        for badge in badges {
            Frame::new()
                .fill(theme::PILL_BG)
                .corner_radius(10)
                .inner_margin(Margin::symmetric(8, 1))
                .show(ui, |ui| {
                    ui.add(
                        Label::new(
                            RichText::new(badge)
                                .font(pill_font.clone())
                                .color(theme::PILL_TEXT),
                        )
                        .selectable(false),
                    );
                });
        }
    });
}

/// 設定列裡下拉最多占整列寬度的比例。
const MAX_CONTROL_SHARE: f32 = 0.45;

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
                        // 視窗窄時下拉讓出寬度，名稱與說明才不會被擠成一欄幾個字
                        let spacing = ui.spacing_mut();
                        spacing.combo_width = spacing.combo_width.min(width * MAX_CONTROL_SHARE);
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
        let (off_fill, off_border, on_fill) = if response.hovered() {
            (theme::CONTROL_BG, theme::NAV_TEXT, lighter(theme::ACCENT))
        } else {
            (Color32::TRANSPARENT, theme::TEXT_MUTED, theme::ACCENT)
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            radius,
            off_fill.lerp_to_gamma(on_fill, t),
            Stroke::new(1.0, off_border.lerp_to_gamma(on_fill, t)),
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

/// 按鈕文字旁的圖示：`Leading` 在文字前、`Trailing` 在文字後。
#[derive(Clone, Copy)]
enum Glyph {
    Leading(char),
    Trailing(char),
}

/// 按鈕裡圖示與文字的間距。
const GLYPH_GAP: f32 = 6.0;

/// hover 時亮一階的實心底色（主要、危險按鈕與開著的開關）。
fn lighter(color: Color32) -> Color32 {
    color.lerp_to_gamma(Color32::WHITE, 0.15)
}

/// 自己畫的按鈕：高 `CONTROL_HEIGHT`、內容水平垂直置中；egui 的 `Button` 會依外層 Layout 對齊內容，
/// 在 `Align::Max`／`Align::Min` 的列裡字會貼底或貼頂。
///
/// `fill` 是（底色, 字色），`None` 用控制項的樣式（hover、按下跟 `theme::apply` 的 widgets 一致）；
/// 無障礙名稱只取文字。
fn styled_button(
    ui: &mut Ui,
    text: &str,
    fill: Option<(Color32, Color32)>,
    glyph: Option<Glyph>,
) -> Response {
    let (ink, font) = match fill {
        Some((_, ink)) => (ink, theme::bold(theme::CONTROL_TEXT_SIZE)),
        None => (theme::TEXT, FontId::proportional(theme::CONTROL_TEXT_SIZE)),
    };
    let painter = ui.painter();
    let label = painter.layout_no_wrap(text.to_owned(), font, ink);
    let icon = glyph.map(|g| {
        let (c, size) = match g {
            Glyph::Leading(c) => (c, 12.0),
            Glyph::Trailing(c) => (c, 11.0),
        };
        painter.layout_no_wrap(c.to_string(), theme::icons(size), ink)
    });
    let icon_width = icon.as_ref().map_or(0.0, |g| g.size().x + GLYPH_GAP);
    let content_width = label.size().x + icon_width;
    let padding = ui.spacing().button_padding.x;
    let size = vec2(content_width + 2.0 * padding, theme::CONTROL_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), text));
    if ui.is_rect_visible(rect) {
        let (bg, border) = match fill {
            Some((fill, _)) if response.hovered() || response.is_pointer_button_down_on() => {
                (lighter(fill), lighter(fill))
            }
            Some((fill, _)) => (fill, fill),
            None => {
                let visuals = ui.style().interact(&response);
                (visuals.bg_fill, visuals.bg_stroke.color)
            }
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            theme::CONTROL_RADIUS,
            bg,
            Stroke::new(1.0, border),
            StrokeKind::Inside,
        );
        let mut x = rect.center().x - content_width / 2.0;
        let mut put = |galley: Arc<Galley>| {
            let pos = pos2(x, rect.center().y - galley.size().y / 2.0);
            x += galley.size().x + GLYPH_GAP;
            painter.galley(pos, galley, ink);
        };
        match (glyph, icon) {
            (Some(Glyph::Leading(_)), Some(icon)) => {
                put(icon);
                put(label);
            }
            (_, Some(icon)) => {
                put(label);
                put(icon);
            }
            (_, None) => put(label),
        }
    }
    focus_ring(ui, &response, theme::CONTROL_RADIUS);
    response
}

/// 主要按鈕：`ACCENT` 底、粗體深色字。
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    styled_button(ui, text, Some((theme::ACCENT, theme::ON_ACCENT)), None)
}

/// 主要按鈕，文字後接一個圖示（如〔下一步〕的箭頭）；無障礙名稱只取文字。
pub fn primary_icon_button(ui: &mut Ui, text: &str, glyph: char) -> Response {
    styled_button(
        ui,
        text,
        Some((theme::ACCENT, theme::ON_ACCENT)),
        Some(Glyph::Trailing(glyph)),
    )
}

/// 次要按鈕：控制項底色加外框。
pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    styled_button(ui, text, None, None)
}

/// 次要按鈕，文字前有一個圖示；無障礙名稱只取文字。
pub fn secondary_icon_button(ui: &mut Ui, glyph: char, text: &str) -> Response {
    styled_button(ui, text, None, Some(Glyph::Leading(glyph)))
}

/// 危險操作（刪除確認）按鈕：`DANGER` 底。
pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    styled_button(ui, text, Some((theme::DANGER, theme::ON_DANGER)), None)
}

/// 占滿寬度、高 `height` 的虛線框按鈕（如〔＋ 新增翻譯服務〕）：平時透明，hover／按下時同控制項的樣式。
pub fn dashed_button(ui: &mut Ui, text: &str, height: f32) -> Response {
    let size = vec2(ui.available_width(), height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), text));
    if ui.is_rect_visible(rect) {
        let (fill, border) = if response.hovered() || response.is_pointer_button_down_on() {
            let visuals = ui.style().interact(&response);
            (visuals.bg_fill, visuals.bg_stroke.color)
        } else {
            (Color32::TRANSPARENT, theme::CONTROL_BORDER)
        };
        let painter = ui.painter();
        painter.rect_filled(rect, theme::CONTROL_RADIUS, fill);
        let edge = rect.shrink(0.5);
        let outline = [
            edge.left_top(),
            edge.right_top(),
            edge.right_bottom(),
            edge.left_bottom(),
            edge.left_top(),
        ];
        painter.extend(Shape::dashed_line(
            &outline,
            Stroke::new(1.0, border),
            4.0,
            3.0,
        ));
        let font = FontId::proportional(theme::CONTROL_TEXT_SIZE);
        let galley = painter.layout_no_wrap(text.to_owned(), font, theme::TEXT);
        let pos = rect.center() - galley.size() / 2.0;
        painter.galley(pos, galley, theme::TEXT);
    }
    focus_ring(ui, &response, theme::CONTROL_RADIUS);
    response
}

/// 只有圖示的方形按鈕；`tooltip` 同時是無障礙名稱。
pub fn icon_button(ui: &mut Ui, glyph: char, tooltip: &str) -> Response {
    icon_button_with(ui, glyph, tooltip, theme::CONTROL_HEIGHT, theme::CONTROL_BG)
}

/// 框內圖示按鈕與控制項外框的間距。
pub const INLINE_ICON_INSET: f32 = 4.0;

/// 放在控制項框內的圖示按鈕（例如金鑰欄的顯示切換）：比框矮一圈，hover 底色要和框底分得出來。
pub fn inline_icon_button(ui: &mut Ui, glyph: char, tooltip: &str) -> Response {
    icon_button_with(
        ui,
        glyph,
        tooltip,
        theme::CONTROL_HEIGHT - 2.0 * INLINE_ICON_INSET,
        theme::CONTROL_BORDER,
    )
}

fn icon_button_with(
    ui: &mut Ui,
    glyph: char,
    tooltip: &str,
    size: f32,
    hover_fill: Color32,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), tooltip));
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered() && ui.is_enabled();
        if hovered {
            ui.painter()
                .rect_filled(rect, theme::CONTROL_RADIUS, hover_fill);
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
        let hovered = response.hovered();
        if hovered {
            painter.rect_filled(rect.shrink(1.0), theme::CARD_RADIUS, theme::CONTROL_BG);
        }
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
            if hovered {
                theme::TEXT
            } else {
                theme::TEXT_SECONDARY
            },
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

/// 把熱鍵字串依 `split_hotkey` 拆成「段 → 鍵」的按鍵帽文字，每個詞首字母大寫。
pub fn keycaps(hotkey: &str) -> Vec<Vec<String>> {
    let capitalize = |word: &str| {
        let mut chars = word.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect::<String>()
        })
    };
    split_hotkey(hotkey)
        .into_iter()
        .filter(|step| step.iter().any(|key| !pystr::strip(key).is_empty()))
        .map(|step| {
            step.into_iter()
                .map(|key| key.split(' ').map(capitalize).collect::<Vec<_>>().join(" "))
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

/// 不換行也不截斷的單行文字排版。
pub(super) fn extended_galley(
    ui: &Ui,
    text: impl Into<WidgetText>,
    style: TextStyle,
) -> Arc<Galley> {
    text.into()
        .into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, style)
}

/// 一個按鍵帽：高 26、左右內距 9，外框 1 寬、底邊 2 寬。
fn keycap(ui: &mut Ui, key: &str) {
    let text = RichText::new(key).size(theme::HINT_SIZE).color(theme::TEXT);
    let galley = extended_galley(ui, text.clone(), TextStyle::Body);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 18.0, 26.0), Sense::hover());
    let face = Rect::from_min_max(rect.min + vec2(1.0, 1.0), rect.max - vec2(1.0, 2.0));
    let painter = ui.painter();
    painter.rect_filled(rect, 5, theme::KEYCAP_BORDER);
    painter.rect_filled(face, 4, theme::KEYCAP_BG);
    ui.put(face, Label::new(text).selectable(false));
}

/// `item` 裡第一段與 `query` 相符的位元組範圍：去頭尾空白、不分（ASCII）大小寫；查詢空白時沒有。
pub fn match_range(item: &str, query: &str) -> Option<std::ops::Range<usize>> {
    let keyword = pystr::strip(query).to_ascii_lowercase();
    if keyword.is_empty() {
        return None;
    }
    let start = item.to_ascii_lowercase().find(&keyword)?;
    Some(start..start + keyword.len())
}

/// 清單裡最符合 `query` 的一項：完全相同優先，其次開頭相符，再來是包含；同級取最前面的。
pub fn best_match(models: &[String], query: &str) -> Option<usize> {
    let keyword = pystr::strip(query);
    let rank = |item: &String| match match_range(item, keyword)? {
        range if range.start == 0 && range.end == item.len() => Some(0),
        range if range.start == 0 => Some(1),
        _ => Some(2),
    };
    models
        .iter()
        .enumerate()
        .filter_map(|(i, item)| rank(item).map(|r| (r, i)))
        .min()
        .map(|(_, i)| i)
}

/// 儲存被擋下的提示：警告圖示、粗體標題，下面每個問題一行；占滿可用寬度。
pub fn callout(ui: &mut Ui, title: &str, lines: &[String]) {
    Frame::new()
        .fill(theme::WARN_BG)
        .stroke(Stroke::new(1.0, theme::WARN_BORDER))
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let title_font = theme::bold(13.0);
                glyph(ui, icon::WARNING, 14.0, theme::WARN_TEXT, &title_font);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(Label::new(
                        RichText::new(title)
                            .font(title_font)
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
        glyph(ui, mark, 13.0, color, &FontId::proportional(13.0));
        ui.vertical(|ui| rich(ui, text, 13.0, color));
    });
}
