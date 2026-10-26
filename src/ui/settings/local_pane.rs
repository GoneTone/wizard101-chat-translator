//! 「本地模型」分頁：引擎、模型載入與已下載模型三張卡；預先載入、閒置卸載與 MTP 開關是草稿，檔案操作立即生效。

use std::time::Instant;

use eframe::egui::Ui;
use serde_json::Value;

use super::dialog::Choice;
use super::env::SettingsEnv;
use super::local_card::{confirm_danger, section};
use super::local_dialog::{DialogMode, DialogOutcome, DownloadDialog, Tab};
use super::local_engine::EngineCard;
use super::local_models::{self, ModelAction};
use super::local_rate::Rates;
use super::quant_table::gb;
use super::theme;
use super::widgets::{combo_box, focus_ring, name_by_row, setting_row, toggle};
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
    rates: Rates,
}

impl LocalPane {
    pub fn new(cfg: &Config) -> Self {
        Self {
            settings: settings(cfg),
            engine: EngineCard::default(),
            dialog: None,
            confirm_delete: None,
            listed: false,
            rates: Rates::default(),
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
        self.ui_at(ui, env, snapshot, services, Instant::now());
    }

    /// 同 `ui`，`now` 是這份快照的時間（下載速度由相鄰快照推算）。
    pub fn ui_at(
        &mut self,
        ui: &mut Ui,
        env: &dyn SettingsEnv,
        snapshot: &Snapshot,
        services: &[Value],
        now: Instant,
    ) {
        self.rates.observe(now, snapshot);
        // GitHub 匿名呼叫每小時只有 60 次：已經列過就不再列，失敗時改由〔重試〕
        if !self.listed {
            self.listed = true;
            if !matches!(snapshot.builds, Some(Ok(_))) {
                env.local_command(Command::ListBuilds);
            }
        }
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = theme::CARD_GAP;
            self.engine.ui(ui, env, snapshot, &self.rates);
            self.load_card(ui);
            match local_models::ui(ui, env, snapshot, &mut self.settings, &self.rates) {
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
