//! 按鍵名稱與快捷鍵解析對照 Python `keyboard` 匯出的 golden fixture。

mod common;

use serde_json::{Value, json};
use wizard101_chat_translator::input::keys::{
    is_modifier, key_codes_or_empty, key_to_scan_codes, parse_hotkey, scan_code_to_vk,
};

fn or_error<T: serde::Serialize>(result: Result<T, String>) -> Value {
    result.map_or_else(|_| json!({"error": true}), |v| json!(v))
}

#[test]
fn key_to_scan_codes_matches_python() {
    let cases = common::golden("keyboard_names");
    assert!(cases.len() > 1000, "only {} cases", cases.len());
    for c in cases {
        let name = c["input"].as_str().unwrap();
        assert_eq!(or_error(key_to_scan_codes(name)), c["output"], "{name:?}");
    }
}

#[test]
fn parse_hotkey_matches_python() {
    for c in common::golden("keyboard_parse") {
        let hotkey = c["input"].as_str().unwrap();
        assert_eq!(or_error(parse_hotkey(hotkey)), c["output"], "{hotkey:?}");
    }
}

#[test]
fn is_modifier_matches_python() {
    let cases = common::golden("keyboard_is_modifier");
    assert!(
        cases.iter().any(|c| c["output"] == true),
        "no modifier codes"
    );
    for c in cases {
        let code = c["input"].as_i64().unwrap() as i32;
        assert_eq!(json!(is_modifier(code)), c["output"], "{code}");
    }
}

#[test]
fn missing_key_error_matches_python() {
    assert_eq!(
        key_to_scan_codes("foo"),
        Err("Key 'foo' is not mapped to any known key.".to_string())
    );
    assert!(key_codes_or_empty("foo").is_empty());
}

#[test]
fn scan_code_to_vk_maps_negative_codes_and_letters() {
    assert_eq!(scan_code_to_vk(-0x41), 0x41);
    assert_eq!(scan_code_to_vk(key_to_scan_codes("a").unwrap()[0]), 0x41);
}
