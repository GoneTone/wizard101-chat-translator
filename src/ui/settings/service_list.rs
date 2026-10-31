//! 「翻譯服務」分頁：預設服務、各用途指定與服務清單，外加服務表單、服務商選擇與刪除確認三種對話框。

use eframe::egui::{
    Align, AsIdSalt, Atoms, Frame, Id, Image, Label, Layout, Margin, Response, RichText, Ui, Vec2,
};
use serde_json::Value;

use super::dialog::{Choice, DialogSpec, confirm_delete, dialog};
use super::env::{SettingsEnv, SettingsEvent};
use super::provider_picker::{CardLayout, provider_cards, provider_icon, provider_texture};
use super::service_draft::{ServiceDraft, id_of, local_model_gone};
use super::service_form::{FormResponse, ServiceForm};
use super::theme;
use super::widgets::{
    callout, card, collapsible, combo_box, danger_secondary_button, dashed_button, focus_ring,
    full_text_tip, list_card, name_by_row, primary_button, secondary_button, setting_row,
};
use crate::config::py::py_bool;
use crate::config::services::{LOCAL_PROVIDER, ServiceSlot, describe, new_service, str_field};
use crate::i18n::t;
use crate::local::Snapshot;
use crate::logs::log;

enum Dialog {
    Closed,
    /// 新增服務的第一步：選服務商。
    Pick,
    Form {
        form: Box<ServiceForm>,
        /// 表單上疊著服務商選擇（按了〔更改〕）。
        picking: bool,
        /// 按〔確定〕時沒過的欄位；顯示後隨編輯即時更新。
        errors: Vec<String>,
    },
    Confirm {
        id: String,
    },
}

impl Dialog {
    /// 剛開啟的服務表單。
    fn form(form: ServiceForm) -> Self {
        Self::Form {
            form: Box::new(form),
            picking: false,
            errors: Vec::new(),
        }
    }
}

/// 翻譯服務分頁；`draft()` 是目前編輯中的值，設定視窗按儲存時寫回。
pub struct ServicePane {
    draft: ServiceDraft,
    dialog: Dialog,
    /// 表單上按了連到本地模型頁的連結；外層讀走（`take_open_local`）後切頁。
    open_local: bool,
}

impl ServicePane {
    pub fn new(draft: ServiceDraft) -> Self {
        Self {
            draft,
            dialog: Dialog::Closed,
            open_local: false,
        }
    }

    /// 這一幀表單有沒有要求切到本地模型頁；讀走就清掉。
    pub fn take_open_local(&mut self) -> bool {
        std::mem::take(&mut self.open_local)
    }

    pub fn draft(&self) -> &ServiceDraft {
        &self.draft
    }

    /// 測試連線與模型清單的結果轉給開著的服務表單。
    pub fn apply(&mut self, event: &SettingsEvent) {
        if let Dialog::Form { form, .. } = &mut self.dialog {
            form.apply(event);
        }
    }

    /// 畫分頁與開著的對話框；測試連線翻成 `target_language`。
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        env: &dyn SettingsEnv,
        snapshot: &Snapshot,
        target_language: &str,
    ) {
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = theme::CARD_GAP;
            card(ui, |ui| {
                setting_row(ui, &t("service.default", &[]), None, |ui| {
                    self.default_combo(ui);
                });
            });
            self.slots_card(ui);
            ui.add_space(6.0);
            ui.add(Label::new(
                RichText::new(t("service.my_services", &[])).font(theme::bold(theme::TEXT_SIZE)),
            ));
            let can_delete = self.draft.can_delete();
            let mut action = None;
            for service in &self.draft.services {
                action = action.or(self.service_card(ui, service, can_delete, snapshot));
            }
            if !self.draft.services.is_empty() && !can_delete {
                ui.add(Label::new(
                    RichText::new(t("service.keep_one", &[]))
                        .size(theme::HINT_SIZE)
                        .color(theme::TEXT_SECONDARY),
                ));
            }
            if dashed_button(ui, &t("button.add_service", &[]), 40.0).clicked() {
                self.dialog = Dialog::Pick;
            }
            match action {
                Some(CardAction::Edit(id)) => self.open_form(&id),
                Some(CardAction::Delete(id)) => self.dialog = Dialog::Confirm { id },
                None => {}
            }
        });
        self.show_dialog(ui, env, snapshot, target_language);
    }

    fn open_form(&mut self, id: &str) {
        if let Some(service) = self.draft.find(Some(id)) {
            self.dialog = Dialog::form(ServiceForm::new(
                service.clone(),
                self.draft.services.clone(),
            ));
        }
    }

    fn default_combo(&mut self, ui: &mut Ui) {
        let current = self.draft.default.clone();
        let (response, chosen) =
            service_combo(ui, "default_service", &self.draft, current.as_deref(), None);
        let response = name_by_row(ui, response);
        focus_ring(ui, &response, theme::CONTROL_RADIUS);
        if let Some(Some(id)) = chosen {
            self.draft.set_default(&id);
        }
    }

    fn slots_card(&mut self, ui: &mut Ui) {
        let assigned = self.draft.slots.iter().any(Option::is_some);
        card(ui, |ui| {
            let id = ui.scope_id().with("service_slots");
            let title = t("service.slots_title", &[]);
            let hint = t("service.slots_hint", &[]);
            collapsible(ui, id, &title, Some(&hint), assigned, |ui| {
                for (index, slot) in ServiceSlot::ALL.into_iter().enumerate() {
                    self.slot_row(ui, index, slot);
                }
                ui.add_space(6.0);
            });
        });
    }

    fn slot_row(&mut self, ui: &mut Ui, index: usize, slot: ServiceSlot) {
        let current = self.draft.slots[index].clone();
        let follow = t("service.follow_default", &[]);
        let mut chosen = None;
        let margin = Margin {
            left: 44,
            right: 16,
            top: 6,
            bottom: 6,
        };
        Frame::new().inner_margin(margin).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(theme::CONTROL_HEIGHT);
                let label = ui.add(Label::new(t(&format!("slot.{}", slot.key()), &[])));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (response, picked) = service_combo(
                        ui,
                        ("service_slot", slot.key()),
                        &self.draft,
                        current.as_deref(),
                        Some(&follow),
                    );
                    chosen = picked;
                    let response = response.labelled_by(label.id);
                    focus_ring(ui, &response, theme::CONTROL_RADIUS);
                });
            });
        });
        if let Some(id) = chosen {
            self.draft.set_slot(slot, id);
        }
    }

    fn service_card(
        &self,
        ui: &mut Ui,
        service: &Value,
        can_delete: bool,
        snapshot: &Snapshot,
    ) -> Option<CardAction> {
        let id = id_of(service);
        let mut badges = self.draft.badges(service);
        if local_model_gone(service, snapshot) {
            badges.push(t("local.form.model_missing", &[]));
        }
        let (edit, delete) = list_card(
            ui,
            Id::unique(("service_card_buttons", id)),
            |ui| provider_icon(ui, str_field(service, "provider"), 32.0),
            str_field(service, "name"),
            &badges,
            &describe(service, &model_label(service, snapshot)),
            |ui| {
                let edit = secondary_button(ui, &t("button.edit", &[])).clicked();
                let delete = ui
                    .add_enabled_ui(can_delete, |ui| {
                        danger_secondary_button(ui, &t("button.delete", &[]))
                    })
                    .inner
                    .clicked();
                (edit, delete)
            },
        );
        if edit {
            Some(CardAction::Edit(id.to_owned()))
        } else if delete {
            Some(CardAction::Delete(id.to_owned()))
        } else {
            None
        }
    }

    fn show_dialog(
        &mut self,
        ui: &Ui,
        env: &dyn SettingsEnv,
        snapshot: &Snapshot,
        target_language: &str,
    ) {
        match &mut self.dialog {
            Dialog::Closed => {}
            Dialog::Pick => match picker_dialog(ui) {
                Choice::Open => {}
                Choice::Cancel => self.dialog = Dialog::Closed,
                Choice::Done(key) => {
                    let service = new_service(key, &self.draft.services);
                    self.dialog =
                        Dialog::form(ServiceForm::new(service, self.draft.services.clone()));
                }
            },
            Dialog::Form {
                form,
                picking,
                errors,
            } => {
                let (choice, response) =
                    form_dialog(ui, form, errors, env, snapshot, target_language);
                *picking |= response.change_provider;
                self.open_local |= response.open_local_models;
                pick_provider_over(ui, form, picking);
                match choice {
                    Choice::Open => {}
                    Choice::Cancel => self.dialog = Dialog::Closed,
                    Choice::Done(service) => {
                        self.draft.apply_dialog_result(service);
                        self.dialog = Dialog::Closed;
                    }
                }
            }
            Dialog::Confirm { id } => {
                let id = id.clone();
                match self.confirm_dialog(ui, &id) {
                    Choice::Open => {}
                    Choice::Cancel => self.dialog = Dialog::Closed,
                    Choice::Done(()) => {
                        self.draft.delete(&id);
                        self.dialog = Dialog::Closed;
                    }
                }
            }
        }
    }

    fn confirm_dialog(&self, ui: &Ui, id: &str) -> Choice<()> {
        let name = self
            .draft
            .find(Some(id))
            .map_or("", |s| str_field(s, "name"));
        let in_use = self.draft.slots.iter().any(|s| s.as_deref() == Some(id));
        let question = if in_use {
            "service.confirm_delete_in_use"
        } else {
            "service.confirm_delete"
        };
        confirm_delete(ui, "confirm_delete", t(question, &[("name", name)]))
    }
}

enum CardAction {
    Edit(String),
    Delete(String),
}

/// 服務下拉：列出 `draft` 的服務；`follow` 有值就多一個該文字的「跟隨預設」選項。
///
/// 選了服務回 `Some(Some(id))`，選了跟隨預設回 `Some(None)`；命名與焦點框留給呼叫端。
fn service_combo(
    ui: &mut Ui,
    salt: impl AsIdSalt,
    draft: &ServiceDraft,
    current: Option<&str>,
    follow: Option<&str>,
) -> (Response, Option<Option<String>>) {
    let selected = service_atoms(ui, draft.find(current), follow.unwrap_or(""));
    let mut chosen = None;
    let combo = combo_box(ui, salt)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            if let Some(follow) = follow {
                let muted = RichText::new(follow).color(theme::TEXT_SECONDARY);
                if ui.selectable_label(current.is_none(), muted).clicked() {
                    chosen = Some(None);
                }
            }
            for service in &draft.services {
                let id = id_of(service);
                let atoms = service_atoms(ui, Some(service), "");
                if ui.selectable_label(current == Some(id), atoms).clicked() {
                    chosen = Some(Some(id.to_owned()));
                }
            }
        });
    let name = draft.find(current).map(|s| str_field(s, "name"));
    // 有服務時名稱前多了服務商圖示與間距
    let leading = name.map_or(0.0, |_| 18.0 + ui.spacing().icon_spacing);
    full_text_tip(ui, &combo.response, name.or(follow).unwrap_or(""), leading);
    (combo.response, chosen)
}

/// 下拉裡的一個服務：服務商圖示加名稱；沒有服務時顯示 `fallback`（說明色）。
fn service_atoms(ui: &Ui, service: Option<&Value>, fallback: &str) -> Atoms<'static> {
    let mut atoms = Atoms::default();
    match service {
        Some(service) => {
            if let Some(texture) = provider_texture(ui.ctx(), str_field(service, "provider")) {
                let size = Vec2::splat(18.0);
                atoms.push_right(Image::from_texture((texture, size)).fit_to_exact_size(size));
            }
            atoms.push_right(str_field(service, "name").to_owned());
        }
        None => atoms.push_right(RichText::new(fallback).color(theme::TEXT_SECONDARY)),
    }
    atoms
}

/// 表單上按了〔更改〕時疊上服務商選擇：選定就換服務商；選定或取消後收起（`picking` 回到 `false`）。
pub(super) fn pick_provider_over(ui: &Ui, form: &mut ServiceForm, picking: &mut bool) {
    if !*picking {
        return;
    }
    match picker_dialog(ui) {
        Choice::Open => {}
        Choice::Cancel => *picking = false,
        Choice::Done(key) => {
            form.set_provider(key);
            *picking = false;
        }
    }
}

fn picker_dialog(ui: &Ui) -> Choice<&'static str> {
    let spec = DialogSpec {
        salt: "provider_picker",
        width: theme::PICKER_DIALOG_WIDTH,
        title: t("service.pick_provider", &[]),
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
                .show(ui, |ui| provider_cards(ui, CardLayout::List))
                .inner
        },
        |ui| secondary_button(ui, &t("button.cancel", &[])).clicked(),
    );
    match shown.body {
        Some(key) => Choice::Done(key),
        None if shown.footer || shown.cancel => Choice::Cancel,
        None => Choice::Open,
    }
}

/// 服務表單對話框，另回傳這一幀表單的回報（〔更改〕服務商、連到本地模型頁）；〔確定〕時欄位不完整就在表單底部列出，不關閉。
fn form_dialog(
    ui: &Ui,
    form: &mut ServiceForm,
    errors: &mut Vec<String>,
    env: &dyn SettingsEnv,
    snapshot: &Snapshot,
    target_language: &str,
) -> (Choice<Value>, FormResponse) {
    if !errors.is_empty() {
        *errors = form.errors();
    }
    // 內容高度會隨測試結果、提示與錯誤變動，釘住上緣，按鈕才不會在游標下跳動
    let spec = DialogSpec {
        salt: "service_form",
        width: theme::SERVICE_DIALOG_WIDTH,
        title: t("service.dialog_title", &[]),
        closable: true,
        pinned_top: true,
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
                    bottom: 16,
                })
                .show(ui, |ui| {
                    let response = form.ui(ui, env, snapshot, target_language);
                    if !errors.is_empty() {
                        ui.add_space(12.0);
                        let lines: Vec<String> = errors.iter().map(|e| t(e, &[])).collect();
                        callout(ui, &t("dialog.incomplete_title", &[]), &lines);
                    }
                    response
                })
                .inner
        },
        |ui| {
            let cancel = secondary_button(ui, &t("button.cancel", &[])).clicked();
            let ok = primary_button(ui, &t("button.ok", &[])).clicked();
            (ok, cancel)
        },
    );
    let (ok, cancel) = shown.footer;
    let choice = if cancel || shown.cancel {
        Choice::Cancel
    } else if ok {
        save(form, errors)
    } else {
        Choice::Open
    };
    (choice, shown.body)
}

/// 〔確定〕：欄位都過了就回傳要存的服務，否則記下沒過的原因。
fn save(form: &ServiceForm, errors: &mut Vec<String>) -> Choice<Value> {
    let found = form.errors();
    if !found.is_empty() {
        log(format!(
            "[settings] service not saved: {}",
            found.join(", ")
        ));
        *errors = found;
        return Choice::Open;
    }
    let service = form.values();
    log(format!(
        "[settings] service saved (provider={}, has_key={})",
        str_field(&service, "provider"),
        py_bool(!str_field(&service, "api_key").is_empty())
    ));
    Choice::Done(service)
}

/// 本地服務顯示「org/repo · 量化」而不是內部 id；其他服務照存的模型名稱。
fn model_label(service: &Value, snapshot: &Snapshot) -> String {
    let model = str_field(service, "model");
    match snapshot.ready_model(model) {
        Some(m) if str_field(service, "provider") == LOCAL_PROVIDER => m.manifest.display_name(),
        _ => model.to_owned(),
    }
}
