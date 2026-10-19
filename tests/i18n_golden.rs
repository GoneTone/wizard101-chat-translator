//! i18n 與字型選擇對照 Python 匯出的 golden fixture。
//!
//! 會切換全域介面語言，獨立成一個測試執行檔，檔內以鎖排隊。

mod common;

use std::sync::Mutex;

use wizard101_chat_translator::config::app_name;
use wizard101_chat_translator::i18n;
use wizard101_chat_translator::ui::fonts::font_family;

#[test]
fn best_match_matches_python() {
    for case in common::golden("i18n_best_match") {
        let tags: Vec<String> = case["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            i18n::best_match(&tags),
            case["output"].as_str().unwrap(),
            "{tags:?}"
        );
    }
}

#[test]
fn available_languages_match_python() {
    let case = &common::golden("i18n_available")[0];
    let expected: Vec<(String, String)> = case["output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p[0].as_str().unwrap().into(), p[1].as_str().unwrap().into()))
        .collect();
    assert_eq!(i18n::available_languages(), expected);
}

/// 全域介面語言只有一份：本檔切換語言的測試互相排隊（lib 單元測試一律不切換）。
static LANGUAGE: Mutex<()> = Mutex::new(());

#[test]
fn t_matches_python() {
    let _language = LANGUAGE.lock().unwrap_or_else(|e| e.into_inner());
    for case in common::golden("i18n_t") {
        let input = &case["input"];
        let lang = input["lang"].as_str().unwrap();
        let key = input["key"].as_str().unwrap();
        let args: Vec<(&str, &str)> = input["args"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str().unwrap()))
            .collect();
        i18n::set_language(lang);
        assert_eq!(
            i18n::t(key, &args),
            case["output"].as_str().unwrap(),
            "{lang} {key} {args:?}"
        );
    }
}

#[test]
fn app_name_follows_language() {
    let _language = LANGUAGE.lock().unwrap_or_else(|e| e.into_inner());
    i18n::set_language("zh-TW");
    let zh = app_name();
    i18n::set_language("en-US");
    assert_eq!(zh, "Wizard101 對話翻譯助手");
    assert_eq!(app_name(), "Wizard101 Chat Translator");
}

#[test]
fn font_family_matches_python() {
    for case in common::golden("fonts_family") {
        let tag = case["input"].as_str().unwrap();
        assert_eq!(font_family(tag), case["output"].as_str().unwrap(), "{tag}");
    }
}

#[test]
fn duplicate_keys_are_rejected() {
    let error = i18n::parse_catalog(r#"{"a": "1", "a": "2"}"#).unwrap_err();
    assert!(error.contains("duplicate key: a"), "{error}");
    assert!(i18n::parse_catalog(r#"{"a": "1", "b": "2"}"#).is_ok());
}

#[test]
fn system_language_detection_returns_an_offered_code() {
    let code = i18n::detect_system_language();
    assert!(i18n::available_languages().iter().any(|(c, _)| *c == code));
}

#[test]
fn unknown_language_code_falls_back_to_default() {
    let _language = LANGUAGE.lock().unwrap_or_else(|e| e.into_inner());
    i18n::set_language("xx-XX");
    assert_eq!(i18n::current_language(), "en-US");
    i18n::set_language("zh-TW");
    i18n::set_language("xx-XX");
    assert_eq!(i18n::current_language(), "en-US");
}

#[test]
fn languages_are_listed_as_endonyms() {
    let languages = i18n::available_languages();
    let codes: Vec<&String> = languages.iter().map(|(c, _)| c).collect();
    let mut sorted = codes.clone();
    sorted.sort();
    assert_eq!(codes, sorted);
    let source = languages.iter().find(|(c, _)| c == i18n::SOURCE_LANGUAGE);
    assert_eq!(
        source.map(|(_, name)| name.as_str()),
        Some("繁體中文（台灣）")
    );
}

#[test]
fn language_names_are_unique() {
    let names: Vec<String> = i18n::available_languages()
        .into_iter()
        .map(|(_, name)| name)
        .collect();
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(names.len(), unique.len(), "duplicate endonyms: {names:?}");
}
