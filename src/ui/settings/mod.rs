//! 設定視窗與首次設定精靈的介面：配色、共用元件與耗時工作的介面。

mod dialog;
pub mod env;
pub mod fields;
pub mod hotkeys;
pub mod language;
pub mod provider_picker;
pub mod service_draft;
pub mod service_form;
pub mod service_list;
pub mod theme;
pub mod thinking;
pub mod widgets;
pub mod window;
pub mod wizard;

use serde_json::Value;

use crate::config::{Config, default_config, py};

/// 設定值；缺鍵時取預設值。
fn setting(cfg: &Config, key: &str) -> Value {
    cfg.get(key)
        .cloned()
        .unwrap_or_else(|| default_config()[key].clone())
}

fn text_setting(cfg: &Config, key: &str) -> String {
    setting(cfg, key).as_str().unwrap_or_default().to_owned()
}

/// 開關值照 Python 真值判定：手改成 `1` 也算開。
fn flag_setting(cfg: &Config, key: &str) -> bool {
    py::truthy(Some(&setting(cfg, key)))
}
