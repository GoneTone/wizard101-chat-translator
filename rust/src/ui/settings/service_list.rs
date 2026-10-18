//! 「翻譯服務」分頁：預設服務、各用途指定與服務清單，外加服務表單、服務商選擇與刪除確認三種對話框。

use eframe::egui::accesskit;
use eframe::egui::{
    Align, Atoms, FontId, Frame, Id, Image, Label, Layout, Margin, RichText, Role, Sense, Ui,
    UiBuilder, Vec2, vec2,
};
use serde_json::Value;

use super::dialog::{DialogSpec, dialog};
use super::env::{SettingsEnv, SettingsEvent};
use super::provider_picker::{CardLayout, paint_provider_icon, provider_cards, provider_texture};
use super::service_form::ServiceForm;
use super::theme;
use super::widgets::{
    callout, card, collapsible, combo_box, danger_button, dashed_button, focus_ring, name_by_row,
    primary_button, right_aligned, right_aligned_width, secondary_button, setting_row,
};
use crate::config::Config;
use crate::config::py::py_bool;
use crate::config::services::{
    ServiceSlot, describe, new_service, str_field, unique_name, validate_service,
};
use crate::i18n::t;
use crate::logs::log;

fn id_of(service: &Value) -> &str {
    str_field(service, "id")
}

/// 服務分頁編輯中的三個設定鍵：服務清單、預設服務與各用途指定；`write_into` 寫回設定。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServiceDraft {
    pub services: Vec<Value>,
    pub default: Option<String>,
    /// 各用途指定的服務 id，順序同 `ServiceSlot::ALL`；`None` 是跟隨預設。
    pub slots: [Option<String>; 3],
}

impl ServiceDraft {
    /// 從設定取出三個鍵。
    pub fn from_config(cfg: &Config) -> Self {
        let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
        let slots = cfg.get("service_slots");
        Self {
            services: cfg
                .get("services")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            default: text(cfg.get("default_service")),
            slots: ServiceSlot::ALL.map(|slot| text(slots.and_then(|s| s.get(slot.key())))),
        }
    }

    /// 寫回設定；`service_slots` 裡認不得的鍵原樣保留。
    pub fn write_into(&self, cfg: &mut Config) {
        cfg.insert("services".into(), Value::Array(self.services.clone()));
        cfg.insert("default_service".into(), self.default.clone().into());
        let mut slots = cfg
            .get("service_slots")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (slot, id) in ServiceSlot::ALL.iter().zip(&self.slots) {
            slots.insert(slot.key().into(), id.clone().into());
        }
        cfg.insert("service_slots".into(), Value::Object(slots));
    }

    /// 這筆服務擔任的角色：先「預設」，再依用途順序。
    fn badges(&self, service: &Value) -> Vec<String> {
        let id = id_of(service);
        let default =
            (self.default.as_deref() == Some(id)).then(|| t("service.badge_default", &[]));
        let slots = ServiceSlot::ALL
            .iter()
            .zip(&self.slots)
            .filter(|(_, assigned)| assigned.as_deref() == Some(id))
            .map(|(slot, _)| t(&format!("slot.{}", slot.key()), &[]));
        default.into_iter().chain(slots).collect()
    }

    /// 卡片的標題：服務名稱後接它擔任的角色，以 ` · ` 分隔。
    pub fn card_label(&self, service: &Value) -> String {
        let name = str_field(service, "name");
        let badges = self.badges(service);
        if badges.is_empty() {
            name.to_owned()
        } else {
            format!("{name}\u{3000}{}", badges.join(" · "))
        }
    }

    /// 最後一筆不給刪：刪光就無從翻譯。
    pub fn can_delete(&self) -> bool {
        self.services.len() > 1
    }

    /// 把對話框確定的服務併回清單（同 id 就地取代，否則加在最後）；清單原本是空的就接手預設。
    pub fn apply_dialog_result(&mut self, mut service: Value) {
        let id = id_of(&service).to_owned();
        // 三個用途下拉只顯示名稱，同名會讓人選錯
        service["name"] =
            unique_name(str_field(&service, "name"), &self.services, Some(&id)).into();
        match self.services.iter_mut().find(|s| id_of(s) == id) {
            Some(existing) => *existing = service,
            None => self.services.push(service),
        }
        if self.default.is_none() {
            log(format!("[settings] first service added; default={id}"));
            self.default = Some(id);
        }
    }

    /// 刪除一筆服務：指到它的用途改回跟隨預設，被刪的是預設就交給清單第一筆。
    pub fn delete(&mut self, id: &str) {
        if !self.can_delete() {
            return;
        }
        self.services.retain(|s| id_of(s) != id);
        for (slot, assigned) in ServiceSlot::ALL.iter().zip(&mut self.slots) {
            if assigned.as_deref() == Some(id) {
                *assigned = None;
                log(format!(
                    "[settings] slot {} followed the default after its service was deleted",
                    slot.key()
                ));
            }
        }
        if self.default.as_deref() == Some(id) {
            self.default = self.services.first().map(|s| id_of(s).to_owned());
            log(format!(
                "[settings] default service deleted; default={}",
                self.default.as_deref().unwrap_or("None")
            ));
        }
    }

    pub fn set_default(&mut self, id: &str) {
        self.default = Some(id.to_owned());
        log(format!("[settings] default service set to {id}"));
    }

    /// 指定用途的服務；`None` 是跟隨預設。
    pub fn set_slot(&mut self, slot: ServiceSlot, id: Option<String>) {
        log(format!(
            "[settings] slot {} set to {}",
            slot.key(),
            id.as_deref().unwrap_or("default")
        ));
        if let Some(index) = ServiceSlot::ALL.iter().position(|s| *s == slot) {
            self.slots[index] = id;
        }
    }

    fn find(&self, id: Option<&str>) -> Option<&Value> {
        let id = id?;
        self.services.iter().find(|s| id_of(s) == id)
    }
}

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

/// 對話框這一幀的結果。
enum Choice<T> {
    Open,
    Cancel,
    Done(T),
}

/// 翻譯服務分頁；`draft()` 是目前編輯中的值，設定視窗按儲存時寫回。
pub struct ServicePane {
    draft: ServiceDraft,
    dialog: Dialog,
}

impl ServicePane {
    pub fn new(draft: ServiceDraft) -> Self {
        Self {
            draft,
            dialog: Dialog::Closed,
        }
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
    pub fn ui(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, target_language: &str) {
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
                action = action.or(self.service_card(ui, service, can_delete));
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
        self.show_dialog(ui, env, target_language);
    }

    fn open_form(&mut self, id: &str) {
        if let Some(service) = self.draft.find(Some(id)) {
            let form = ServiceForm::new(service.clone(), self.draft.services.clone());
            self.dialog = Dialog::Form {
                form: Box::new(form),
                picking: false,
                errors: Vec::new(),
            };
        }
    }

    fn default_combo(&mut self, ui: &mut Ui) {
        let current = self.draft.default.clone();
        let selected = service_atoms(ui, self.draft.find(current.as_deref()), "");
        let mut chosen = None;
        let response = combo_box(ui, "default_service")
            .selected_text(selected)
            .truncate()
            .show_ui(ui, |ui| {
                for service in &self.draft.services {
                    let id = id_of(service);
                    let atoms = service_atoms(ui, Some(service), "");
                    if ui
                        .selectable_label(current.as_deref() == Some(id), atoms)
                        .clicked()
                    {
                        chosen = Some(id.to_owned());
                    }
                }
            })
            .response;
        let response = name_by_row(ui, response);
        focus_ring(ui, &response, theme::CONTROL_RADIUS);
        if let Some(id) = chosen {
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
                    let selected = service_atoms(ui, self.draft.find(current.as_deref()), &follow);
                    let combo = combo_box(ui, ("service_slot", slot.key()))
                        .selected_text(selected)
                        .truncate()
                        .show_ui(ui, |ui| {
                            let muted = RichText::new(follow.as_str()).color(theme::TEXT_SECONDARY);
                            if ui.selectable_label(current.is_none(), muted).clicked() {
                                chosen = Some(None);
                            }
                            for service in &self.draft.services {
                                let id = id_of(service);
                                let atoms = service_atoms(ui, Some(service), "");
                                if ui
                                    .selectable_label(current.as_deref() == Some(id), atoms)
                                    .clicked()
                                {
                                    chosen = Some(Some(id.to_owned()));
                                }
                            }
                        });
                    let response = combo.response.labelled_by(label.id);
                    focus_ring(ui, &response, theme::CONTROL_RADIUS);
                });
            });
        });
        if let Some(id) = chosen {
            self.draft.set_slot(slot, id);
        }
    }

    fn service_card(&self, ui: &mut Ui, service: &Value, can_delete: bool) -> Option<CardAction> {
        let label = self.draft.card_label(service);
        let badges = self.draft.badges(service);
        let key = str_field(service, "provider");
        let id = id_of(service);
        card(ui, |ui| {
            let action = Frame::new()
                .inner_margin(Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    let name_font = theme::bold(theme::TEXT_SIZE);
                    let pill_font = FontId::proportional(11.5);
                    let hint_font = FontId::proportional(theme::HINT_SIZE);
                    let (name_h, pill_h, hint_h) = ui.fonts_mut(|f| {
                        (
                            f.row_height(&name_font),
                            f.row_height(&pill_font),
                            f.row_height(&hint_font),
                        )
                    });
                    let name_h = name_h.max(pill_h + 2.0);
                    let column_h = name_h + 3.0 + hint_h;
                    let size = vec2(ui.available_width(), column_h.max(theme::CONTROL_HEIGHT));
                    let buttons = Id::unique(("service_card_buttons", id));
                    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 14.0;
                        if provider_texture(ui.ctx(), key).is_some() {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
                            paint_provider_icon(ui, key, rect);
                        }
                        let reserved = right_aligned_width(ui, buttons);
                        let text_width = (ui.available_width() - reserved - 16.0).max(24.0);
                        let (column, _) =
                            ui.allocate_exact_size(vec2(text_width, size.y), Sense::hover());
                        let layout = UiBuilder::new()
                            .max_rect(column)
                            .layout(Layout::top_down(Align::Min));
                        // 不經父層配置空間：角色標記太多時截掉，不可把按鈕擠出卡片
                        let mut text = ui.new_child(layout);
                        text.shrink_clip_rect(column);
                        text.spacing_mut().item_spacing.y = 3.0;
                        text.add_space((size.y - column_h) / 2.0);
                        let name = str_field(service, "name");
                        name_line(&mut text, name, &badges, name_font, name_h);
                        text.add(
                            Label::new(
                                RichText::new(describe(service))
                                    .font(hint_font)
                                    .color(theme::TEXT_SECONDARY),
                            )
                            .truncate(),
                        );
                        let (edit, delete) = right_aligned(ui, buttons, |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let edit = secondary_button(ui, &t("button.edit", &[])).clicked();
                            let delete = ui
                                .add_enabled_ui(can_delete, |ui| {
                                    secondary_button(ui, &t("button.delete", &[]))
                                })
                                .inner
                                .clicked();
                            (edit, delete)
                        });
                        if edit {
                            Some(CardAction::Edit(id.to_owned()))
                        } else if delete {
                            Some(CardAction::Delete(id.to_owned()))
                        } else {
                            None
                        }
                    })
                    .inner
                })
                .inner;
            name_card(ui, label);
            action
        })
    }

    fn show_dialog(&mut self, ui: &Ui, env: &dyn SettingsEnv, target_language: &str) {
        match &mut self.dialog {
            Dialog::Closed => {}
            Dialog::Pick => match picker_dialog(ui) {
                Choice::Open => {}
                Choice::Cancel => self.dialog = Dialog::Closed,
                Choice::Done(key) => {
                    let service = new_service(key, &self.draft.services);
                    let form = ServiceForm::new(service, self.draft.services.clone());
                    self.dialog = Dialog::Form {
                        form: Box::new(form),
                        picking: false,
                        errors: Vec::new(),
                    };
                }
            },
            Dialog::Form {
                form,
                picking,
                errors,
            } => {
                let (choice, change) = form_dialog(ui, form, errors, env, target_language);
                *picking |= change;
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
        let question = t(question, &[("name", name)]);
        let spec = DialogSpec {
            salt: "confirm_delete",
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
                            Label::new(RichText::new(question).size(theme::CONTROL_TEXT_SIZE))
                                .wrap(),
                        );
                    });
            },
            |ui| {
                let cancel = secondary_button(ui, &t("button.cancel", &[])).clicked();
                let delete = danger_button(ui, &t("button.delete", &[])).clicked();
                (delete, cancel)
            },
        );
        let (delete, cancel) = shown.footer;
        if delete {
            Choice::Done(())
        } else if cancel || shown.cancel {
            Choice::Cancel
        } else {
            Choice::Open
        }
    }
}

/// 整張卡片是一個以 `label` 為名的群組，螢幕報讀一次念完名稱與角色。
fn name_card(ui: &Ui, label: String) {
    let rect = ui.min_rect();
    ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
        node.set_role(Role::Group);
        node.set_label(label);
        node.set_bounds(accesskit::Rect {
            x0: rect.min.x.into(),
            y0: rect.min.y.into(),
            x1: rect.max.x.into(),
            y1: rect.max.y.into(),
        });
    });
}

enum CardAction {
    Edit(String),
    Delete(String),
}

/// 服務卡片的名稱列：名稱（太長就截斷）後接角色標記。
fn name_line(ui: &mut Ui, name: &str, badges: &[String], font: FontId, height: f32) {
    let size = vec2(ui.available_width(), height);
    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let pill_font = FontId::proportional(11.5);
        let pills: f32 = badges
            .iter()
            .map(|b| {
                let galley =
                    ui.painter()
                        .layout_no_wrap(b.clone(), pill_font.clone(), theme::PILL_TEXT);
                galley.size().x + 16.0 + 8.0
            })
            .sum();
        let room = (ui.available_width() - pills).max(24.0);
        ui.scope(|ui| {
            ui.set_max_width(room);
            ui.add(Label::new(RichText::new(name).font(font)).truncate());
        });
        for badge in badges {
            Frame::new()
                .fill(theme::PILL_BG)
                .corner_radius(10)
                .inner_margin(Margin::symmetric(8, 1))
                .show(ui, |ui| {
                    ui.add(
                        Label::new(
                            RichText::new(badge)
                                .font(pill_font.clone())
                                .color(theme::PILL_TEXT),
                        )
                        .selectable(false),
                    );
                });
        }
    });
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

/// 服務表單對話框，另回傳是否按了〔更改〕服務商；〔確定〕時欄位不完整就在表單底部列出，不關閉。
fn form_dialog(
    ui: &Ui,
    form: &mut ServiceForm,
    errors: &mut Vec<String>,
    env: &dyn SettingsEnv,
    target_language: &str,
) -> (Choice<Value>, bool) {
    if !errors.is_empty() {
        *errors = validate_service(&form.api_values());
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
                    left: 24,
                    right: 24,
                    top: 8,
                    bottom: 16,
                })
                .show(ui, |ui| {
                    let response = form.ui(ui, env, target_language);
                    if !errors.is_empty() {
                        ui.add_space(12.0);
                        let lines: Vec<String> = errors.iter().map(|e| t(e, &[])).collect();
                        callout(ui, &t("dialog.incomplete_title", &[]), &lines);
                    }
                    response.change_provider
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

/// 〔確定〕：欄位完整就回傳要存的服務，否則記下缺的欄位。
fn save(form: &ServiceForm, errors: &mut Vec<String>) -> Choice<Value> {
    let found = validate_service(&form.api_values());
    if !found.is_empty() {
        log(format!(
            "[settings] service not saved: missing {}",
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
