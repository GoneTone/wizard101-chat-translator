//! 本地模型頁三張卡共用的版面：卡片外框與標題列、內容區內距、刪除確認框。

use eframe::egui::{Frame, Label, Margin, Response, RichText, Ui};

use super::dialog::{Choice, confirm};
use super::theme;
use super::widgets::{card, danger_button, right_aligned};
use crate::i18n::t;

/// 卡片：標題列（左標題、右 `trailing`）加 `body`。
pub(super) fn section(
    ui: &mut Ui,
    title: &str,
    trailing: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui),
) {
    card(ui, |ui| {
        let id = ui.scope_id().with(("local_section", title));
        Frame::new()
            .inner_margin(Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let title = RichText::new(title).font(theme::bold(theme::PROVIDER_TITLE_SIZE));
                    ui.add(Label::new(title));
                    right_aligned(ui, id, |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        trailing(ui);
                    });
                });
            });
        body(ui);
    });
}

/// 卡片內容區：左右內距，底部留白。
pub(super) fn padded(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .inner_margin(Margin {
            left: 16,
            right: 16,
            top: 0,
            bottom: 12,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui);
        });
}

/// 確認框，確定鈕是紅色的〔刪除〕；`salt` 區分同時存在的不同確認框。
pub(super) fn confirm_danger(ui: &Ui, salt: &'static str, question: String) -> Choice<()> {
    confirm(ui, salt, question, |ui| -> Response {
        danger_button(ui, &t("button.delete", &[]))
    })
}
