//! 「帳號」分頁：遊戲位置卡、帳號清單（執行中標記、單獨開啟、編輯、刪除）與底部的 Steam 限制說明。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{Id, Label, RichText, Ui};
use serde_json::Value;

use super::account_dialog::{AccountDialog, kind_label};
use super::dialog::{Choice, confirm_delete};
use super::env::{Round, SettingsEnv, SettingsEvent};
use super::theme;
use super::widgets::{
    card, danger_secondary_button, dashed_button, hint, list_card, secondary_button,
    setting_row_with,
};
use crate::config::Config;
use crate::config::services::new_id;
use crate::i18n::t;
use crate::launcher::accounts::{self, Account};
use crate::launcher::install::{Install, InstallSource, installs_with, is_valid_install};
use crate::launcher::orchestrate::LaunchStatus;
use crate::launcher::running::RUNNING_REFRESH_SECS;
use crate::logs::log;
use crate::ui::overlay::launch_popup::{status_color, status_text};

enum Dialog {
    Closed,
    Edit(Box<AccountDialog>),
    Confirm { id: String },
}

enum CardAction {
    Launch(String),
    Edit(String),
    Delete(String),
}

/// 帳號分頁；帳號清單與遊戲位置是編輯中的草稿，設定視窗按儲存時寫回（`write_into`）。
pub struct AccountsPane {
    accounts: Vec<Account>,
    /// 使用者指定的遊戲位置；`None` 是用偵測結果。
    game_path: Option<PathBuf>,
    dialog: Dialog,
    /// 偵測到的安裝；進入分頁時才在背景偵測，結果回來前是 `None`。
    detected: Option<Vec<Install>>,
    /// 偵測還在背景跑。
    detecting: bool,
    running: Vec<String>,
    /// 上次要求掃描執行中帳號的時間；`None` 是下一幀要掃。
    running_at: Option<f64>,
    /// 執行中帳號的掃描還在背景跑；同時只有一次。
    scanning: bool,
    /// 各帳號（id）最近一次的開啟進度。
    statuses: HashMap<String, LaunchStatus>,
    folder_round: Round,
    /// 上次選到的資料夾不是有效的安裝。
    invalid_folder: bool,
    /// 這一幀對話框存好的帳號；外層讀走（`take_committed`）後寫檔。
    committed: Option<Account>,
    /// 剛選定了有效的遊戲位置；外層讀走（`take_path_committed`）後寫檔，設定頁的〔開啟〕才用得到。
    path_committed: bool,
    /// 這一幀按了〔開啟〕的帳號 id；外層讀走（`take_launch`）。
    launch: Option<String>,
}

impl AccountsPane {
    pub fn new(cfg: &Config) -> Self {
        Self {
            accounts: accounts::stored(cfg),
            game_path: cfg
                .get("game_path")
                .and_then(Value::as_str)
                .map(PathBuf::from),
            dialog: Dialog::Closed,
            detected: None,
            detecting: false,
            running: Vec::new(),
            running_at: None,
            scanning: false,
            statuses: HashMap::new(),
            folder_round: Round::new("game folder dialog"),
            invalid_folder: false,
            committed: None,
            path_committed: false,
            launch: None,
        }
    }

    /// 草稿寫進 `cfg` 的 `accounts` 與 `game_path`（沒指定時是 null）。
    pub fn write_into(&self, cfg: &mut Config) {
        accounts::store(cfg, &self.accounts);
        cfg.insert("game_path".into(), self.game_path_value());
    }

    /// 草稿的 `game_path` 設定值（沒指定時是 null）。
    pub fn game_path_value(&self) -> Value {
        self.game_path
            .as_ref()
            .map(|p| p.display().to_string())
            .into()
    }

    /// 進入分頁：重新偵測安裝位置與執行中帳號。
    pub fn entered(&mut self) {
        self.detected = None;
        self.running_at = None;
    }

    /// 帳號對話框開著，且有尚未存下的修改。
    pub fn unsaved(&self) -> bool {
        matches!(&self.dialog, Dialog::Edit(dialog) if dialog.unsaved())
    }

    /// 這一幀對話框存好的帳號；讀走就清掉。
    pub fn take_committed(&mut self) -> Option<Account> {
        self.committed.take()
    }

    /// 剛選定了有效的遊戲位置；讀走就清掉。
    pub fn take_path_committed(&mut self) -> bool {
        std::mem::take(&mut self.path_committed)
    }

    /// 這一幀按了〔開啟〕的帳號 id；讀走就清掉。
    pub fn take_launch(&mut self) -> Option<String> {
        self.launch.take()
    }

    /// 背景結果：偵測到的安裝、執行中帳號、開啟進度與選到的資料夾。
    pub fn apply(&mut self, event: &SettingsEvent) {
        match event {
            SettingsEvent::InstallsDetected { installs } => {
                self.detected = Some(installs.clone());
                self.detecting = false;
            }
            SettingsEvent::RunningAccounts { names } => {
                self.running = names.clone();
                self.scanning = false;
            }
            SettingsEvent::LaunchStatus { id, status } => {
                self.statuses.insert(id.clone(), status.clone());
            }
            SettingsEvent::GameFolderPicked { round, path } => {
                self.folder_picked(*round, path.as_deref())
            }
            _ => {}
        }
    }

    /// 是有效的安裝才採用，否則顯示錯誤；過期或取消的結果丟掉。
    fn folder_picked(&mut self, round: u64, path: Option<&Path>) {
        if !self.folder_round.finish(round) {
            return;
        }
        let Some(path) = path else {
            log("[settings] game folder dialog cancelled");
            return;
        };
        self.invalid_folder = !is_valid_install(path);
        if self.invalid_folder {
            log(format!(
                "[settings] picked folder is not a game install: {}",
                path.display()
            ));
        } else {
            log(format!("[settings] game folder set to {}", path.display()));
            self.game_path = Some(path.to_path_buf());
            self.path_committed = true;
        }
    }

    /// 畫分頁與開著的對話框。
    pub fn ui(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) {
        self.refresh(ui, env);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = theme::CARD_GAP;
            self.location_card(ui, env);
            ui.add_space(6.0);
            ui.add(Label::new(
                RichText::new(t("accounts.my_accounts", &[])).font(theme::bold(theme::TEXT_SIZE)),
            ));
            let mut action = None;
            for account in &self.accounts {
                action = action.or(self.account_card(ui, account));
            }
            if dashed_button(ui, &t("button.add_account", &[]), 40.0).clicked() {
                let ids: Vec<Value> = self
                    .accounts
                    .iter()
                    .map(|a| serde_json::json!({ "id": a.id }))
                    .collect();
                self.dialog = Dialog::Edit(Box::new(AccountDialog::new(new_id(&ids))));
            }
            hint(ui, &t("accounts.steam_limit", &[]), theme::TEXT_SECONDARY);
            match action {
                Some(CardAction::Launch(id)) => {
                    log(format!("[launch] launch requested from settings: {id}"));
                    self.launch = Some(id);
                }
                Some(CardAction::Edit(id)) => {
                    if let Some(account) = self.accounts.iter().find(|a| a.id == id) {
                        self.dialog = Dialog::Edit(Box::new(AccountDialog::edit(account)));
                    }
                }
                Some(CardAction::Delete(id)) => self.dialog = Dialog::Confirm { id },
                None => {}
            }
        });
        self.show_dialog(ui);
    }

    fn refresh(&mut self, ui: &Ui, env: &dyn SettingsEnv) {
        if self.detected.is_none() && !self.detecting {
            self.detecting = true;
            env.detect_installs();
        }
        let now = ui.input(|i| i.time);
        if !self.scanning
            && self
                .running_at
                .is_none_or(|at| now - at >= RUNNING_REFRESH_SECS)
        {
            self.scanning = true;
            self.running_at = Some(now);
            env.scan_running_accounts();
        }
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f64(RUNNING_REFRESH_SECS));
    }

    fn location_card(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) {
        let detecting = self.detected.is_none();
        let detected = self.detected.clone().unwrap_or_default();
        let installs = installs_with(detected, self.game_path.as_deref());
        let invalid = self.invalid_folder;
        let mut change = false;
        card(ui, |ui| {
            let details = |ui: &mut Ui| {
                if detecting {
                    hint(ui, &t("accounts.detecting", &[]), theme::TEXT_SECONDARY);
                } else if installs.is_empty() {
                    hint(ui, &t("accounts.install_none", &[]), theme::TEXT_SECONDARY);
                }
                for install in &installs {
                    let key = match install.source {
                        InstallSource::Steam => "accounts.install_steam",
                        InstallSource::Official => "accounts.install_official",
                    };
                    let path = install.root.display().to_string();
                    hint(ui, &t(key, &[("path", &path)]), theme::TEXT_SECONDARY);
                }
                if invalid {
                    hint(ui, &t("accounts.invalid_folder", &[]), theme::ERROR);
                }
            };
            setting_row_with(ui, &t("accounts.game_location", &[]), details, |ui| {
                change = secondary_button(ui, &t("button.change", &[])).clicked();
            });
        });
        if change {
            let round = self.folder_round.start();
            env.pick_game_folder(round);
        }
    }

    fn account_card(&self, ui: &mut Ui, account: &Account) -> Option<CardAction> {
        let running = self
            .running
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&account.name));
        // 窄視窗放不下時後面的標記會被截掉，執行中比類型重要
        let status = self.statuses.get(&account.id);
        let busy = running || status == Some(&LaunchStatus::LoggingIn);
        let mut badges: Vec<String> = running
            .then(|| t("accounts.running", &[]))
            .into_iter()
            .collect();
        badges.push(kind_label(account.kind));
        let (launch, edit, delete) = list_card(
            ui,
            Id::unique(("account_card_buttons", &account.id)),
            |_| {},
            &account.name,
            &badges,
            "",
            |ui| {
                let launch = ui
                    .add_enabled_ui(!busy, |ui| secondary_button(ui, &t("button.launch", &[])))
                    .inner
                    .clicked();
                let edit = secondary_button(ui, &t("button.edit", &[])).clicked();
                let delete = danger_secondary_button(ui, &t("button.delete", &[])).clicked();
                (launch, edit, delete)
            },
        );
        if let Some(status) = status {
            hint(ui, &status_text(status), status_color(status));
            if status.is_rejected() {
                hint(ui, &t("launch.update_hint", &[]), theme::TEXT_SECONDARY);
            }
        }
        let id = account.id.clone();
        if launch {
            Some(CardAction::Launch(id))
        } else if edit {
            Some(CardAction::Edit(id))
        } else if delete {
            Some(CardAction::Delete(id))
        } else {
            None
        }
    }

    fn show_dialog(&mut self, ui: &Ui) {
        match &mut self.dialog {
            Dialog::Closed => {}
            Dialog::Edit(dialog) => match dialog.show(ui, &self.accounts) {
                Choice::Open => {}
                Choice::Cancel => self.dialog = Dialog::Closed,
                Choice::Done(account) => {
                    accounts::upsert(&mut self.accounts, account.clone());
                    self.committed = Some(account);
                    self.dialog = Dialog::Closed;
                }
            },
            Dialog::Confirm { id } => {
                let id = id.clone();
                let name = self
                    .accounts
                    .iter()
                    .find(|a| a.id == id)
                    .map_or("", |a| a.name.as_str());
                let question = t("accounts.confirm_delete", &[("name", name)]);
                match confirm_delete(ui, "confirm_delete_account", question) {
                    Choice::Open => {}
                    Choice::Cancel => self.dialog = Dialog::Closed,
                    Choice::Done(()) => {
                        self.accounts.retain(|a| a.id != id);
                        log(format!("[settings] account {id} deleted"));
                        self.dialog = Dialog::Closed;
                    }
                }
            }
        }
    }
}
