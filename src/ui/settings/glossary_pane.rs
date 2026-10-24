//! 「術語表」分頁：啟用的術語表與術語表清單，外加新增、刪除確認與匯入合併三種對話框；編輯畫面在分頁內切換。

use std::path::Path;

use eframe::egui::{Frame, Id, Label, Margin, RichText, Ui};

use super::dialog::{Choice, DialogSpec, confirm_delete, dialog};
use super::env::{Round, SettingsEnv, SettingsEvent};
use super::glossary_draft::{GlossaryDraft, builtin_of};
use super::glossary_editor::{EditorAction, GlossaryEditor};
use super::glossary_merge::MergeDialog;
use super::provider_picker::{CardIcon, choice_cards};
use super::theme::{self, icon};
use super::widgets::{
    callout, card, dashed_button, list_card, optional_combo, secondary_button, setting_row,
};
use crate::config::glossaries::StoredGlossary;
use crate::glossary::{Entry, GlossaryFile, ParseError, file_to_json, parse_file, plan_merge};
use crate::i18n::{language_name, t};
use crate::logs::log;
use crate::py::pystr;
use crate::ui::richtext::strip_link_markup;
use crate::util::write_atomic;

enum Screen {
    List,
    Edit(Box<GlossaryEditor>),
}

enum Dialog {
    Closed,
    /// 新增術語表：選起點。
    New,
    Confirm {
        id: String,
    },
    /// 把匯入的檔案併入 `id` 那份。
    Merge {
        id: String,
        merge: MergeDialog,
    },
}

/// 等選檔結果的操作。
enum FileTask {
    /// 從檔案新增一份。
    Create,
    /// 併入 `id` 那份。
    Merge(String),
    /// 匯出 `id` 那份。
    Export(String),
}

/// 新增術語表的起點。
#[derive(Clone, Copy)]
enum Start {
    Blank,
    Import,
}

enum CardAction {
    Edit(String),
    Duplicate(String),
    Delete(String),
}

/// 術語表分頁；`draft()` 是目前編輯中的值，設定視窗按儲存時寫回。
pub struct GlossaryPane {
    draft: GlossaryDraft,
    screen: Screen,
    dialog: Dialog,
    file_round: Round,
    file_task: Option<FileTask>,
    /// 匯入／匯出失敗的提示（標題 key 與錯誤原文），畫在目前畫面上方，換畫面時清掉。
    failure: Option<(&'static str, String)>,
}

impl GlossaryPane {
    pub fn new(draft: GlossaryDraft) -> Self {
        Self {
            draft,
            screen: Screen::List,
            dialog: Dialog::Closed,
            file_round: Round::new("glossary file dialog"),
            file_task: None,
            failure: None,
        }
    }

    pub fn draft(&self) -> &GlossaryDraft {
        &self.draft
    }

    /// 選檔結果：依發起的操作匯入、合併或匯出；過期或取消的結果丟掉。
    pub fn apply(&mut self, event: &SettingsEvent) {
        let (SettingsEvent::GlossaryFilePicked { round, path }
        | SettingsEvent::GlossarySavePathPicked { round, path }) = event
        else {
            return;
        };
        if !self.file_round.finish(*round) {
            return;
        }
        let (Some(task), Some(path)) = (self.file_task.take(), path) else {
            log("[settings] glossary file dialog cancelled");
            return;
        };
        match task {
            FileTask::Create => self.import_new(path),
            FileTask::Merge(id) => self.import_merge(id, path),
            FileTask::Export(id) => self.export(&id, path),
        }
    }

    /// 畫目前的畫面（清單或編輯）與開著的對話框。
    pub fn ui(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) {
        if let Some((title, error)) = &self.failure {
            callout(ui, &t(title, &[]), std::slice::from_ref(error));
            ui.add_space(theme::CARD_GAP);
        }
        if let Screen::Edit(editor) = &mut self.screen {
            let id = editor.id().to_owned();
            let glossary = self.draft.glossaries.iter_mut().find(|g| g.id == id);
            let action = match glossary {
                Some(glossary) => editor.ui(ui, glossary),
                None => EditorAction::Back,
            };
            match action {
                EditorAction::None => {}
                EditorAction::Back => self.set_screen(Screen::List),
                EditorAction::Import => {
                    let round = self.start(FileTask::Merge(id));
                    env.pick_glossary_file(round);
                }
                EditorAction::Export => {
                    let name = self.draft.find(&id).map_or("", |g| g.name.as_str());
                    let file_name = export_file_name(name);
                    let round = self.start(FileTask::Export(id));
                    env.pick_glossary_save_path(round, file_name);
                }
            }
        } else {
            self.list_ui(ui);
        }
        self.show_dialog(ui, env);
    }

    fn set_screen(&mut self, screen: Screen) {
        self.screen = screen;
        self.failure = None;
    }

    /// 開始等一次選檔結果；回傳本輪編號。
    fn start(&mut self, task: FileTask) -> u64 {
        self.file_task = Some(task);
        self.failure = None;
        self.file_round.start()
    }

    fn import_new(&mut self, path: &Path) {
        let Some(file) = self.read(path) else {
            return;
        };
        let name = path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let id = self.draft.add_from_file(&name, file);
        self.set_screen(Screen::Edit(Box::new(GlossaryEditor::new(&id))));
    }

    fn import_merge(&mut self, id: String, path: &Path) {
        let Some(incoming) = self.read(path) else {
            return;
        };
        let Some(glossary) = self.draft.find(&id) else {
            return;
        };
        // 合併只加條目，建立者維持目前這份的
        let merge = MergeDialog::new(plan_merge(&glossary.entries, &incoming.entries));
        self.dialog = Dialog::Merge { id, merge };
    }

    /// 讀檔並解析；失敗時記下提示（log 寫英文細節，畫面顯示說明）。
    fn read(&mut self, path: &Path) -> Option<GlossaryFile> {
        let parsed = std::fs::read_to_string(path)
            .map_err(|e| (e.to_string(), read_error(&e)))
            .and_then(|text| {
                parse_file(text.strip_prefix('\u{feff}').unwrap_or(&text))
                    .map_err(|e| (e.to_string(), parse_error(&e)))
            })
            .and_then(|file| {
                if file.entries.is_empty() {
                    Err(("no entries".into(), t("glossary.error_empty", &[])))
                } else {
                    Ok(file)
                }
            });
        match parsed {
            Ok(file) => {
                log(format!(
                    "[settings] glossary imported: {} entries from {}",
                    file.entries.len(),
                    path.display()
                ));
                Some(file)
            }
            Err((detail, shown)) => {
                log(format!(
                    "[settings] glossary import failed: {}: {detail}",
                    path.display()
                ));
                self.failure = Some(("glossary.import_failed", shown));
                None
            }
        }
    }

    /// 寫出建立者與自訂條目（略過兩側都空白的列）。
    fn export(&mut self, id: &str, path: &Path) {
        let Some(glossary) = self.draft.find(id) else {
            return;
        };
        let entries: Vec<Entry> = glossary
            .entries
            .iter()
            .filter(|e| !e.is_blank())
            .cloned()
            .collect();
        let file = GlossaryFile {
            creators: pystr::strip(&glossary.creators).into(),
            entries,
        };
        match write_atomic(path, &file_to_json(&file)) {
            Ok(()) => log(format!(
                "[settings] glossary exported: {} entries to {}",
                file.entries.len(),
                path.display()
            )),
            Err(e) => {
                log(format!(
                    "[settings] glossary export failed: {}: {e}",
                    path.display()
                ));
                self.failure = Some(("glossary.export_failed", e.to_string()));
            }
        }
    }

    fn list_ui(&mut self, ui: &mut Ui) {
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = theme::CARD_GAP;
            card(ui, |ui| {
                setting_row(ui, &t("glossary.active", &[]), None, |ui| {
                    self.active_combo(ui);
                });
            });
            ui.add_space(6.0);
            ui.add(Label::new(
                RichText::new(t("glossary.my_glossaries", &[])).font(theme::bold(theme::TEXT_SIZE)),
            ));
            let mut action = None;
            for glossary in &self.draft.glossaries {
                action = action.or(self.glossary_card(ui, glossary));
            }
            if dashed_button(ui, &t("button.add_glossary", &[]), 40.0).clicked() {
                self.dialog = Dialog::New;
            }
            match action {
                Some(CardAction::Edit(id)) => {
                    self.set_screen(Screen::Edit(Box::new(GlossaryEditor::new(&id))))
                }
                Some(CardAction::Duplicate(id)) => {
                    self.draft.duplicate(&id);
                }
                Some(CardAction::Delete(id)) => self.dialog = Dialog::Confirm { id },
                None => {}
            }
        });
    }

    fn active_combo(&mut self, ui: &mut Ui) {
        let current = self.draft.active.clone();
        let none = t("glossary.none", &[]);
        let selected = current
            .as_deref()
            .and_then(|id| self.draft.find(id))
            .map_or(none.clone(), |g| g.name.clone());
        let options: Vec<(String, String)> = self
            .draft
            .glossaries
            .iter()
            .map(|g| (g.id.clone(), g.name.clone()))
            .collect();
        let chosen = optional_combo(
            ui,
            "active_glossary",
            selected,
            &none,
            &options,
            current.as_deref(),
        );
        if let Some(id) = chosen {
            self.draft.set_active(id);
        }
    }

    fn glossary_card(&self, ui: &mut Ui, glossary: &StoredGlossary) -> Option<CardAction> {
        let id = &glossary.id;
        let badges: Vec<String> = (self.draft.active.as_ref() == Some(id))
            .then(|| t("glossary.badge_active", &[]))
            .into_iter()
            .collect();
        let (edit, copy, delete) = list_card(
            ui,
            Id::unique(("glossary_card_buttons", id)),
            |_| {},
            &glossary.name,
            &badges,
            &summary(glossary),
            |ui| {
                (
                    secondary_button(ui, &t("button.edit", &[])).clicked(),
                    secondary_button(ui, &t("button.copy", &[])).clicked(),
                    secondary_button(ui, &t("button.delete", &[])).clicked(),
                )
            },
        );
        if edit {
            Some(CardAction::Edit(id.clone()))
        } else if copy {
            Some(CardAction::Duplicate(id.clone()))
        } else if delete {
            Some(CardAction::Delete(id.clone()))
        } else {
            None
        }
    }

    fn show_dialog(&mut self, ui: &Ui, env: &dyn SettingsEnv) {
        match &mut self.dialog {
            Dialog::Closed => {}
            Dialog::New => match new_dialog(ui) {
                Choice::Open => {}
                Choice::Cancel => self.dialog = Dialog::Closed,
                Choice::Done(Start::Blank) => {
                    let id = self.draft.add_blank();
                    self.set_screen(Screen::Edit(Box::new(GlossaryEditor::new(&id))));
                    self.dialog = Dialog::Closed;
                }
                Choice::Done(Start::Import) => {
                    self.dialog = Dialog::Closed;
                    let round = self.start(FileTask::Create);
                    env.pick_glossary_file(round);
                }
            },
            Dialog::Merge { id, merge } => match merge.ui(ui) {
                Choice::Open => {}
                Choice::Cancel => {
                    log("[settings] glossary merge cancelled");
                    self.dialog = Dialog::Closed;
                }
                Choice::Done(()) => {
                    let id = id.clone();
                    if let Some(glossary) = self.draft.glossaries.iter_mut().find(|g| g.id == id) {
                        merge.apply(&mut glossary.entries);
                    }
                    self.dialog = Dialog::Closed;
                }
            },
            Dialog::Confirm { id } => {
                let id = id.clone();
                let name = self.draft.find(&id).map_or("", |g| g.name.as_str());
                let question = if self.draft.active.as_deref() == Some(id.as_str()) {
                    "glossary.confirm_delete_active"
                } else {
                    "glossary.confirm_delete"
                };
                match confirm_delete(ui, t(question, &[("name", name)])) {
                    Choice::Open => {}
                    Choice::Cancel => self.dialog = Dialog::Closed,
                    Choice::Done(()) => {
                        self.draft.remove(&id);
                        self.dialog = Dialog::Closed;
                    }
                }
            }
        }
    }
}

/// 匯入時讀不到檔案的說明；不是 UTF-8 時系統訊息看不懂，其餘照系統原文。
fn read_error(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::InvalidData => t("glossary.error_not_utf8", &[]),
        _ => e.to_string(),
    }
}

/// 匯入檔格式不符的說明。
fn parse_error(e: &ParseError) -> String {
    match e {
        ParseError::Json { line } => t("glossary.error_json", &[("line", &line.to_string())]),
        ParseError::NotObject => {
            // 大括號會被當成文案佔位符，範例由程式組好帶入
            let target = t("glossary.example_target", &[]);
            let example = format!(r#"{{"Hello World": "{target}"}}"#);
            t("glossary.error_not_object", &[("example", &example)])
        }
        ParseError::NotText { key } => t("glossary.error_not_text", &[("key", key)]),
    }
}

/// 卡片的說明行：自訂條數，加上引入的內建術語表與其條數；有建立者時接在最後（只留名字）。
fn summary(glossary: &StoredGlossary) -> String {
    let custom = glossary.entries.len().to_string();
    let custom = t("glossary.custom_count", &[("count", &custom)]);
    let builtin = match &glossary.builtin {
        Some(code) => {
            let count = builtin_of(glossary).len().to_string();
            let language = language_name(code);
            t(
                "glossary.builtin_count",
                &[("language", &language), ("count", &count)],
            )
        }
        None => t("glossary.no_builtin", &[]),
    };
    let creators = strip_link_markup(pystr::strip(&glossary.creators));
    if creators.is_empty() {
        format!("{custom} · {builtin}")
    } else {
        let by = t("glossary.by_creators", &[("creators", &creators)]);
        format!("{custom} · {builtin} · {by}")
    }
}

/// 新增術語表的對話框：選空白或從檔案匯入。
fn new_dialog(ui: &Ui) -> Choice<Start> {
    let spec = DialogSpec {
        salt: "new_glossary",
        width: theme::PICKER_DIALOG_WIDTH,
        title: t("glossary.new_title", &[]),
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
                    top: 8,
                    bottom: 18,
                })
                .show(ui, |ui| {
                    let cards = [
                        (
                            CardIcon::Glyph(icon::NEW_DOCUMENT),
                            t("glossary.start_blank", &[]),
                            t("glossary.start_blank_desc", &[]),
                        ),
                        (
                            CardIcon::Glyph(icon::IMPORT),
                            t("glossary.start_import", &[]),
                            t("glossary.start_import_desc", &[]),
                        ),
                    ];
                    choice_cards(ui, &cards).map(|index| [Start::Blank, Start::Import][index])
                })
                .inner
        },
        |ui| secondary_button(ui, &t("button.cancel", &[])).clicked(),
    );
    match shown.body {
        Some(start) => Choice::Done(start),
        None if shown.footer || shown.cancel => Choice::Cancel,
        None => Choice::Open,
    }
}

/// 匯出的預設檔名：Windows 檔名不允許的字元換成 `_`，剩下空白就用預設名稱。
fn export_file_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"\/:*?"<>|"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    match pystr::strip(&safe) {
        "" => format!("{}.json", t("glossary.default_name", &[])),
        safe => format!("{safe}.json"),
    }
}
