//! 本地模型設定：config 的 `local_models` 區塊（預先載入、閒置卸載、prompt 快取上限、各模型的 MTP 開關）與載入時的正規化。

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::Config;
use crate::logs::log;

const KEY: &str = "local_models";

/// 閒置幾分鐘後卸載模型的可選值；0 代表不卸載。
pub const IDLE_CHOICES: [u32; 4] = [0, 5, 15, 30];
/// llama-server 存在 RAM 的 prompt 快取上限（MiB）可選值；0 代表關閉。
pub const PROMPT_CACHE_CHOICES: [u32; 5] = [0, 1024, 2048, 4096, 8192];

const DEFAULT_PRELOAD: bool = true;
const DEFAULT_IDLE_MINUTES: u32 = 0;
/// llama-server 自己的預設值。
const DEFAULT_PROMPT_CACHE_MIB: u32 = 8192;

/// `local_models` 區塊的讀出形狀。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalSettings {
    /// 本軟體開啟時就預先載入模型。
    pub preload: bool,
    /// 閒置幾分鐘後卸載；0 代表不卸載。
    pub idle_minutes: u32,
    /// prompt 快取上限（MiB，`--cache-ram`）；0 代表關閉。
    pub prompt_cache_mib: u32,
    /// 模型 id 對應的 MTP 開關。
    pub mtp: BTreeMap<String, bool>,
}

impl LocalSettings {
    /// 模型的 MTP 開關；沒設定過的預設開啟。
    pub fn mtp_on(&self, id: &str) -> bool {
        self.mtp.get(id).copied().unwrap_or(true)
    }
}

impl Default for LocalSettings {
    fn default() -> Self {
        Self {
            preload: DEFAULT_PRELOAD,
            idle_minutes: DEFAULT_IDLE_MINUTES,
            prompt_cache_mib: DEFAULT_PROMPT_CACHE_MIB,
            mtp: BTreeMap::new(),
        }
    }
}

/// 預設的 `local_models` 區塊。
pub fn default_value() -> Value {
    to_value(&LocalSettings::default())
}

fn to_value(s: &LocalSettings) -> Value {
    json!({
        "preload": s.preload,
        "idle_minutes": s.idle_minutes,
        "prompt_cache_mib": s.prompt_cache_mib,
        "mtp": s.mtp,
    })
}

/// 讀出設定；缺漏或型別不對的欄位取預設值。
pub fn settings(cfg: &Config) -> LocalSettings {
    let block = cfg.get(KEY);
    let defaults = LocalSettings::default();
    LocalSettings {
        preload: block
            .and_then(|b| b.get("preload"))
            .and_then(Value::as_bool)
            .unwrap_or(defaults.preload),
        idle_minutes: block
            .and_then(|b| b.get("idle_minutes"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| IDLE_CHOICES.contains(n))
            .unwrap_or(defaults.idle_minutes),
        prompt_cache_mib: block
            .and_then(|b| b.get("prompt_cache_mib"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| PROMPT_CACHE_CHOICES.contains(n))
            .unwrap_or(defaults.prompt_cache_mib),
        mtp: block
            .and_then(|b| b.get("mtp"))
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(id, on)| Some((id.clone(), on.as_bool()?)))
            .collect(),
    }
}

/// 把設定寫回 config 的 `local_models` 區塊。
pub fn write_into(cfg: &mut Config, s: &LocalSettings) {
    cfg.insert(KEY.into(), to_value(s));
}

/// 載入時把 `local_models` 修成合法形狀；有改動回 `true`（該寫回 config.json）。
pub fn normalize(cfg: &mut Config) -> bool {
    let clean = to_value(&settings(cfg));
    if cfg.get(KEY) == Some(&clean) {
        return false;
    }
    log(format!(
        "[config] normalized local_models (changed: {})",
        changed_keys(cfg.get(KEY), &clean).join(", ")
    ));
    cfg.insert(KEY.into(), clean);
    true
}

/// 舊區塊與正規化結果不同的鍵名（含被丟掉的未知鍵）；區塊本身不是物件時回 `["local_models"]`。
fn changed_keys(old: Option<&Value>, clean: &Value) -> Vec<String> {
    let (Some(old), Some(clean)) = (old.and_then(Value::as_object), clean.as_object()) else {
        return vec![KEY.to_owned()];
    };
    let mut keys: Vec<String> = old
        .keys()
        .chain(clean.keys())
        .filter(|k| old.get(*k) != clean.get(*k))
        .cloned()
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_config;

    #[test]
    fn defaults_to_preload_and_never_unloading() {
        let s = settings(&default_config());
        assert_eq!(
            s,
            LocalSettings {
                preload: true,
                idle_minutes: 0,
                prompt_cache_mib: 8192,
                mtp: BTreeMap::new()
            }
        );
    }

    #[test]
    fn normalize_snaps_unknown_idle_values_to_default() {
        let mut cfg = default_config();
        cfg.insert(
            "local_models".into(),
            json!({"preload": "yes", "idle_minutes": 7, "prompt_cache_mib": 3000, "mtp": {"a": true, "b": 1}}),
        );
        assert!(normalize(&mut cfg));
        assert_eq!(
            settings(&cfg),
            LocalSettings {
                preload: true,
                idle_minutes: 0,
                prompt_cache_mib: 8192,
                mtp: [("a".into(), true)].into()
            }
        );
    }

    #[test]
    fn normalize_leaves_a_clean_block_alone() {
        let mut cfg = default_config();
        assert!(!normalize(&mut cfg));
    }
}
