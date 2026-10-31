//! 「本地模型」分頁：引擎、模型載入與已下載模型三張卡；預先載入、閒置卸載與 MTP 開關是草稿，檔案操作立即生效。

use std::time::Instant;

use eframe::egui::Ui;
use serde_json::Value;

use super::dialog::{Choice, confirm_delete};
use super::env::SettingsEnv;
use super::local_card::section;
use super::local_dialog::{DialogMode, DialogOutcome, DownloadDialog, Tab};
use super::local_engine::EngineCard;
use super::local_models::{self, ModelAction};
use super::local_rate::Rates;
use super::quant_table::gb;
use super::theme;
use super::widgets::{combo_box, focus_ring, full_text_tip, name_by_row, setting_row, toggle};
use crate::config::Config;
use crate::config::local::{IDLE_CHOICES, LocalSettings, PROMPT_CACHE_CHOICES, settings};
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
    /// 這次打開設定視窗後畫過這一頁。
    opened: bool,
    rates: Rates,
}

impl LocalPane {
    pub fn new(cfg: &Config) -> Self {
        Self {
            settings: settings(cfg),
            engine: EngineCard::default(),
            dialog: None,
            confirm_delete: None,
            opened: false,
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
        if !self.opened {
            self.opened = true;
            // 硬體只在量的當下準：打開這頁就重量一次可用量與本軟體模型的實際佔用
            env.local_command(Command::RefreshHardware {
                redetect_gpu: false,
            });
            // GitHub 匿名呼叫每小時只有 60 次：已經列過就不再列，失敗時改由〔重試〕
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
                Some(ModelAction::Delete(id)) => self.confirm_delete = Some(id),
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
                let hint = t("local.load.prompt_cache_hint", &[]);
                setting_row(ui, &t("local.load.prompt_cache", &[]), Some(&hint), |ui| {
                    self.prompt_cache_combo(ui);
                });
            },
        );
    }

    fn idle_combo(&mut self, ui: &mut Ui) {
        let selected = idle_text(self.settings.idle_minutes);
        let combo = combo_box(ui, "local_idle")
            .selected_text(selected.clone())
            .show_ui(ui, |ui| {
                for minutes in IDLE_CHOICES {
                    let on = self.settings.idle_minutes == minutes;
                    if ui.selectable_label(on, idle_text(minutes)).clicked() {
                        self.settings.idle_minutes = minutes;
                    }
                }
            });
        let response = name_by_row(ui, combo.response);
        full_text_tip(ui, &response, &selected, 0.0);
        focus_ring(ui, &response, theme::CONTROL_RADIUS);
    }

    fn prompt_cache_combo(&mut self, ui: &mut Ui) {
        let selected = prompt_cache_text(self.settings.prompt_cache_mib);
        let combo = combo_box(ui, "local_prompt_cache")
            .selected_text(selected.clone())
            .show_ui(ui, |ui| {
                for mib in PROMPT_CACHE_CHOICES {
                    let on = self.settings.prompt_cache_mib == mib;
                    if ui.selectable_label(on, prompt_cache_text(mib)).clicked() {
                        self.settings.prompt_cache_mib = mib;
                    }
                }
            });
        let response = name_by_row(ui, combo.response);
        full_text_tip(ui, &response, &selected, 0.0);
        focus_ring(ui, &response, theme::CONTROL_RADIUS);
    }

    /// 沒有本地服務用著就直接刪，否則先問。
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
        let users = users_of(services, &id);
        let name = model.manifest.display_name();
        let question = if users.is_empty() {
            t(
                "local.models.confirm_delete_unused",
                &[("name", &name), ("size", &size)],
            )
        } else {
            let names = users.join(&t("local.list_separator", &[]));
            t(
                "local.models.confirm_delete",
                &[("services", &names), ("name", &name), ("size", &size)],
            )
        };
        // 對話框窄，數字與單位之間用不換行空白，免得「2.7」和「GB」分在兩行
        let question = question.replace(" GB", "\u{a0}GB");
        match confirm_delete(ui, "confirm_delete_model", question) {
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

fn prompt_cache_text(mib: u32) -> String {
    match mib {
        0 => t("local.load.prompt_cache_off", &[]),
        m => t(
            "local.load.prompt_cache_gb",
            &[("gb", &(m / 1024).to_string())],
        ),
    }
}

fn idle_text(minutes: u32) -> String {
    match minutes {
        0 => t("local.load.idle_never", &[]),
        m => t("local.load.idle_minutes", &[("minutes", &m.to_string())]),
    }
}
