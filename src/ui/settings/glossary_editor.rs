//! 術語表的編輯畫面：名稱、建立者、引入的內建術語表、搜尋、自訂條目與唯讀的內建條目。

use eframe::egui::{Color32, Frame, Id, Label, Margin, RichText, TextStyle, Ui, vec2};

use super::fields::text_field;
use super::glossary_draft::builtin_of;
use super::theme::{self, icon};
use super::widgets::{
    LazyRows, card, collapsible, dashed_button, fixed_cell, icon_button, optional_combo, rich,
    right_aligned, right_aligned_width, secondary_button, secondary_icon_button, setting_row_with,
};
use crate::config::glossaries::StoredGlossary;
use crate::glossary::builtin::{builtin_codes, builtin_creators};
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
    custom_rows: LazyRows,
    builtin_rows: LazyRows,
    /// 每刪一列加一，讓各列換新 Id。
    deletions: u64,
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
            custom_rows: LazyRows::default(),
            builtin_rows: LazyRows::default(),
            deletions: 0,
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
            let size = vec2(ui.spacing().combo_width, theme::CONTROL_HEIGHT);
            setting_row_with(ui, &t("glossary.name", &[]), notice, |ui| {
                ui.allocate_ui(size, |ui| text_field(ui, &mut glossary.name));
            });
            // 說明裡的 [名字](網址) 是要照字面顯示的範例，不能走會解析連結的 setting_row
            let creators_hint = |ui: &mut Ui| {
                hint(ui, &t("glossary.creators_hint", &[]), theme::TEXT_SECONDARY);
            };
            setting_row_with(ui, &t("glossary.creators", &[]), creators_hint, |ui| {
                ui.allocate_ui(size, |ui| text_field(ui, &mut glossary.creators));
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
        if self
            .checked
            .as_ref()
            .is_none_or(|(_, b)| *b != glossary.builtin)
        {
            self.builtin_rows.clear();
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
        let creators = glossary
            .builtin
            .as_deref()
            .map_or_else(String::new, builtin_creators);
        let notice = |ui: &mut Ui| {
            if missing {
                hint(ui, &t("glossary.builtin_missing", &[]), theme::WARN_TEXT);
            } else if !creators.is_empty() {
                let by = t("glossary.by_creators", &[("creators", &creators)]);
                rich(ui, &by, theme::HINT_SIZE, theme::TEXT_SECONDARY);
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

    fn custom_card(&mut self, ui: &mut Ui, entries: &mut Vec<Entry>) {
        let query = fold(&self.query);
        let (rows, issues, deletions) = (&mut self.custom_rows, &self.issues, self.deletions);
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
                                fixed_cell(ui, column, |ui| ui.add(Label::new(text))).id
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
                        // 以條目位置當 Id，篩選與提示增減時游標與復原紀錄才不會跑到別列
                        rows.row(ui, i, theme::CONTROL_HEIGHT, |ui| {
                            ui.push_id(i, |ui| {
                                ui.horizontal(|ui| {
                                    // 刪列後下面各列往上遞補：輸入框換新 Id 免得接手被刪那列的復原紀錄，
                                    // 刪除鈕不換，鍵盤刪列後焦點才會留在遞補上來那列的刪除鈕
                                    ui.push_id(deletions, |ui| {
                                        ui.allocate_ui(cell, |ui| {
                                            text_field(ui, &mut entry.source).labelled_by(source);
                                        });
                                        ui.allocate_ui(cell, |ui| {
                                            text_field(ui, &mut entry.target).labelled_by(target);
                                        });
                                    });
                                    if icon_button(ui, icon::CLOSE, &delete).clicked() {
                                        removed = Some(i);
                                    }
                                });
                                if entry.is_blank() {
                                    return;
                                }
                                if let Some((text, color)) = row_hint(&issues[i]) {
                                    hint(ui, &text, color);
                                }
                            });
                        });
                    }
                    added = dashed_button(ui, &t("button.add_entry", &[]), 36.0).clicked();
                });
        });
        if let Some(i) = removed {
            entries.remove(i);
            self.custom_rows.remove(i);
            self.deletions += 1;
        }
        if added {
            entries.push(Entry {
                source: String::new(),
                target: String::new(),
            });
        }
    }

    /// 最下方預設收起的唯讀內建條目；沒有條目（含代碼不存在）時不畫。
    fn builtin_card(&mut self, ui: &mut Ui, code: &str) {
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
                        ui.spacing_mut().item_spacing.y = 6.0;
                        let gap = 16.0;
                        let column = (ui.available_width() - gap) / 2.0;
                        let estimate = ui.text_style_height(&TextStyle::Body);
                        let rows = self.builtin.iter().zip(&self.overridden).enumerate();
                        for (i, (entry, overridden)) in rows {
                            if !matches(entry, &query) {
                                continue;
                            }
                            self.builtin_rows.row(ui, i, estimate, |ui| {
                                ui.horizontal_top(|ui| {
                                    ui.spacing_mut().item_spacing.x = gap;
                                    fixed_cell(ui, column, |ui| {
                                        ui.add(Label::new(&entry.source).wrap());
                                    });
                                    fixed_cell(ui, column, |ui| {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label(&entry.target);
                                            if *overridden {
                                                ui.label(
                                                    RichText::new(&tag)
                                                        .size(theme::HINT_SIZE)
                                                        .color(theme::TEXT_MUTED),
                                                );
                                            }
                                        });
                                    });
                                });
                            });
                        }
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
