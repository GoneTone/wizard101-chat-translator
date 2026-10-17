//! 設定頁用的服務操作與進階數值解析（移植自 Python 的 test_services／test_window）。

use std::sync::{Mutex, MutexGuard};

use serde_json::{Value, json};
use wizard101_chat_translator::config::services::{
    EFFORTS, describe, is_auto_name, new_service, provider, providers, unique_name,
    validate_endpoint_fields, validate_service,
};
use wizard101_chat_translator::config::{AdvancedText, advanced_limits, parse_advanced};
use wizard101_chat_translator::i18n;

/// 介面語言是全域狀態，會翻文案的測試要排隊。
fn language_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn text(fade: &str, max: &str, parallel: &str, delay: &str, alpha: f64) -> AdvancedText {
    AdvancedText {
        fade_seconds: fade.into(),
        max_messages: max.into(),
        max_parallel_translations: parallel.into(),
        type_delay: delay.into(),
        overlay_alpha: alpha,
    }
}

fn valid() -> AdvancedText {
    text("0", "200", "4", "0.02", 0.84)
}

#[test]
fn every_provider_has_ui_metadata() {
    let keys: Vec<&str> = providers().iter().map(|p| p.key).collect();
    assert_eq!(keys, ["openai", "claude", "gemini", "custom"]);
    assert!(provider("custom").unwrap().needs_base_url());
    assert!(!provider("openai").unwrap().needs_base_url());
    assert!(provider("nope").is_none());
    assert_eq!(
        provider("claude").unwrap().key_url,
        Some("https://console.anthropic.com/settings/keys")
    );
    assert_eq!(provider("custom").unwrap().key_url, None);
    assert_eq!(provider("gemini").unwrap().label_key, "provider.gemini");
    assert_eq!(provider("gemini").unwrap().desc_key, "provider.gemini_desc");
    assert!(provider("claude").unwrap().has_field("effort"));
    assert!(!provider("openai").unwrap().has_field("base_url"));
    assert_eq!(EFFORTS, ["auto", "low"]);
}

#[test]
fn short_names_are_brands_except_the_custom_endpoint() {
    let _guard = language_lock();
    i18n::set_language("en-US");
    assert_eq!(provider("openai").unwrap().short_name(), "OpenAI");
    assert_eq!(provider("claude").unwrap().short_name(), "Claude");
    assert_ne!(
        provider("custom").unwrap().short_name(),
        "provider.custom_short"
    );
}

#[test]
fn every_provider_label_and_description_resolve() {
    let _guard = language_lock();
    i18n::set_language("zh-TW");
    for p in providers() {
        for key in [p.label_key, p.desc_key] {
            assert_ne!(i18n::t(key, &[]), key, "{key} has no zh-TW text");
        }
    }
}

#[test]
fn validate_service_reports_every_missing_required_field() {
    let errors = validate_service(&json!({"provider": "openai", "model": "", "api_key": ""}));
    assert_eq!(errors, ["error.need_model", "error.need_api_key"]);
}

#[test]
fn validate_endpoint_fields_ignores_the_model() {
    let custom = json!({"provider": "custom", "base_url": "http://x", "model": ""});
    assert!(validate_endpoint_fields(&custom).is_empty());
    let blank = json!({"provider": "custom", "base_url": " ", "model": ""});
    assert_eq!(validate_endpoint_fields(&blank), ["error.need_base_url"]);
}

#[test]
fn new_service_fills_the_provider_defaults_and_a_name() {
    let service = new_service("claude", &[]);
    assert_eq!(service["provider"], "claude");
    assert_eq!(service["name"], "Claude");
    assert_eq!(service["model"], "");
    assert_eq!(service["effort"], "auto");
    assert_eq!(service["id"].as_str().unwrap().len(), 8);
    assert_eq!(new_service("gemini", &[])["effort"], "low");
}

#[test]
fn new_services_never_share_an_id() {
    let mut services: Vec<Value> = Vec::new();
    for _ in 0..50 {
        let service = new_service("openai", &services);
        services.push(service);
    }
    let mut ids: Vec<&str> = services.iter().map(|s| s["id"].as_str().unwrap()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 50);
}

#[test]
fn unique_name_numbers_collisions_from_two() {
    let mut services = vec![json!({"id": "a", "name": "ChatGPT"})];
    assert_eq!(unique_name("ChatGPT", &services, None), "ChatGPT (2)");
    services.push(json!({"id": "b", "name": "ChatGPT (2)"}));
    assert_eq!(unique_name("ChatGPT", &services, None), "ChatGPT (3)");
    assert_eq!(unique_name("Claude", &services, None), "Claude");
}

#[test]
fn unique_name_does_not_collide_with_the_entry_being_edited() {
    let services = vec![json!({"id": "a", "name": "ChatGPT"})];
    assert_eq!(unique_name("ChatGPT", &services, Some("a")), "ChatGPT");
}

#[test]
fn is_auto_name_accepts_the_bare_base_and_numbered_suffixes() {
    assert!(is_auto_name("ChatGPT", "ChatGPT"));
    assert!(is_auto_name("ChatGPT (2)", "ChatGPT"));
    assert!(is_auto_name("ChatGPT (10)", "ChatGPT"));
}

#[test]
fn is_auto_name_rejects_shapes_unique_name_never_produces() {
    for name in [
        "ChatGPT 我的",
        "Claude",
        "ChatGPT ()",
        "ChatGPT (x)",
        "ChatGPT (2) extra",
        "ChatGPT (0)",
        "ChatGPT (1)",
        "ChatGPT (01)",
    ] {
        assert!(!is_auto_name(name, "ChatGPT"), "{name}");
    }
}

#[test]
fn is_auto_name_accepts_exactly_the_numbers_unique_name_can_produce() {
    let base = "ChatGPT";
    let mut services = vec![json!({"id": "seed", "name": base})];
    let mut generated = Vec::new();
    for n in 2..6 {
        let name = unique_name(base, &services, None);
        assert_eq!(name, format!("{base} ({n})"));
        generated.push(n);
        services.push(json!({"id": name, "name": name}));
    }
    let accepted: Vec<u32> = (0..6)
        .filter(|n| is_auto_name(&format!("{base} ({n})"), base))
        .collect();
    assert_eq!(accepted, generated);
}

#[test]
fn describe_shows_the_brand_and_the_model() {
    let mut service = new_service("claude", &[]);
    service["model"] = "claude-opus-5".into();
    assert_eq!(describe(&service), "Claude · claude-opus-5");
}

#[test]
fn parse_advanced_clamps_valid_numeric_input() {
    let values = parse_advanced(&text("-5", "99999", "4", "9.0", 2.0)).unwrap();
    assert_eq!(values["fade_seconds"], json!(0));
    assert_eq!(values["max_messages"], json!(1000));
    assert_eq!(values["type_delay"], json!(0.5));
    assert_eq!(values["overlay_alpha"], json!(1.0));
    assert_eq!(values["max_parallel_translations"], json!(4));
    assert_eq!(values.len(), 5);
}

#[test]
fn parse_advanced_clamps_parallel() {
    let values = parse_advanced(&text("0", "200", "99", "0.02", 0.84)).unwrap();
    assert_eq!(values["max_parallel_translations"], json!(8));
}

#[test]
fn parse_advanced_passes_valid_values_through() {
    let values = parse_advanced(&valid()).unwrap();
    assert_eq!(values["fade_seconds"], json!(0));
    assert_eq!(values["max_messages"], json!(200));
    assert_eq!(values["type_delay"], json!(0.02));
    assert_eq!(values["overlay_alpha"], json!(0.84));
}

#[test]
fn parse_advanced_rejects_non_numbers_and_non_finite() {
    for bad in [
        "abc", "", "   ", "NaN", "nan", "inf", "-inf", "Infinity", "1e999",
    ] {
        for field in 0..4 {
            let mut input = valid();
            match field {
                0 => input.fade_seconds = bad.into(),
                1 => input.max_messages = bad.into(),
                2 => input.max_parallel_translations = bad.into(),
                _ => input.type_delay = bad.into(),
            }
            assert_eq!(
                parse_advanced(&input),
                Err("error.advanced_not_number"),
                "{bad:?} in field {field}"
            );
        }
    }
    let mut input = valid();
    input.overlay_alpha = f64::NAN;
    assert_eq!(parse_advanced(&input), Err("error.advanced_not_number"));
}

#[test]
fn parse_advanced_follows_tk_intvar_for_integer_fields() {
    let values = parse_advanced(&text(" 1.9 ", "50.7", "-2.5", " 0.1 ", 0.5)).unwrap();
    assert_eq!(values["fade_seconds"], json!(1));
    assert_eq!(values["max_messages"], json!(50));
    assert_eq!(values["max_parallel_translations"], json!(1));
    assert_eq!(values["type_delay"], json!(0.1));
    let values = parse_advanced(&text("99999", "200", "4", "0.02", 0.84)).unwrap();
    assert_eq!(values["fade_seconds"], json!(3600));
}

#[test]
fn advanced_limits_report_the_clamp_ranges() {
    assert_eq!(advanced_limits("fade_seconds"), (0.0, 3600.0));
    assert_eq!(advanced_limits("overlay_alpha"), (0.3, 1.0));
}
