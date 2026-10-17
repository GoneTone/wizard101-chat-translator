//! 設定介面的輸入欄位：可自行輸入的下拉與帶 −／＋ 的數值欄。

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    Align2, Button, Event, Frame, Id, InputState, Key, LayerId, Popup, PopupKind, Rect, Response,
    Role, Sense, Stroke, StrokeKind, SurrenderFocusOn, TextEdit, Ui, WidgetInfo, pos2, vec2,
};

use crate::py::pystr;
use crate::ui::settings::theme::{self, icon};
use crate::ui::settings::widgets::{filter_models, focus_ring, name_by_row};

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
        // 彈出層的 Ui 從全域樣式建立，要帶上設定視窗自己的樣式
        Popup::new(popup_id, ctx.clone(), rect, ui.layer_id())
            .kind(PopupKind::Popup)
            .style((**ui.style()).clone())
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
