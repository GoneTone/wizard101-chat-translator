//! 設定視窗與首次設定精靈共用的元件：卡片、設定列、開關、按鈕、收合區塊、按鍵帽、可輸入下拉、數值欄、提示與結果。
//!
//! 用到 `fonts_egui` 的粗體與圖示字族，畫之前要先 `fonts_egui::install`。

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    Align, Align2, Button, Color32, CornerRadius, Event, FontId, Frame, Id, InputState, Key, Label,
    LayerId, Layout, Margin, Popup, PopupKind, Rect, Response, RichText, Role, Sense, Stroke,
    StrokeKind, SurrenderFocusOn, TextEdit, TextStyle, TextWrapMode, Ui, UiBuilder, UiStackInfo,
    Vec2, WidgetInfo, WidgetText, lerp, pos2, vec2,
};

use crate::py::pystr;
use crate::ui::richtext;
use crate::ui::settings::theme::{self, icon};

/// 聚焦時在元件外畫一圈 `ACCENT` 外框。
fn focus_ring(ui: &Ui, response: &Response, radius: u8) {
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
fn glyph(ui: &mut Ui, glyph: char, size: f32, color: Color32) {
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
fn name_by_row(ui: &Ui, response: Response) -> Response {
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
fn rich(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    ui.scope(|ui| {
        ui.style_mut().override_font_id = Some(FontId::proportional(size));
        richtext::show(ui, text, color);
    });
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
/// 本模組的開關、下拉、數值欄放在這裡時以這一列的名稱當無障礙名稱。
/// 控制項的寬高與左欄高度取上一幀量到的值排版，變了就重排一次（`request_discard`）。
pub fn setting_row(ui: &mut Ui, label: &str, hint: Option<&str>, control: impl FnOnce(&mut Ui)) {
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
                    if let Some(hint) = hint {
                        rich(ui, hint, theme::HINT_SIZE, theme::TEXT_SECONDARY);
                    }
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
    response.on_hover_text(tooltip)
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
                Frame::new()
                    .fill(theme::KEYCAP_BG)
                    .stroke(Stroke::new(1.0, theme::KEYCAP_BORDER))
                    .corner_radius(5)
                    .inner_margin(Margin::symmetric(9, 4))
                    .show(ui, |ui| {
                        ui.add(
                            Label::new(RichText::new(key).size(theme::HINT_SIZE)).selectable(false),
                        );
                    });
            }
        }
    });
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

/// 消耗沒按修飾鍵的 `key`，回傳按了幾次。
fn consume_unmodified(input: &mut InputState, key: Key) -> usize {
    let mut count = 0;
    input.events.retain(|event| {
        let hit = matches!(
            event,
            Event::Key { key: k, modifiers, pressed: true, .. } if *k == key && modifiers.is_none()
        );
        count += usize::from(hit);
        !hit
    });
    count
}

#[derive(Clone, Copy, Default)]
struct ComboState {
    open: bool,
    hot: Option<usize>,
}

/// 可自行輸入的下拉：點輸入框、打字或按 ↓ 展開；`filter` 時邊打邊篩（同 `filter_models`）。
///
/// ↑↓ 選、Enter 選定、Esc 收起，點別處收起；沒有相符項目時不展開。寬度取 `spacing.combo_width`。
/// 不在 `setting_row` 裡時，呼叫端要對回傳值 `labelled_by`／`accessible_name`。
pub fn editable_combo(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    options: &[String],
    filter: bool,
) -> Response {
    let mut state: ComboState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let edit_id = id.with("edit");
    let popup_id = id.with("popup");
    let matches = |text: &str| {
        if filter {
            filter_models(options, text)
        } else {
            options.to_vec()
        }
    };
    let was_visible = state.open && !matches(text).is_empty();
    let (mut down, mut up, mut pick, mut close) = (0, 0, false, false);
    if ui.memory(|m| m.has_focus(edit_id)) {
        ui.input_mut(|input| {
            down = consume_unmodified(input, Key::ArrowDown);
            up = consume_unmodified(input, Key::ArrowUp);
            // 沒選項目時 Enter 照常交給輸入框（結束輸入），Esc 在清單收著時留給對話框當取消
            if was_visible && state.hot.is_some() {
                pick = consume_unmodified(input, Key::Enter) > 0;
            }
            if was_visible {
                close = consume_unmodified(input, Key::Escape) > 0;
            }
        });
    }

    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.spacing().combo_width, theme::CONTROL_HEIGHT),
        Sense::hover(),
    );
    ui.painter().rect(
        rect,
        theme::CONTROL_RADIUS,
        theme::CONTROL_BG,
        Stroke::new(1.0, theme::CONTROL_BORDER),
        StrokeKind::Inside,
    );
    let chevron_rect = Rect::from_min_max(pos2(rect.right() - 28.0, rect.top()), rect.max);
    let edit_rect = Rect::from_min_max(
        pos2(rect.left() + 10.0, rect.top()),
        chevron_rect.left_bottom(),
    );

    // 點清單時不讓輸入框失焦，否則清單會在點擊生效前收起
    let popup_layer = LayerId::new(PopupKind::Popup.order(), popup_id);
    let ctx = ui.ctx().clone();
    let pointer_on_popup = was_visible
        && ctx
            .pointer_interact_pos()
            .and_then(|pos| ctx.layer_id_at(pos))
            == Some(popup_layer);
    let surrender = ctx.options(|o| o.input_options.surrender_focus_on);
    if pointer_on_popup {
        ctx.options_mut(|o| o.input_options.surrender_focus_on = SurrenderFocusOn::Never);
    }
    let mut response = ui.put(
        edit_rect,
        TextEdit::singleline(text)
            .id(edit_id)
            .frame(Frame::NONE)
            .align(Align2::LEFT_CENTER),
    );
    ctx.options_mut(|o| o.input_options.surrender_focus_on = surrender);
    response = name_by_row(ui, response);

    let chevron = ui.interact(chevron_rect, id.with("chevron"), Sense::CLICK);
    if chevron.clicked() {
        response.request_focus();
        state.open = !state.open;
    }
    if response.clicked() || down > 0 {
        state.open = true;
    }
    if response.changed() {
        state.open = true;
        state.hot = None;
    }
    let list = matches(text);
    if let Some(last) = list.len().checked_sub(1) {
        for _ in 0..down {
            state.hot = Some(state.hot.map_or(0, |h| (h + 1).min(last)));
        }
        for _ in 0..up {
            state.hot = state.hot.and_then(|h| h.checked_sub(1));
        }
        state.hot = state.hot.map(|h| h.min(last));
    } else {
        state.hot = None;
    }
    let mut picked = pick.then(|| state.hot.map(|h| list[h].clone())).flatten();
    if close || (!response.has_focus() && !chevron.clicked()) {
        state.open = false;
    }

    let visible = state.open && picked.is_none() && !list.is_empty();
    if visible {
        Popup::new(popup_id, ctx.clone(), rect, ui.layer_id())
            .kind(PopupKind::Popup)
            .open(true)
            .width(rect.width())
            .frame(Frame::popup(ui.style()).inner_margin(4))
            .show(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let moved = ui.input(|i| i.pointer.is_moving());
                for (i, option) in list.iter().enumerate() {
                    let item = ui.add(
                        Button::selectable(state.hot == Some(i), option.as_str())
                            .min_size(vec2(ui.available_width(), theme::CONTROL_HEIGHT)),
                    );
                    if item.hovered() && moved {
                        state.hot = Some(i);
                    }
                    if item.clicked() {
                        picked = Some(option.clone());
                    }
                }
            });
    }
    if let Some(choice) = picked {
        let end = CCursor::new(choice.chars().count());
        *text = choice;
        state.open = false;
        state.hot = None;
        let mut edit = TextEditState::load(&ctx, edit_id).unwrap_or_default();
        edit.cursor.set_char_range(Some(CCursorRange::one(end)));
        edit.store(&ctx, edit_id);
        response.mark_changed();
    }
    ui.data_mut(|d| d.insert_temp(id, state));

    ui.painter().text(
        chevron_rect.center(),
        Align2::CENTER_CENTER,
        icon::EXPAND,
        theme::icons(10.0),
        theme::TEXT_SECONDARY,
    );
    underline(ui, rect, response.has_focus());
    response
}

/// 輸入框底線：聚焦時 2 寬 `ACCENT`，否則 1 寬 `CONTROL_UNDERLINE`。
fn underline(ui: &Ui, rect: Rect, focused: bool) {
    let (width, color) = if focused {
        (2.0, theme::ACCENT)
    } else {
        (1.0, theme::CONTROL_UNDERLINE)
    };
    let y = rect.bottom() - width / 2.0;
    ui.painter()
        .hline(rect.x_range().shrink(2.0), y, Stroke::new(width, color));
}

/// `step` 的小數位數（`0.01` → 2，`1` → 0）。
fn decimals(step: f64) -> usize {
    step.to_string().split_once('.').map_or(0, |(_, f)| f.len())
}

/// 數值欄：輸入框加 −／＋ 兩鈕；按鈕依 `step` 增減並夾在 `range` 內，打的字不在這裡驗證。
///
/// 輸入框的字不是數字時，按鈕從 `range` 的下限起算。不在 `setting_row` 裡時，呼叫端要對回傳值命名。
pub fn number_field(ui: &mut Ui, text: &mut String, step: f64, range: (f64, f64)) -> Response {
    let (min, max) = range;
    let button_width = 30.0;
    let (rect, _) = ui.allocate_exact_size(
        vec2(theme::NUMBER_WIDTH, theme::CONTROL_HEIGHT),
        Sense::hover(),
    );
    ui.painter().rect(
        rect,
        theme::CONTROL_RADIUS,
        theme::CONTROL_BG,
        Stroke::new(1.0, theme::CONTROL_BORDER),
        StrokeKind::Inside,
    );
    let edit_rect = Rect::from_min_max(
        pos2(rect.left() + 10.0, rect.top()),
        pos2(rect.right() - 2.0 * button_width, rect.bottom()),
    );
    let mut response = ui.put(
        edit_rect,
        TextEdit::singleline(text)
            .frame(Frame::NONE)
            .align(Align2::LEFT_CENTER),
    );
    response = name_by_row(ui, response);
    let buttons = [(icon::DECREASE, "−", -step), (icon::INCREASE, "+", step)];
    for (i, (glyph, label, delta)) in buttons.into_iter().enumerate() {
        let left = rect.right() - (2 - i) as f32 * button_width;
        let button_rect = Rect::from_min_max(
            pos2(left, rect.top()),
            pos2(left + button_width, rect.bottom()),
        );
        let button = ui.interact(button_rect, response.id.with(label), Sense::click());
        button.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), label));
        if button.clicked() {
            let current = pystr::strip(text)
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .unwrap_or(min);
            *text = format!("{:.*}", decimals(step), (current + delta).clamp(min, max));
            response.mark_changed();
        }
        let painter = ui.painter();
        painter.vline(
            left,
            rect.y_range().shrink(1.0),
            Stroke::new(1.0, theme::CONTROL_BORDER),
        );
        let color = if button.hovered() {
            theme::TEXT
        } else {
            theme::NAV_TEXT
        };
        painter.text(
            button_rect.center(),
            Align2::CENTER_CENTER,
            glyph,
            theme::icons(11.0),
            color,
        );
        focus_ring(ui, &button, theme::CONTROL_RADIUS);
    }
    underline(ui, rect, response.has_focus());
    response
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
