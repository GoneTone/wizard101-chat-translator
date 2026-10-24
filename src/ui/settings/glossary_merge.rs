//! 匯入檔案併入目前術語表前的確認對話框：統計、逐列勾選的衝突列表，按〔合併〕才寫入。

use eframe::egui::{Frame, Label, Margin, RichText, ScrollArea, Ui};

use super::dialog::{Choice, DialogSpec, dialog};
use super::theme;
use super::widgets::{fixed_cell, primary_button, secondary_button};
use crate::glossary::{Entry, MergePlan, apply_merge};
use crate::i18n::t;
use crate::logs::log;

/// 衝突列表的最大高度；超過就只捲動這一段。
const LIST_HEIGHT: f32 = 240.0;

/// 合併確認對話框的狀態：規劃結果與每筆衝突是否改用檔案的譯名。
pub(super) struct MergeDialog {
    plan: MergePlan,
    take: Vec<bool>,
}

impl MergeDialog {
    pub(super) fn new(plan: MergePlan) -> Self {
        let take = vec![false; plan.conflicts.len()];
        Self { plan, take }
    }

    /// 依勾選把檔案併入 `entries`。
    pub(super) fn apply(&self, entries: &mut Vec<Entry>) {
        apply_merge(entries, &self.plan, &self.take);
        log(format!(
            "[settings] glossary merge: added={} identical={} conflicts={} taken={}",
            self.plan.added.len(),
            self.plan.identical,
            self.plan.conflicts.len(),
            self.take.iter().filter(|t| **t).count()
        ));
    }

    /// 畫一幀；按〔合併〕回 `Done`。
    pub(super) fn ui(&mut self, ui: &Ui) -> Choice<()> {
        let spec = DialogSpec {
            salt: "glossary_merge",
            width: theme::SERVICE_DIALOG_WIDTH,
            title: t("glossary.merge_title", &[]),
            closable: true,
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
                    .show(ui, |ui| self.body(ui));
            },
            |ui| {
                let cancel = secondary_button(ui, &t("button.cancel", &[])).clicked();
                let merge = primary_button(ui, &t("button.merge", &[])).clicked();
                (merge, cancel)
            },
        );
        match shown.footer {
            (true, _) => Choice::Done(()),
            (_, true) => Choice::Cancel,
            _ if shown.cancel => Choice::Cancel,
            _ => Choice::Open,
        }
    }

    fn body(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.y = 6.0;
        let plan = &self.plan;
        let count = |n: usize| n.to_string();
        let total = plan.added.len() + plan.identical + plan.conflicts.len();
        let lines = [
            t("glossary.merge_total", &[("total", &count(total))]),
            t(
                "glossary.merge_added",
                &[("added", &count(plan.added.len()))],
            ),
            t(
                "glossary.merge_identical",
                &[("identical", &count(plan.identical))],
            ),
            t(
                "glossary.merge_conflicts",
                &[("n", &count(plan.conflicts.len()))],
            ),
        ];
        for line in lines {
            ui.add(Label::new(
                RichText::new(line).size(theme::CONTROL_TEXT_SIZE),
            ));
        }
        if plan.conflicts.is_empty() {
            return;
        }
        ui.add_space(6.0);
        ui.add(
            Label::new(
                RichText::new(t("glossary.merge_hint", &[]))
                    .size(theme::HINT_SIZE)
                    .color(theme::TEXT_SECONDARY),
            )
            .wrap(),
        );
        let gap = ui.spacing().item_spacing.x;
        let width = (ui.available_width() - 2.0 * gap) / 3.0;
        ui.horizontal(|ui| {
            let mut all = self.take.iter().all(|t| *t);
            fixed_cell(ui, width, |ui| {
                // 部分勾選時顯示未勾，按下即全勾
                if ui
                    .checkbox(&mut all, t("glossary.select_all", &[]))
                    .changed()
                {
                    self.take.fill(all);
                }
            });
            for key in ["glossary.current_target", "glossary.file_target"] {
                let text = RichText::new(t(key, &[]))
                    .size(theme::HINT_SIZE)
                    .color(theme::TEXT_SECONDARY);
                fixed_cell(ui, width, |ui| ui.add(Label::new(text)));
            }
        });
        ScrollArea::vertical()
            .id_salt("glossary_merge_rows")
            .max_height(LIST_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (conflict, take) in self.plan.conflicts.iter().zip(&mut self.take) {
                    ui.horizontal(|ui| {
                        fixed_cell(ui, width, |ui| ui.checkbox(take, &conflict.current.source));
                        for text in [&conflict.current.target, &conflict.incoming.target] {
                            fixed_cell(ui, width, |ui| ui.add(Label::new(text).wrap()));
                        }
                    });
                }
            });
    }
}
