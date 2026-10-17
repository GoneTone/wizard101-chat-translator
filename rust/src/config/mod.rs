//! config.json 讀寫：缺漏欄位以預設值補齊、舊格式遷移後立刻落地，壞檔絕不覆寫。

pub(crate) mod py;
pub mod services;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Number, Value, json};

pub use services::{ServiceSlot, ServiceSpec};

use crate::i18n::{SOURCE_LANGUAGE, detect_system_language, language_name, t};
use crate::logs::log;
use services::{UNSUPPORTED_SERVICES, find, normalize, quarantined, validate_service};

/// 整份設定：保留未知欄位與鍵的順序，寫回時才能與 Python 版逐 byte 相同。
pub type Config = Map<String, Value>;

/// 應用程式顯示名稱（隨介面語言變動）。
pub fn app_name() -> String {
    t("app.name", &[])
}

/// 應用程式目錄：config.json 與 log 的存放處；發行版為 exe 所在目錄，開發時為專案根目錄。
pub fn app_dir() -> PathBuf {
    if cfg!(debug_assertions) {
        return PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_default()
}

/// 本機狀態目錄：跨啟動保留、但不屬於使用者資料的檔案（hook 修復狀態、譯文快取）。
pub fn local_state_dir() -> PathBuf {
    state_dir_under(std::env::var_os("LOCALAPPDATA"))
}

/// `local_state_dir` 的本體：`localappdata` 沒設或是空字串時退回使用者家目錄。
fn state_dir_under(localappdata: Option<std::ffi::OsString>) -> PathBuf {
    localappdata
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(std::env::home_dir)
        .unwrap_or_default()
        .join("wizard101-chat-translator")
}

/// config.json 的路徑。
pub fn config_path() -> PathBuf {
    app_dir().join("config.json")
}

fn default_config() -> Config {
    let slots: Map<String, Value> = ServiceSlot::ALL
        .iter()
        .map(|s| (s.key().to_string(), Value::Null))
        .collect();
    let Value::Object(cfg) = json!({
        "config_version": 2,
        "services": [],
        "default_service": null,
        "service_slots": slots,
        (UNSUPPORTED_SERVICES): [],
        "ui_language": null,
        "target_language": language_name(SOURCE_LANGUAGE),
        "fade_seconds": 0,
        "max_messages": 200,
        "max_parallel_translations": 4,
        "translate_system_messages": false,
        "hotkey": "ctrl+space",
        "region_hotkey": "ctrl+shift+space",
        "auto_show_input": true,
        "paste_hotkey": true,
        "type_delay": 0.02,
        "overlay_alpha": 0.80,
        "overlay": {"x": null, "y": null, "width": 640, "height": 420},
        "bubble_position": {"x": null, "y": null},
    }) else {
        unreachable!("json! object literal")
    };
    cfg
}

/// 進階數值的安全範圍（鍵, 下限, 上限, 是否為浮點數）；Python 的 int／float 型別要原樣保留。
const ADVANCED_LIMITS: &[(&str, f64, f64, bool)] = &[
    ("fade_seconds", 0.0, 3600.0, false),
    ("max_messages", 10.0, 1000.0, false),
    ("max_parallel_translations", 1.0, 8.0, false),
    ("type_delay", 0.0, 0.5, true),
    ("overlay_alpha", 0.3, 1.0, true),
];

fn bound(x: f64, is_float: bool) -> Value {
    if is_float {
        Number::from_f64(x).map_or(Value::Null, Value::Number)
    } else {
        Value::Number((x as i64).into())
    }
}

/// 把進階數值夾在安全範圍；非數值退回預設值並留 log。
fn clamp_advanced(values: &mut Config, defaults: &Config) {
    for &(key, lo, hi, is_float) in ADVANCED_LIMITS {
        let mut value = values.get(key).cloned().unwrap_or(Value::Null);
        if !value.is_number() {
            log(format!(
                "[config] {key} is not a number ({}); using default {}",
                py::repr(&value),
                py::display(&defaults[key])
            ));
            value = defaults[key].clone();
        }
        // 照 Python min(hi, max(lo, value)) 的取值：相等時留下先出現的那個（連同型別）
        let x = py::number(&value).unwrap_or(lo);
        let value = if x > lo { value } else { bound(lo, is_float) };
        let x = py::number(&value).unwrap_or(hi);
        let value = if x < hi { value } else { bound(hi, is_float) };
        values.insert(key.into(), value);
    }
}

fn merge(base: &Config, overrides: &Config) -> Config {
    let mut out = base.clone();
    for (key, value) in overrides {
        let merged = match (value, out.get(key)) {
            (Value::Object(o), Some(Value::Object(b))) => Value::Object(merge(b, o)),
            _ => value.clone(),
        };
        out.insert(key.clone(), merged);
    }
    out
}

fn read_object(path: &Path) -> Result<Option<Config>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    match serde_json::from_str(&text).map_err(|e| e.to_string())? {
        Value::Object(map) => Ok(Some(map)),
        other => Err(format!(
            "top level is {}, expected an object",
            py::type_name(&other)
        )),
    }
}

/// 讀 config.json 並補齊、遷移；檔案不存在或壞掉時回預設值。
///
/// 壞檔不覆寫：使用者的金鑰還在裡面，留給他自己修，只在 log 說明原因。
pub fn load_config(path: &Path) -> Config {
    let defaults = default_config();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let data = match read_object(path) {
        Ok(Some(data)) => data,
        Ok(None) => return defaults,
        Err(error) => {
            log(format!(
                "[config] {name} is unreadable, using defaults (fix the file to restore your settings): {error}"
            ));
            return defaults;
        }
    };
    let mut cfg = merge(&defaults, &data);
    clamp_advanced(&mut cfg, &defaults);
    let mut changed = normalize(&mut cfg);
    // 已移除的設定：留在檔案裡會讓手改的人以為它還有作用
    if cfg.shift_remove("poll_interval").is_some() {
        log("[config] dropped removed setting poll_interval");
        changed = true;
    }
    if changed {
        match save_config(path, &cfg) {
            Ok(()) => log(format!("[config] rewrote {name} after normalizing it")),
            Err(error) => log(format!("[config] could not rewrite {name}: {error}")),
        }
    }
    cfg
}

/// 把整份設定寫回 config.json；與 Python 的 `json.dumps(indent=2)` 經 `write_text` 逐 byte 相同（CRLF、無結尾換行）。
pub fn save_config(path: &Path, cfg: &Config) -> io::Result<()> {
    let text = py::dumps_indent2(&Value::Object(cfg.clone()));
    fs::write(path, text.replace('\n', "\r\n"))
}

/// 預設服務存在且設定完整（不完整 → 啟動時進首次設定精靈）。
pub fn is_configured(cfg: &Config) -> bool {
    let default = cfg.get("default_service").and_then(Value::as_str);
    find(cfg, default).is_some_and(|s| validate_service(s).is_empty())
}

/// 決定啟動時的介面語言碼；真正首次執行時順便把 target_language 補成同語言。
///
/// 首次執行只能看 `config_existed`：舊 config 補值後 ui_language 同樣是 null。
pub fn bootstrap_language(cfg: &mut Config, config_existed: bool) -> String {
    bootstrap_language_with(cfg, config_existed, detect_system_language)
}

/// [`bootstrap_language`] 的本體，`detect` 只在 ui_language 未設定時才呼叫（供測試注入）。
pub fn bootstrap_language_with(
    cfg: &mut Config,
    config_existed: bool,
    detect: impl FnOnce() -> String,
) -> String {
    match cfg.get("ui_language") {
        None | Some(Value::Null) => {
            let detected = detect();
            if !config_existed {
                cfg.insert("target_language".into(), language_name(&detected).into());
            }
            detected
        }
        Some(code) => py::display(code),
    }
}

fn service_summary(service: Option<&Value>) -> String {
    let Some(service) = service else {
        return "none".into();
    };
    let field = |k: &str| py::display(service.get(k).unwrap_or(&Value::Null));
    format!(
        "{}({}/{},has_key={})",
        field("name"),
        field("provider"),
        field("model"),
        py::py_bool(py::truthy(service.get("api_key")))
    )
}

/// 一行設定摘要（啟動與套用設定時記錄）；金鑰只記有沒有，絕不列入內容。
pub fn config_summary(cfg: &Config) -> String {
    let get = |k: &str| cfg.get(k).unwrap_or(&Value::Null);
    let services = get("services").as_array().map_or(0, Vec::len);
    let default = get("default_service").as_str();
    let mut fields = vec![
        format!("services={services}"),
        // 隔離筆數：使用者回報「服務不見了」時，這一格就是答案
        format!("quarantined={}", quarantined(cfg).len()),
        format!("default={}", service_summary(find(cfg, default))),
    ];
    for slot in ServiceSlot::ALL {
        let id = get("service_slots").get(slot.key());
        let summary = if py::truthy(id) {
            service_summary(find(cfg, id.and_then(Value::as_str)))
        } else {
            "default".into()
        };
        fields.push(format!("{}={summary}", slot.key()));
    }
    fields.push(format!(
        "target_language={}",
        py::repr(get("target_language"))
    ));
    for (label, key) in [
        ("ui_language", "ui_language"),
        ("hotkey", "hotkey"),
        ("region_hotkey", "region_hotkey"),
        ("paste_hotkey", "paste_hotkey"),
        ("auto_show_input", "auto_show_input"),
        ("parallel", "max_parallel_translations"),
        ("fade_seconds", "fade_seconds"),
        ("max_messages", "max_messages"),
        ("overlay_alpha", "overlay_alpha"),
        ("translate_system_messages", "translate_system_messages"),
    ] {
        fields.push(format!("{label}={}", py::display(get(key))));
    }
    fields.join(", ")
}

/// 視窗位置與大小；x／y 為 `None` 表示尚未拖曳過。
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    /// 左上角 x。
    pub x: Option<i32>,
    /// 左上角 y。
    pub y: Option<i32>,
    /// 寬度。
    pub width: i32,
    /// 高度。
    pub height: i32,
}

/// 執行期唯讀的設定檢視。
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// 訊息淡出秒數；0 表示永不淡出。
    pub fade_seconds: u32,
    /// 視窗保留的訊息則數上限。
    pub max_messages: usize,
    /// 同時翻譯則數。
    pub max_parallel_translations: usize,
    /// 是否翻譯遊戲系統訊息。
    pub translate_system_messages: bool,
    /// 收訊的目標語言（人讀名稱）。
    pub target_language: String,
    /// 疊加視窗的位置與大小。
    pub overlay: Geometry,
    /// 縮小泡泡的位置；`None` 表示跟著疊加視窗右上角。
    pub bubble_position: (Option<i32>, Option<i32>),
    /// 視窗不透明度。
    pub overlay_alpha: f32,
}

fn lookup<'a>(cfg: &'a Config, path: &[&str]) -> Option<&'a Value> {
    let (first, rest) = path.split_first()?;
    rest.iter().try_fold(cfg.get(*first)?, |v, k| v.get(k))
}

impl Settings {
    /// 從（已載入、正規化過的）設定取出執行期需要的值；手改成非數值的欄位退回預設值。
    pub fn from_config(cfg: &Config) -> Self {
        let defaults = default_config();
        let own = |path: &[&str]| lookup(cfg, path).and_then(Value::as_f64);
        let num = |path: &[&str]| {
            own(path)
                .or_else(|| lookup(&defaults, path).and_then(Value::as_f64))
                .unwrap_or_default()
        };
        let coord = |path: &[&str]| own(path).map(|v| v as i32);
        Settings {
            fade_seconds: num(&["fade_seconds"]) as u32,
            max_messages: num(&["max_messages"]) as usize,
            max_parallel_translations: num(&["max_parallel_translations"]) as usize,
            translate_system_messages: py::truthy(cfg.get("translate_system_messages")),
            target_language: py::display(
                cfg.get("target_language")
                    .unwrap_or(&defaults["target_language"]),
            ),
            overlay: Geometry {
                x: coord(&["overlay", "x"]),
                y: coord(&["overlay", "y"]),
                width: num(&["overlay", "width"]) as i32,
                height: num(&["overlay", "height"]) as i32,
            },
            bubble_position: (
                coord(&["bubble_position", "x"]),
                coord(&["bubble_position", "y"]),
            ),
            overlay_alpha: num(&["overlay_alpha"]) as f32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_dir_in_debug_is_the_project_root() {
        assert!(app_dir().join("pyproject.toml").exists());
    }

    #[test]
    fn local_state_dir_is_under_localappdata() {
        let dir = local_state_dir();
        assert!(dir.ends_with("wizard101-chat-translator"));
        if let Some(base) = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
            assert_eq!(dir.parent(), Some(Path::new(&base)));
        }
    }

    #[test]
    fn local_state_dir_falls_back_to_home_without_localappdata() {
        let home = std::env::home_dir()
            .unwrap()
            .join("wizard101-chat-translator");
        assert_eq!(state_dir_under(None), home);
        assert_eq!(state_dir_under(Some("".into())), home);
        assert_eq!(
            state_dir_under(Some(r"X:\Local".into())),
            Path::new(r"X:\Local\wizard101-chat-translator")
        );
    }

    #[test]
    fn default_region_hotkey_differs_from_the_input_hotkey() {
        let cfg = default_config();
        assert_eq!(cfg["region_hotkey"], "ctrl+shift+space");
        assert_ne!(cfg["region_hotkey"], cfg["hotkey"]);
        assert_eq!(cfg["ui_language"], Value::Null);
        assert_eq!(cfg["translate_system_messages"], false);
    }

    #[test]
    fn settings_read_defaults_and_overrides() {
        let mut cfg = default_config();
        cfg.insert(
            "overlay".into(),
            json!({"x": 10, "y": null, "width": "big"}),
        );
        cfg.insert("fade_seconds".into(), json!(5.5));
        let settings = Settings::from_config(&cfg);
        assert_eq!(settings.fade_seconds, 5);
        assert_eq!(settings.max_messages, 200);
        assert_eq!(settings.max_parallel_translations, 4);
        assert_eq!(
            settings.overlay,
            Geometry {
                x: Some(10),
                y: None,
                width: 640,
                height: 420
            }
        );
        assert_eq!(settings.bubble_position, (None, None));
        assert!((settings.overlay_alpha - 0.8).abs() < 1e-6);
    }

    #[test]
    fn clamp_keeps_python_int_and_float_types() {
        let defaults = default_config();
        let mut cfg = defaults.clone();
        cfg.insert("max_parallel_translations".into(), json!(99));
        cfg.insert("type_delay".into(), json!(0));
        cfg.insert("max_messages".into(), json!(10.0));
        cfg.insert("overlay_alpha".into(), json!(1));
        clamp_advanced(&mut cfg, &defaults);
        assert_eq!(cfg["max_parallel_translations"], json!(8));
        assert_eq!(cfg["type_delay"], json!(0.0));
        assert_eq!(cfg["max_messages"], json!(10));
        assert_eq!(cfg["overlay_alpha"], json!(1.0));
    }
}
