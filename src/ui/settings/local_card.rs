//! 本地模型頁三張卡共用的版面：卡片外框與標題列、內容區內距。

use eframe::egui::{Color32, Frame, Label, Margin, RichText, Ui};

use super::theme;
use super::widgets::{STACK_GAP, card, right_aligned, right_aligned_width};

/// 卡片：標題列（左標題、右 `trailing`，放不下時 `trailing` 換到標題下一行）加 `body`。
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
                let font = theme::bold(theme::PROVIDER_TITLE_SIZE);
                let width = ui
                    .painter()
                    .layout_no_wrap(title.to_owned(), font.clone(), Color32::PLACEHOLDER)
                    .size()
                    .x;
                let title = Label::new(RichText::new(title).font(font));
                let trailing = |ui: &mut Ui| {
                    right_aligned(ui, id, |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        trailing(ui);
                    })
                };
                // 標題要跟右側同列置中，不走 text_with_controls 的固定寬度文字區
                if ui.available_width() - right_aligned_width(ui, id) - 12.0 >= width {
                    ui.horizontal(|ui| {
                        ui.add(title);
                        trailing(ui);
                    });
                } else {
                    ui.add(title);
                    ui.add_space(STACK_GAP);
                    ui.horizontal(trailing);
                }
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
