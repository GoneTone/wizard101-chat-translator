//! 設定介面的 modal 對話框外殼：標題列、可捲動的內容與底部按鈕列，以及共用的刪除確認對話框。

use eframe::egui::{
    Align, Align2, CornerRadius, Frame, Id, Key, Label, Layout, Margin, Modal, Modifiers, Response,
    RichText, ScrollArea, Stroke, Ui, UiKind, vec2,
};

use super::theme::{self, icon};
use super::widgets::{danger_button, icon_button, right_aligned, secondary_button};
use crate::i18n::t;

/// 對話框與視窗邊緣的最小距離。
const MARGIN: f32 = 24.0;
// 左右與內容、按鈕列同為 20，標題才會和內文對齊
const HEADER_MARGIN: Margin = Margin {
    left: 20,
    right: 20,
    top: 18,
    bottom: 8,
};
const FOOTER_MARGIN: Margin = Margin::symmetric(20, 14);
const HEADER_HEIGHT: f32 =
    (HEADER_MARGIN.top as f32) + theme::CONTROL_HEIGHT + (HEADER_MARGIN.bottom as f32);
const FOOTER_HEIGHT: f32 =
    (FOOTER_MARGIN.top as f32) + theme::CONTROL_HEIGHT + (FOOTER_MARGIN.bottom as f32);

/// 對話框的固定設定。
pub(super) struct DialogSpec {
    /// 對話框的 Id 來源。
    pub salt: &'static str,
    pub width: f32,
    pub title: String,
    /// 標題列右側有關閉鈕。
    pub closable: bool,
    /// 上緣固定在視窗頂端下方；否則置中。
    pub pinned_top: bool,
}

/// `dialog` 這一幀的結果：內容與按鈕列各自的回傳值，以及是否按了 Esc 或關閉鈕。
pub(super) struct Shown<B, F> {
    pub body: B,
    pub footer: F,
    pub cancel: bool,
}

/// modal 對話框：標題列、可捲動的內容、靠右的底部按鈕列（依加入順序由左往右）；點遮罩不關閉，只有最上層的對話框收 Esc。
pub(super) fn dialog<B, F>(
    parent: &Ui,
    spec: DialogSpec,
    body: impl FnOnce(&mut Ui) -> B,
    footer: impl FnOnce(&mut Ui) -> F,
) -> Shown<B, F> {
    let ctx = parent.ctx().clone();
    // 對話框的 Ui 從全域樣式建立，要帶上設定介面自己的樣式
    let style = parent.style().clone();
    let frame = Frame::new()
        .fill(theme::CARD_BG)
        .stroke(Stroke::new(1.0, theme::CONTROL_BORDER))
        .corner_radius(theme::DIALOG_RADIUS)
        .shadow(style.visuals.window_shadow);
    let max_body = ctx.content_rect().height() - 2.0 * MARGIN - HEADER_HEIGHT - FOOTER_HEIGHT;
    let id = Id::unique(("settings_dialog", spec.salt));
    let mut area = Modal::default_area(id);
    if spec.pinned_top {
        area = area.anchor(Align2::CENTER_TOP, vec2(0.0, MARGIN));
    }
    let modal = Modal::new(id)
        .area(area)
        .backdrop_color(theme::SCRIM)
        .frame(frame)
        .show(&ctx, |ui| {
            ui.set_style(style);
            name_modal(ui, &spec.title);
            ui.set_width(spec.width - 2.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            let close = header(ui, &spec.title, spec.closable);
            let body = ScrollArea::vertical()
                .max_height(max_body.max(120.0))
                .auto_shrink([false, true])
                .show(ui, body)
                .inner;
            let foot = Frame::new()
                .fill(theme::MODAL_FOOT)
                .corner_radius(CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: theme::DIALOG_RADIUS,
                    se: theme::DIALOG_RADIUS,
                })
                .inner_margin(FOOTER_MARGIN)
                .show(ui, |ui| {
                    let size = vec2(ui.available_width(), theme::CONTROL_HEIGHT);
                    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        right_aligned(ui, id.with("footer"), footer)
                    })
                    .inner
                });
            let line = foot.response.rect;
            ui.painter().hline(
                line.x_range(),
                line.top(),
                Stroke::new(1.0, theme::CARD_BORDER),
            );
            (body, foot.inner, close)
        });
    let escape = modal.is_top_modal
        && !modal.any_popup_open
        && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let (body, footer, close) = modal.inner;
    Shown {
        body,
        footer,
        cancel: close || escape,
    }
}

/// 對話框這一幀的結果。
pub(super) enum Choice<T> {
    Open,
    Cancel,
    Done(T),
}

/// 刪除確認對話框：顯示 `question`，按紅色的〔刪除〕才算確定；`salt` 區分同時存在的不同確認框。
pub(super) fn confirm_delete(ui: &Ui, salt: &'static str, question: String) -> Choice<()> {
    confirm(ui, salt, question, |ui| {
        danger_button(ui, &t("button.delete", &[]))
    })
}

/// 確認對話框：顯示 `question`，按 `ok` 畫出的按鈕才算確定；`salt` 區分同時存在的不同確認框。
pub(super) fn confirm(
    ui: &Ui,
    salt: &'static str,
    question: String,
    ok: impl FnOnce(&mut Ui) -> Response,
) -> Choice<()> {
    let spec = DialogSpec {
        salt,
        width: theme::CONFIRM_DIALOG_WIDTH,
        title: t("dialog.confirm_title", &[]),
        closable: false,
        pinned_top: false,
    };
    let shown = dialog(
        ui,
        spec,
        |ui| {
            Frame::new()
                .inner_margin(Margin {
                    left: 20,
                    right: 20,
                    top: 4,
                    bottom: 16,
                })
                .show(ui, |ui| {
                    ui.add(
                        Label::new(RichText::new(question).size(theme::CONTROL_TEXT_SIZE)).wrap(),
                    );
                });
        },
        |ui| {
            let cancel = secondary_button(ui, &t("button.cancel", &[])).clicked();
            let ok = ok(ui).clicked();
            (ok, cancel)
        },
    );
    let (ok, cancel) = shown.footer;
    if ok {
        Choice::Done(())
    } else if cancel || shown.cancel {
        Choice::Cancel
    } else {
        Choice::Open
    }
}

/// 以標題當對話框的無障礙名稱。
fn name_modal(ui: &Ui, title: &str) {
    let modal = ui
        .stack()
        .iter()
        .find(|frame| frame.kind() == Some(UiKind::Modal))
        .map(|frame| frame.unique_id);
    if let Some(id) = modal {
        ui.ctx()
            .accesskit_node_builder(id, |node| node.set_label(title));
    }
}

/// 標題列；回傳是否按了關閉鈕。
fn header(ui: &mut Ui, title: &str, closable: bool) -> bool {
    Frame::new()
        .inner_margin(HEADER_MARGIN)
        .show(ui, |ui| {
            let size = vec2(ui.available_width(), theme::CONTROL_HEIGHT);
            ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
                ui.add(Label::new(RichText::new(title).font(theme::bold(17.0))).truncate());
                closable
                    && ui
                        .with_layout(Layout::right_to_left(Align::Center), |ui| {
                            icon_button(ui, icon::CLOSE, &t("tooltip.close", &[])).clicked()
                        })
                        .inner
            })
            .inner
        })
        .inner
}
