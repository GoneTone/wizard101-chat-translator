//! config 載入、遷移與服務對照 Python 匯出的 golden fixture。

mod common;

use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use wizard101_chat_translator::config::{
    self, Config, ServiceSlot, bootstrap_language_with, config_summary, load_config, save_config,
    services,
};

fn hex_decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 同 exporter 的 `_canon_ids`：`source` 裡沒有的服務 id 依出現順序換成 `@idN`。
fn canon_ids(cfg: &Config, source: &str, texts: &[String]) -> Vec<String> {
    let mut ids: Vec<&str> = Vec::new();
    for service in cfg
        .get("services")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(id) = service.get("id").and_then(Value::as_str)
            && !source.contains(id)
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    texts
        .iter()
        .map(|text| {
            ids.iter().enumerate().fold(text.clone(), |t, (n, id)| {
                t.replace(id, &format!("@id{}", n + 1))
            })
        })
        .collect()
}

fn as_config(value: &Value) -> Config {
    value.as_object().unwrap().clone()
}

/// 比對時連鍵的順序一起比（serde_json 的 Map 相等不看順序），數字依 Python 的值與型別比。
fn ordered(value: &Value) -> String {
    fn python_numbers(value: &Value) -> Value {
        match value {
            Value::Number(n) if n.as_str().contains(['.', 'e', 'E']) => {
                Value::String(format!("float:{:?}", n.as_str().parse::<f64>().unwrap()))
            }
            Value::Number(n) if n.as_str() == "-0" => Value::String("int:0".into()),
            Value::Number(n) => Value::String(format!("int:{n}")),
            Value::Array(items) => Value::Array(items.iter().map(python_numbers).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), python_numbers(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    serde_json::to_string(&python_numbers(value)).unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("w101-config-{}-{name}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn load_matches_python() {
    let dir = temp_dir("load");
    for (i, case) in common::golden("config_load").iter().enumerate() {
        let raw = hex_decode(case["input"].as_str().unwrap());
        let path = dir.join(format!("{i}.json"));
        fs::write(&path, &raw).unwrap();
        let cfg = load_config(&path);
        let after = fs::read(&path).unwrap();
        let source = String::from_utf8_lossy(&raw);
        let texts = canon_ids(
            &cfg,
            &source,
            &[
                serde_json::to_string(&cfg).unwrap(),
                String::from_utf8_lossy(&after).into_owned(),
            ],
        );
        let expected = &case["output"];
        let actual: Value = serde_json::from_str(&texts[0]).unwrap();
        assert_eq!(
            ordered(&actual),
            ordered(&expected["config"]),
            "case {i}: {source}"
        );
        let written = (after != raw).then(|| hex_encode(texts[1].as_bytes()));
        assert_eq!(
            written.as_deref(),
            expected["written"].as_str(),
            "case {i}: {source}"
        );
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn corrupt_file_is_never_overwritten() {
    let dir = temp_dir("corrupt");
    let defaults = load_config(&dir.join("missing.json"));
    let corrupt: &[&[u8]] = &[
        b"",
        b"{\"hotkey\": \"f8\",}",
        b"\xff\xfe{}",
        b"\xef\xbb\xbf{\"hotkey\": \"f8\"}",
        b"[1, 2]",
        b"null",
        b"{\"api\": {\"provider\": \"openai\", \"openai\": {\"api_key\": \"SK-1\"}}",
    ];
    for (i, raw) in corrupt.iter().enumerate() {
        let path = dir.join(format!("{i}.json"));
        fs::write(&path, raw).unwrap();
        assert_eq!(
            ordered(&Value::Object(load_config(&path))),
            ordered(&Value::Object(defaults.clone()))
        );
        assert_eq!(&fs::read(&path).unwrap(), raw, "case {i}");
    }
    // 讀不到（路徑是目錄）也退回預設值。
    assert_eq!(load_config(&dir), defaults);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn read_only_file_still_loads_after_migration() {
    let dir = temp_dir("readonly");
    let path = dir.join("config.json");
    let raw = br#"{"api": {"provider": "custom", "base_url": "http://x", "model": "m"}}"#;
    fs::write(&path, raw).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&path, perms.clone()).unwrap();
    let cfg = load_config(&path);
    assert_eq!(cfg["services"][0]["base_url"], "http://x");
    assert_eq!(fs::read(&path).unwrap(), raw);
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(&path, perms).unwrap();
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn save_bytes_match_python() {
    let dir = temp_dir("save");
    let path = dir.join("config.json");
    let mut checked = 0;
    for case in common::golden("config_load") {
        let Some(written) = case["output"]["written"].as_str() else {
            continue;
        };
        save_config(&path, &as_config(&case["output"]["config"])).unwrap();
        assert_eq!(hex_encode(&fs::read(&path).unwrap()), written);
        checked += 1;
    }
    assert!(checked > 10);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn normalize_matches_python() {
    for case in common::golden("config_normalize") {
        let mut cfg = as_config(&case["input"]);
        let source = serde_json::to_string(&case["input"]).unwrap();
        let changed = services::normalize(&mut cfg);
        let again = services::normalize(&mut cfg);
        let text = canon_ids(&cfg, &source, &[serde_json::to_string(&cfg).unwrap()]);
        let actual: Value = serde_json::from_str(&text[0]).unwrap();
        let expected = &case["output"];
        assert_eq!(changed, expected["changed"].as_bool().unwrap(), "{source}");
        assert_eq!(
            again,
            expected["changed_again"].as_bool().unwrap(),
            "{source}"
        );
        assert_eq!(ordered(&actual), ordered(&expected["cfg"]), "{source}");
    }
}

#[test]
fn resolve_matches_python() {
    for case in common::golden("config_resolve") {
        let cfg = as_config(&case["input"]["cfg"]);
        let slot = match case["input"]["slot"].as_str().unwrap() {
            "incoming" => ServiceSlot::Incoming,
            "outgoing" => ServiceSlot::Outgoing,
            _ => ServiceSlot::Region,
        };
        let spec = services::resolve(&cfg, slot);
        let out = &case["output"];
        let text = |k: &str| out.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let expected = services::ServiceSpec {
            id: text("id"),
            name: text("name"),
            provider: text("provider"),
            model: text("model"),
            api_key: text("api_key"),
            base_url: text("base_url"),
            thinking: out
                .get("thinking")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            effort: text("effort"),
        };
        assert_eq!(spec, expected, "{}", case["input"]);
    }
}

#[test]
fn validate_matches_python() {
    for case in common::golden("config_validate") {
        let expected: Vec<String> = serde_json::from_value(case["output"].clone()).unwrap();
        assert_eq!(
            services::validate_service(&case["input"]),
            expected,
            "{}",
            case["input"]
        );
    }
}

#[test]
fn summary_matches_python() {
    for case in common::golden("config_summary") {
        let cfg = as_config(&case["input"]);
        let summary = config_summary(&cfg);
        assert_eq!(summary, case["output"].as_str().unwrap());
        for service in cfg["services"].as_array().unwrap() {
            let key = service["api_key"].as_str().unwrap();
            assert!(
                key.is_empty() || !summary.contains(key),
                "summary leaks {key}"
            );
        }
    }
}

#[test]
fn bootstrap_matches_python() {
    for case in common::golden("config_bootstrap_language") {
        let input = &case["input"];
        let mut cfg = Config::new();
        cfg.insert("ui_language".into(), input["ui_language"].clone());
        cfg.insert("target_language".into(), "Original".into());
        let mut called = false;
        let code =
            bootstrap_language_with(&mut cfg, input["config_existed"].as_bool().unwrap(), || {
                called = true;
                input["detected"].as_str().unwrap().to_string()
            });
        let out = &case["output"];
        assert_eq!(code, out["code"].as_str().unwrap(), "{input}");
        assert_eq!(cfg["target_language"], out["target_language"], "{input}");
        assert_eq!(called, out["detect_called"].as_bool().unwrap(), "{input}");
    }
}

#[test]
fn is_configured_needs_a_complete_default_service() {
    let cfg = |service: Value| -> Config {
        let mut cfg = Config::new();
        cfg.insert("services".into(), Value::Array(vec![service]));
        cfg.insert("default_service".into(), "s1".into());
        cfg
    };
    let custom = serde_json::json!({"id": "s1", "provider": "custom", "base_url": "http://x", "model": "m", "api_key": ""});
    assert!(config::is_configured(&cfg(custom.clone())));
    let mut no_url = custom;
    no_url["base_url"] = "".into();
    assert!(!config::is_configured(&cfg(no_url)));
    let openai = serde_json::json!({"id": "s1", "provider": "openai", "model": "m", "api_key": ""});
    assert!(!config::is_configured(&cfg(openai)));
    assert!(!config::is_configured(&Config::new()));
}

/// 手動對照：`W101_LOAD_CONFIG` 指向一份真實 config.json 的副本，以 Rust 載入它；設了 `W101_SAVE_CONFIG` 就再寫回。
#[test]
#[ignore = "runs the loader on a file named by W101_LOAD_CONFIG; used for manual byte-compares"]
fn load_config_from_env_path() {
    let path = PathBuf::from(std::env::var_os("W101_LOAD_CONFIG").expect("W101_LOAD_CONFIG"));
    let cfg = load_config(&path);
    if std::env::var_os("W101_SAVE_CONFIG").is_some() {
        save_config(&path, &cfg).unwrap();
    }
}
