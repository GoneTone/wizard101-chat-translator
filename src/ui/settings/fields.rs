//! 設定介面的輸入欄位：單行輸入框、金鑰欄、可自行輸入的下拉與帶 −／＋ 的數值欄。

use eframe::egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    self, Align, Align2, Button, Color32, CornerRadius, Event, Frame, Id, InputState, Key, LayerId,
    Popup, PopupKind, Rect, Response, Role, Sense, Stroke, StrokeKind, SurrenderFocusOn, TextEdit,
    Ui, WidgetInfo, pos2, vec2,
};

use crate::config::parse_float;
use crate::py::pystr;
use crate::ui::settings::theme::{self, icon};
use crate::ui::settings::widgets::{
    INLINE_ICON_INSET, best_match, focus_ring, inline_icon_button, match_range, name_by_row,
    paint_chevron, popup_style,
};

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
    /// 下一次畫清單時捲到最符合的項目（剛展開或剛打字）。
    jump: bool,
}

/// 可自行輸入的下拉：點輸入框、打字或按 ↓ 展開，清單一律列出全部選項。
///
/// 每一項裡與輸入相符的字上色，並捲到最符合的項目（同 `best_match`），輸入清空時捲回最上面。
/// ↑↓ 選、Enter 選定、Esc 收起，點別處收起。寬度取 `spacing.combo_width`。
/// 不在 `setting_row` 裡時，呼叫端要對回傳值 `labelled_by`／`accessible_name`。
pub fn editable_combo(ui: &mut Ui, id: Id, text: &mut String, options: &[String]) -> Response {
    let mut state: ComboState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let edit_id = id.with("edit");
    let popup_id = id.with("popup");
    let was_visible = state.open && !options.is_empty();
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

    // 點清單時不讓輸入框失焦，否則清單會在點擊生效前收起
    let popup_layer = LayerId::new(PopupKind::Popup.order(), popup_id);
    let ctx = ui.ctx().clone();
    let pointer_on_popup = was_visible
        && ctx
            .pointer_interact_pos()
            .and_then(|pos| ctx.layer_id_at(pos))
            == Some(popup_layer);
    let (rect, (mut response, chevron)) = control_box(ui, ui.spacing().combo_width, |ui, rect| {
        let chevron_rect = Rect::from_min_max(
            pos2(rect.right() - 2.0 * theme::CHEVRON_INSET, rect.top()),
            rect.max,
        );
        let edit_rect = Rect::from_min_max(
            pos2(rect.left() + 10.0, rect.top()),
            chevron_rect.left_bottom(),
        );
        let surrender = ctx.options(|o| o.input_options.surrender_focus_on);
        if pointer_on_popup {
            ctx.options_mut(|o| o.input_options.surrender_focus_on = SurrenderFocusOn::Never);
        }
        let response = ui.put(
            edit_rect,
            TextEdit::singleline(text)
                .id(edit_id)
                .frame(Frame::NONE)
                .align(Align2::LEFT_CENTER),
        );
        ctx.options_mut(|o| o.input_options.surrender_focus_on = surrender);
        let chevron = ui.interact(chevron_rect, id.with("chevron"), Sense::CLICK);
        paint_chevron(ui, chevron_rect.center());
        (name_by_row(ui, response), chevron)
    });
    if chevron.clicked() {
        response.request_focus();
        state.open = !state.open;
    }
    if response.clicked() || down > 0 {
        state.jump |= !state.open;
        state.open = true;
    }
    if response.changed() {
        state.open = true;
        state.hot = None;
        state.jump = true;
    }
    let list = options;
    let best = best_match(list, text);
    let jump_to = if pystr::strip(text).is_empty() {
        Some(0)
    } else {
        best
    };
    if let Some(last) = list.len().checked_sub(1) {
        for _ in 0..down {
            // 還沒選項目時從最符合的那一項開始，不從清單頂端
            state.hot = Some(state.hot.map_or(best.unwrap_or(0), |h| (h + 1).min(last)));
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

    let keyed = down + up > 0;
    let visible = state.open && picked.is_none() && !list.is_empty();
    if visible {
        // 彈出層的 Ui 從全域樣式建立，要帶上設定視窗自己的樣式
        Popup::new(popup_id, ctx.clone(), rect, ui.layer_id())
            .kind(PopupKind::Popup)
            .style(popup_style(ui))
            .open(true)
            .width(rect.width())
            .frame(Frame::popup(ui.style()).inner_margin(4))
            .show(|ui| {
                let moved = ui.input(|i| i.pointer.is_moving());
                // 超出上限時固定撐滿上限：預設的 64 px 下限加上彈出層沿用上一幀的大小，篩選放寬後會卡在兩列高
                egui::ScrollArea::vertical()
                    .max_height(ui.spacing().combo_height)
                    .min_scrolled_height(ui.spacing().combo_height)
                    .show(ui, |ui| {
                        for (i, option) in list.iter().enumerate() {
                            let label = option_label(ui, option, text);
                            let item = ui.add(
                                Button::selectable(state.hot == Some(i), label)
                                    .min_size(vec2(ui.available_width(), theme::CONTROL_HEIGHT)),
                            );
                            if item.hovered() && moved {
                                state.hot = Some(i);
                            }
                            if item.clicked() {
                                picked = Some(option.clone());
                            }
                            if keyed && state.hot == Some(i) {
                                item.scroll_to_me(None);
                            } else if !keyed && state.jump && jump_to == Some(i) {
                                item.scroll_to_me(Some(Align::TOP));
                            }
                        }
                    });
            });
    }
    if visible {
        state.jump = false;
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
    underline(ui, rect, response.has_focus());
    response
}

/// 下拉項目的文字：`query` 相符的那段用強調色，其餘沿用按鈕的文字色（hover 時照常變色）。
fn option_label(ui: &Ui, option: &str, query: &str) -> LayoutJob {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let plain = TextFormat::simple(font.clone(), Color32::PLACEHOLDER);
    let mut job = LayoutJob::default();
    match match_range(option, query) {
        Some(range) => {
            job.append(&option[..range.start], 0.0, plain.clone());
            job.append(
                &option[range.clone()],
                0.0,
                TextFormat::simple(font, theme::ACCENT),
            );
            job.append(&option[range.end..], 0.0, plain);
        }
        None => job.append(option, 0.0, plain),
    }
    job
}

/// 單行輸入框，寬度占滿。不在 `setting_row` 裡時，呼叫端要對回傳值命名。
pub fn text_field(ui: &mut Ui, text: &mut String) -> Response {
    let (rect, response) = control_box(ui, ui.available_width(), |ui, rect| {
        boxed_edit(ui, rect.shrink2(vec2(10.0, 0.0)), text, false)
    });
    underline(ui, rect, response.has_focus());
    response
}

/// 預設遮住的金鑰欄：框內右側的眼睛鈕切換 `revealed`，`tooltip` 是它的提示與無障礙名稱。
///
/// 不在 `setting_row` 裡時，呼叫端要對回傳值命名。
pub fn secret_field(
    ui: &mut Ui,
    text: &mut String,
    revealed: &mut bool,
    show_tooltip: &str,
    hide_tooltip: &str,
) -> Response {
    let (rect, response) = control_box(ui, ui.available_width(), |ui, rect| {
        let side = theme::CONTROL_HEIGHT - 2.0 * INLINE_ICON_INSET;
        let eye = Rect::from_min_size(
            pos2(
                rect.right() - INLINE_ICON_INSET - side,
                rect.center().y - side / 2.0,
            ),
            vec2(side, side),
        );
        let edit_rect = Rect::from_min_max(
            pos2(rect.left() + 10.0, rect.top()),
            pos2(eye.left() - INLINE_ICON_INSET, rect.bottom()),
        );
        let response = boxed_edit(ui, edit_rect, text, !*revealed);
        let (glyph, tooltip) = if *revealed {
            (icon::HIDE_KEY, hide_tooltip)
        } else {
            (icon::SHOW_KEY, show_tooltip)
        };
        if ui
            .put(eye, |ui: &mut Ui| inline_icon_button(ui, glyph, tooltip))
            .clicked()
        {
            *revealed = !*revealed;
        }
        response
    });
    underline(ui, rect, response.has_focus());
    response
}

/// 控制項的底：配置 `width`×控制項高、畫底色與外框（hover 用 `theme::apply` 的 hovered 樣式），
/// 再交給 `contents` 把輸入框等放進範圍內；回傳範圍與 `contents` 的結果。
fn control_box<R>(ui: &mut Ui, width: f32, contents: impl FnOnce(&mut Ui, Rect) -> R) -> (Rect, R) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, theme::CONTROL_HEIGHT), Sense::hover());
    let visuals = if ui.is_enabled() && ui.rect_contains_pointer(rect) {
        ui.visuals().widgets.hovered
    } else {
        ui.visuals().widgets.inactive
    };
    ui.painter().rect(
        rect,
        theme::CONTROL_RADIUS,
        visuals.bg_fill,
        visuals.bg_stroke,
        StrokeKind::Inside,
    );
    let inner = contents(ui, rect);
    // `ui.put` 會把水平排版的游標拉回放入處，要推回整個控制項之後，下一個元件才不會疊上來
    ui.advance_cursor_after_rect(rect);
    (rect, inner)
}

/// 放在控制項底上的無框輸入框，占滿 `rect`。
fn boxed_edit(ui: &mut Ui, rect: Rect, text: &mut String, password: bool) -> Response {
    let response = ui.put(
        rect,
        TextEdit::singleline(text)
            .password(password)
            .desired_width(f32::INFINITY)
            .frame(Frame::NONE)
            .align(Align2::LEFT_CENTER),
    );
    // egui 只在打字或移動游標時捲動，Tab 跳到畫面外的欄位要自己捲進來
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
    name_by_row(ui, response)
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

/// `value` 夾在 `range` 內，依 `step` 的小數位數寫成文字。
fn clamped(value: f64, step: f64, (min, max): (f64, f64)) -> String {
    format!("{:.*}", decimals(step), value.max(min).min(max))
}

/// 數值欄：輸入框加 −／＋ 兩鈕；按鈕依 `step` 增減並夾在 `range` 內，打的數字超出範圍時失焦就夾回來。
///
/// 輸入框的字不是數字時不在這裡驗證，按鈕從 `range` 的下限起算。不在 `setting_row` 裡時，呼叫端要對回傳值命名。
pub fn number_field(ui: &mut Ui, text: &mut String, step: f64, range: (f64, f64)) -> Response {
    let (min, max) = range;
    let button_width = 30.0;
    let (rect, response) = control_box(ui, theme::NUMBER_WIDTH, |ui, rect| {
        let edit_rect = Rect::from_min_max(
            pos2(rect.left() + 10.0, rect.top()),
            pos2(rect.right() - 2.0 * button_width, rect.bottom()),
        );
        let mut response = boxed_edit(ui, edit_rect, text, false);
        // 範圍內的值不重排：照 `step` 的位數重寫會把多打的小數位捨掉
        if response.lost_focus()
            && let Some(value) = parse_float(text).filter(|v| *v < min || *v > max)
        {
            *text = clamped(value, step, range);
            response.mark_changed();
        }
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
                *text = clamped(parse_float(text).unwrap_or(min) + delta, step, range);
                response.mark_changed();
            }
            let painter = ui.painter();
            let hovered = button.hovered();
            if hovered {
                // 最右邊那一鈕的右側兩角跟著控制項是圓的
                let r = theme::CONTROL_RADIUS - 1;
                let corners = if i == 1 {
                    CornerRadius {
                        ne: r,
                        se: r,
                        ..CornerRadius::ZERO
                    }
                } else {
                    CornerRadius::ZERO
                };
                painter.rect_filled(button_rect.shrink(1.0), corners, theme::HOT_BG);
            }
            painter.vline(
                left,
                rect.y_range().shrink(1.0),
                Stroke::new(1.0, theme::CONTROL_BORDER),
            );
            let color = if hovered {
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
        response
    });
    underline(ui, rect, response.has_focus());
    response
}
