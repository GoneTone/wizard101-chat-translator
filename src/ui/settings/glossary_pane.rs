//! 「術語表」分頁：啟用的術語表與術語表清單，外加新增與刪除確認兩種對話框；編輯畫面在分頁內切換。

use eframe::egui::{Frame, Id, Label, Margin, RichText, Ui};

use super::dialog::{Choice, DialogSpec, confirm_delete, dialog};
use super::glossary_draft::{GlossaryDraft, builtin_of};
use super::theme::{self, icon};
use super::widgets::{
    card, combo_box, dashed_button, focus_ring, list_card, name_by_row, secondary_button,
    secondary_icon_button, setting_row,
};
use crate::config::glossaries::StoredGlossary;
use crate::i18n::{language_name, t};

enum Screen {
    List,
    Edit { id: String },
}

enum Dialog {
    Closed,
    /// 新增術語表：選起點。
    New,
    Confirm {
        id: String,
    },
}

/// 新增術語表的起點。
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
}

impl GlossaryPane {
    pub fn new(draft: GlossaryDraft) -> Self {
        Self {
            draft,
            screen: Screen::List,
            dialog: Dialog::Closed,
        }
    }

    pub fn draft(&self) -> &GlossaryDraft {
        &self.draft
    }

    /// 畫目前的畫面（清單或編輯）與開著的對話框。
    pub fn ui(&mut self, ui: &mut Ui) {
        match &self.screen {
            Screen::List => self.list_ui(ui),
            Screen::Edit { id } => {
                let id = id.clone();
                self.edit_ui(ui, &id);
            }
        }
        self.show_dialog(ui);
    }

    fn list_ui(&mut self, ui: &mut Ui) {
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = theme::CARD_GAP;
            card(ui, |ui| {
                setting_row(ui, &t("glossary.active", &[]), None, |ui| {
                    self.active_combo(ui);
                });
            });
            let mut action = None;
            for glossary in &self.draft.glossaries {
                action = action.or(self.glossary_card(ui, glossary));
            }
            if dashed_button(ui, &t("button.add_glossary", &[]), 40.0).clicked() {
                self.dialog = Dialog::New;
            }
            match action {
                Some(CardAction::Edit(id)) => self.screen = Screen::Edit { id },
                Some(CardAction::Duplicate(id)) => {
                    self.draft.duplicate(&id);
                }
                Some(CardAction::Delete(id)) => self.dialog = Dialog::Confirm { id },
                None => {}
            }
        });
    }

    fn edit_ui(&mut self, ui: &mut Ui, id: &str) {
        let back = t("glossary.back_to_list", &[]);
        if secondary_icon_button(ui, icon::BACK, &back).clicked() {
            self.screen = Screen::List;
        }
        if let Some(glossary) = self.draft.find(id) {
            ui.add(Label::new(
                RichText::new(&glossary.name).font(theme::bold(theme::TEXT_SIZE)),
            ));
        }
    }

    fn active_combo(&mut self, ui: &mut Ui) {
        let current = self.draft.active.clone();
        let none = t("glossary.none", &[]);
        let selected = current
            .as_deref()
            .and_then(|id| self.draft.find(id))
            .map_or(none.clone(), |g| g.name.clone());
        let mut chosen = None;
        let combo = combo_box(ui, "active_glossary")
            .selected_text(selected)
            .truncate()
            .show_ui(ui, |ui| {
                let muted = RichText::new(&none).color(theme::TEXT_SECONDARY);
                if ui.selectable_label(current.is_none(), muted).clicked() {
                    chosen = Some(None);
                }
                for glossary in &self.draft.glossaries {
                    let on = current.as_deref() == Some(glossary.id.as_str());
                    if ui.selectable_label(on, &glossary.name).clicked() {
                        chosen = Some(Some(glossary.id.clone()));
                    }
                }
            });
        let response = name_by_row(ui, combo.response);
        focus_ring(ui, &response, theme::CONTROL_RADIUS);
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

    fn show_dialog(&mut self, ui: &Ui) {
        match &self.dialog {
            Dialog::Closed => {}
            Dialog::New => match new_dialog(ui) {
                Choice::Open | Choice::Done(Start::Import) => {}
                Choice::Cancel => self.dialog = Dialog::Closed,
                Choice::Done(Start::Blank) => {
                    let id = self.draft.add_blank();
                    self.screen = Screen::Edit { id };
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

/// 卡片的說明行：自訂條數，加上引入的內建術語表與其條數。
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
    format!("{custom} · {builtin}")
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
                    ui.horizontal(|ui| {
                        let blank = secondary_button(ui, &t("glossary.start_blank", &[]));
                        let import = secondary_button(ui, &t("glossary.start_import", &[]));
                        if blank.clicked() {
                            Some(Start::Blank)
                        } else {
                            import.clicked().then_some(Start::Import)
                        }
                    })
                    .inner
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
