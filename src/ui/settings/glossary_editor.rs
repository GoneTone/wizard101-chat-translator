//! 術語表的編輯畫面：名稱、引入的內建術語表、搜尋、自訂條目與唯讀的內建條目。

use eframe::egui::{Color32, Frame, Grid, Id, Label, Margin, RichText, Ui, vec2};

use super::fields::text_field;
use super::glossary_draft::builtin_of;
use super::theme::{self, icon};
use super::widgets::{
    card, collapsible, dashed_button, icon_button, optional_combo, right_aligned,
    right_aligned_width, secondary_button, secondary_icon_button, setting_row_with,
};
use crate::config::glossaries::StoredGlossary;
use crate::glossary::builtin::builtin_codes;
use crate::glossary::{Entry, RowIssue, fold, overridden, review};
use crate::i18n::{language_name, t};
use crate::logs::log;
use crate::py::pystr;

/// 編輯畫面要外層處理的操作。
pub(super) enum EditorAction {
    None,
    Back,
    Import,
    Export,
}

/// 一份術語表的編輯畫面；衝突提示只在條目或引入的內建改變時重算。
pub(super) struct GlossaryEditor {
    id: String,
    query: String,
    /// 上次檢查時的條目與引入代碼。
    checked: Option<(Vec<Entry>, Option<String>)>,
    builtin: Vec<Entry>,
    issues: Vec<RowIssue>,
    overridden: Vec<bool>,
}

impl GlossaryEditor {
    pub(super) fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            query: String::new(),
            checked: None,
            builtin: Vec::new(),
            issues: Vec::new(),
            overridden: Vec::new(),
        }
    }

    pub(super) fn id(&self) -> &str {
        &self.id
    }

    /// 畫編輯畫面並直接改 `glossary`；回傳這一幀按下的操作。
    pub(super) fn ui(&mut self, ui: &mut Ui, glossary: &mut StoredGlossary) -> EditorAction {
        self.refresh(glossary);
        let back = secondary_icon_button(ui, icon::BACK, &t("glossary.back_to_list", &[]));
        card(ui, |ui| {
            let blank = pystr::strip(&glossary.name).is_empty();
            let notice = |ui: &mut Ui| {
                if blank {
                    hint(ui, &t("error.need_name", &[]), theme::ERROR);
                }
            };
            setting_row_with(ui, &t("glossary.name", &[]), notice, |ui| {
                let size = vec2(ui.spacing().combo_width, theme::CONTROL_HEIGHT);
                ui.allocate_ui(size, |ui| text_field(ui, &mut glossary.name));
            });
            self.builtin_row(ui, glossary);
        });
        let (import, export) = self.search_row(ui);
        self.custom_card(ui, &mut glossary.entries);
        if let Some(code) = glossary.builtin.as_deref() {
            self.builtin_card(ui, code);
        }
        if back.clicked() {
            EditorAction::Back
        } else if import {
            EditorAction::Import
        } else if export {
            EditorAction::Export
        } else {
            EditorAction::None
        }
    }

    fn refresh(&mut self, glossary: &StoredGlossary) {
        let current = (&glossary.entries, &glossary.builtin);
        if self
            .checked
            .as_ref()
            .is_some_and(|(e, b)| (e, b) == current)
        {
            return;
        }
        self.builtin = builtin_of(glossary);
        self.issues = review(&glossary.entries, &self.builtin);
        self.overridden = overridden(&glossary.entries, &self.builtin);
        self.checked = Some((glossary.entries.clone(), glossary.builtin.clone()));
    }

    fn builtin_row(&self, ui: &mut Ui, glossary: &mut StoredGlossary) {
        let codes = builtin_codes();
        let missing = glossary
            .builtin
            .as_deref()
            .is_some_and(|code| !codes.contains(&code));
        let none = t("glossary.builtin_none", &[]);
        let selected = glossary
            .builtin
            .as_deref()
            .map_or(none.clone(), language_name);
        let options: Vec<(String, String)> = codes
            .iter()
            .map(|code| ((*code).into(), language_name(code)))
            .collect();
        let notice = |ui: &mut Ui| {
            if missing {
                hint(ui, &t("glossary.builtin_missing", &[]), theme::WARN_TEXT);
            }
        };
        setting_row_with(ui, &t("glossary.builtin", &[]), notice, |ui| {
            let current = glossary.builtin.as_deref();
            let salt = ("glossary_builtin", &self.id);
            if let Some(code) = optional_combo(ui, salt, selected, &none, &options, current) {
                log(format!(
                    "[settings] glossary {} builtin set to {}",
                    self.id,
                    code.as_deref().unwrap_or("none")
                ));
                glossary.builtin = code;
            }
        });
    }

    /// 搜尋框與右側的〔匯入檔案〕〔匯出〕；回傳兩鈕是否按下。
    fn search_row(&mut self, ui: &mut Ui) -> (bool, bool) {
        let buttons = Id::unique(("glossary_editor_buttons", &self.id));
        ui.horizontal(|ui| {
            let label = ui.add(Label::new(t("glossary.search", &[]))).id;
            let gap = ui.spacing().item_spacing.x;
            let width = ui.available_width() - right_aligned_width(ui, buttons) - gap;
            let size = vec2(width.max(80.0), theme::CONTROL_HEIGHT);
            ui.allocate_ui(size, |ui| {
                text_field(ui, &mut self.query).labelled_by(label);
            });
            right_aligned(ui, buttons, |ui| {
                (
                    secondary_button(ui, &t("glossary.import", &[])).clicked(),
                    secondary_button(ui, &t("glossary.export", &[])).clicked(),
                )
            })
        })
        .inner
    }

    fn custom_card(&self, ui: &mut Ui, entries: &mut Vec<Entry>) {
        let query = fold(&self.query);
        let mut removed = None;
        let mut added = false;
        card(ui, |ui| {
            Frame::new()
                .inner_margin(Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    let title = RichText::new(t("glossary.custom_entries", &[]))
                        .font(theme::bold(theme::TEXT_SIZE));
                    ui.add(Label::new(title));
                    let gap = ui.spacing().item_spacing.x;
                    let column = (ui.available_width() - theme::CONTROL_HEIGHT - 2.0 * gap) / 2.0;
                    let cell = vec2(column, theme::CONTROL_HEIGHT);
                    let (source, target) = ui
                        .horizontal(|ui| {
                            let caption = |ui: &mut Ui, key| {
                                let text = RichText::new(t(key, &[]))
                                    .size(theme::HINT_SIZE)
                                    .color(theme::TEXT_SECONDARY);
                                ui.allocate_ui(vec2(column, 0.0), |ui| ui.add(Label::new(text)))
                                    .inner
                                    .id
                            };
                            (
                                caption(ui, "glossary.source"),
                                caption(ui, "glossary.target"),
                            )
                        })
                        .inner;
                    let delete = t("glossary.delete_entry", &[]);
                    for (i, entry) in entries.iter_mut().enumerate() {
                        if !matches(entry, &query) {
                            continue;
                        }
                        // 以條目位置當 Id：篩選與提示增減時，游標與復原紀錄才不會跑到別列
                        ui.push_id(i, |ui| {
                            ui.horizontal(|ui| {
                                ui.allocate_ui(cell, |ui| {
                                    text_field(ui, &mut entry.source).labelled_by(source);
                                });
                                ui.allocate_ui(cell, |ui| {
                                    text_field(ui, &mut entry.target).labelled_by(target);
                                });
                                if icon_button(ui, icon::CLOSE, &delete).clicked() {
                                    removed = Some(i);
                                }
                            });
                            if entry.is_blank() {
                                return;
                            }
                            if let Some((text, color)) = row_hint(&self.issues[i]) {
                                hint(ui, &text, color);
                            }
                        });
                    }
                    added = dashed_button(ui, &t("button.add_entry", &[]), 36.0).clicked();
                });
        });
        if let Some(i) = removed {
            entries.remove(i);
        }
        if added {
            entries.push(Entry {
                source: String::new(),
                target: String::new(),
            });
        }
    }

    /// 最下方預設收起的唯讀內建條目；沒有條目（含代碼不存在）時不畫。
    fn builtin_card(&self, ui: &mut Ui, code: &str) {
        if self.builtin.is_empty() {
            return;
        }
        let query = fold(&self.query);
        let count = self.builtin.len().to_string();
        let language = language_name(code);
        let title = t(
            "glossary.builtin_section",
            &[("language", &language), ("count", &count)],
        );
        let tag = t("glossary.overridden", &[]);
        card(ui, |ui| {
            let id = Id::unique(("glossary_builtin_section", &self.id));
            collapsible(ui, id, &title, None, false, |ui| {
                Frame::new()
                    .inner_margin(Margin {
                        left: 16,
                        right: 16,
                        top: 0,
                        bottom: 12,
                    })
                    .show(ui, |ui| {
                        Grid::new(id.with("rows"))
                            .num_columns(3)
                            .spacing(vec2(16.0, 6.0))
                            .show(ui, |ui| {
                                let rows = self.builtin.iter().zip(&self.overridden);
                                for (entry, overridden) in rows {
                                    if !matches(entry, &query) {
                                        continue;
                                    }
                                    ui.label(&entry.source);
                                    ui.label(&entry.target);
                                    if *overridden {
                                        ui.label(
                                            RichText::new(&tag)
                                                .size(theme::HINT_SIZE)
                                                .color(theme::TEXT_MUTED),
                                        );
                                    }
                                    ui.end_row();
                                }
                            });
                    });
            });
        });
    }
}

/// 條目的任一側（fold 後）包含 `query`；`query` 空白、或有一側還沒填（剛新增的列）時都算。
fn matches(entry: &Entry, query: &str) -> bool {
    let (source, target) = (fold(&entry.source), fold(&entry.target));
    query.is_empty()
        || source.is_empty()
        || target.is_empty()
        || source.contains(query)
        || target.contains(query)
}

/// 自訂條目那一列的提示文字與顏色：與內建不同是警告色，其餘問題是錯誤色。
fn row_hint(issue: &RowIssue) -> Option<(String, Color32)> {
    let (text, color) = match issue {
        RowIssue::Ok => return None,
        RowIssue::BuiltinDiffers { other } => (
            t("glossary.builtin_differs", &[("other", other)]),
            theme::WARN_TEXT,
        ),
        RowIssue::SameAsBuiltin => (t("glossary.same_as_builtin", &[]), theme::ERROR),
        RowIssue::DuplicateSource => (t("glossary.row_duplicate_source", &[]), theme::ERROR),
        RowIssue::Incomplete => (t("glossary.row_incomplete", &[]), theme::ERROR),
    };
    Some((text, color))
}

fn hint(ui: &mut Ui, text: &str, color: Color32) {
    let text = RichText::new(text).size(theme::HINT_SIZE).color(color);
    ui.add(Label::new(text).wrap());
}
