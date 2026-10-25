//! 「本地模型」分頁：引擎、模型載入與已下載模型三張卡；預先載入、閒置卸載與 MTP 開關是草稿，檔案操作立即生效。

use eframe::egui::{Frame, Label, Margin, Response, RichText, Ui};
use serde_json::Value;

use super::dialog::{Choice, confirm};
use super::env::SettingsEnv;
use super::local_dialog::{DialogMode, DialogOutcome, DownloadDialog, Tab};
use super::local_engine::EngineCard;
use super::local_models::{self, ModelAction};
use super::quant_table::gb;
use super::theme;
use super::widgets::{
    card, combo_box, danger_button, focus_ring, name_by_row, right_aligned, setting_row, toggle,
};
use crate::config::Config;
use crate::config::local::{IDLE_CHOICES, LocalSettings, settings};
use crate::config::services::{LOCAL_PROVIDER, str_field};
use crate::i18n::t;
use crate::local::store::total_size;
use crate::local::{Command, Snapshot};
use crate::logs::log;

/// 本地模型分頁的狀態；`settings()` 是編輯中的草稿，設定視窗按儲存時寫回。
pub struct LocalPane {
    settings: LocalSettings,
    engine: EngineCard,
    dialog: Option<DownloadDialog>,
    /// 等使用者確認要不要刪的模型 id（被本地服務用著的才會問）。
    confirm_delete: Option<String>,
    listed: bool,
}

impl LocalPane {
    pub fn new(cfg: &Config) -> Self {
        Self {
            settings: settings(cfg),
            engine: EngineCard::default(),
            dialog: None,
            confirm_delete: None,
            listed: false,
        }
    }

    pub fn settings(&self) -> &LocalSettings {
        &self.settings
    }

    /// 畫三張卡與開著的對話框；`services` 是編輯中的服務清單，用來判斷模型有沒有被引用。
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        env: &dyn SettingsEnv,
        snapshot: &Snapshot,
        services: &[Value],
    ) {
        if !self.listed {
            self.listed = true;
            env.local_command(Command::ListBuilds);
        }
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = theme::CARD_GAP;
            self.engine.ui(ui, env, snapshot);
            self.load_card(ui);
            match local_models::ui(ui, env, snapshot, &mut self.settings) {
                Some(ModelAction::OpenDownload) => {
                    self.dialog = Some(DownloadDialog::new(DialogMode::Download, Tab::Recommended));
                }
                Some(ModelAction::Delete(id)) => self.request_delete(env, id, services),
                None => {}
            }
        });
        self.show_dialogs(ui, env, snapshot, services);
    }

    fn load_card(&mut self, ui: &mut Ui) {
        section(
            ui,
            &t("local.load.title", &[]),
            |_| {},
            |ui| {
                let preload = t("local.load.preload", &[]);
                let hint = t("local.load.preload_hint", &[]);
                setting_row(ui, &preload, Some(&hint), |ui| {
                    toggle(ui, &mut self.settings.preload, &preload);
                });
                let hint = t("local.load.idle_hint", &[]);
                setting_row(ui, &t("local.load.idle", &[]), Some(&hint), |ui| {
                    self.idle_combo(ui);
                });
            },
        );
    }

    fn idle_combo(&mut self, ui: &mut Ui) {
        let combo = combo_box(ui, "local_idle")
            .selected_text(idle_text(self.settings.idle_minutes))
            .show_ui(ui, |ui| {
                for minutes in IDLE_CHOICES {
                    let on = self.settings.idle_minutes == minutes;
                    if ui.selectable_label(on, idle_text(minutes)).clicked() {
                        self.settings.idle_minutes = minutes;
                    }
                }
            });
        let response = name_by_row(ui, combo.response);
        focus_ring(ui, &response, theme::CONTROL_RADIUS);
    }

    /// 沒有本地服務用著就直接刪，否則先問。
    fn request_delete(&mut self, env: &dyn SettingsEnv, id: String, services: &[Value]) {
        if users_of(services, &id).is_empty() {
            delete(env, id);
        } else {
            self.confirm_delete = Some(id);
        }
    }

    fn show_dialogs(
        &mut self,
        ui: &Ui,
        env: &dyn SettingsEnv,
        snapshot: &Snapshot,
        services: &[Value],
    ) {
        if let Some(dialog) = &mut self.dialog
            && dialog.ui(ui, env, snapshot) != DialogOutcome::Open
        {
            self.dialog = None;
        }
        let Some(id) = self.confirm_delete.clone() else {
            return;
        };
        let Some(model) = snapshot.models.iter().find(|m| m.id == id) else {
            self.confirm_delete = None;
            return;
        };
        let size = gb(total_size(std::slice::from_ref(model)));
        let names = users_of(services, &id).join(&t("local.list_separator", &[]));
        let question = t(
            "local.models.confirm_delete",
            &[("services", &names), ("size", &size)],
        );
        match confirm_danger(ui, "confirm_delete_model", question) {
            Choice::Open => {}
            Choice::Cancel => self.confirm_delete = None,
            Choice::Done(()) => {
                self.confirm_delete = None;
                delete(env, id);
            }
        }
    }
}

fn delete(env: &dyn SettingsEnv, id: String) {
    log(format!("[ui] model delete requested: id={id}"));
    env.local_command(Command::Delete(id));
}

/// 使用 `model` 這個模型的本地服務名稱。
fn users_of(services: &[Value], model: &str) -> Vec<String> {
    services
        .iter()
        .filter(|s| str_field(s, "provider") == LOCAL_PROVIDER && str_field(s, "model") == model)
        .map(|s| str_field(s, "name").to_owned())
        .collect()
}

fn idle_text(minutes: u32) -> String {
    match minutes {
        0 => t("local.load.idle_never", &[]),
        m => t("local.load.idle_minutes", &[("minutes", &m.to_string())]),
    }
}

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
