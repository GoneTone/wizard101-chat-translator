//! 設定視窗測試共用：掛上 `SettingsWindow` 的 kittest harness，以及一筆設定完整的 custom 服務。
#![allow(dead_code)]

use eframe::egui::{self, Ui};
use egui_kittest::Harness;
use serde_json::{Value, json};

use super::kittest::themed_harness;
use super::settings_env::FakeEnv;
use wizard101_chat_translator::config::services::new_service;
use wizard101_chat_translator::config::{Config, default_config};
use wizard101_chat_translator::ui::settings::window::{SettingsAction, SettingsWindow};

pub struct State {
    pub win: SettingsWindow,
    pub env: FakeEnv,
    pub actions: Vec<SettingsAction>,
}

/// 以 `cfg` 開設定視窗，字型依 `font_language` 載入。
pub fn harness(
    cfg: &Config,
    env: FakeEnv,
    size: egui::Vec2,
    font_language: &'static str,
) -> Harness<'static, State> {
    let state = State {
        win: SettingsWindow::new(cfg),
        env,
        actions: Vec::new(),
    };
    themed_harness(
        size,
        font_language,
        |ui: &mut Ui, s: &mut State| {
            let actions = s.win.ui(ui, &s.env);
            s.actions.extend(actions);
        },
        state,
    )
}

/// 一筆 custom 服務，base_url 與 model 都填好（名稱依目前介面語言）。
pub fn custom_service() -> Value {
    let mut service = new_service("custom", &[]);
    service["base_url"] = json!("http://x");
    service["model"] = json!("m");
    service
}

/// 只有一筆完整 custom 服務、並設為預設的設定。
pub fn custom_cfg() -> Config {
    let service = custom_service();
    let mut cfg = default_config();
    cfg.insert("default_service".into(), service["id"].clone());
    cfg.insert("services".into(), json!([service]));
    cfg
}
