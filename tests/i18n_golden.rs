//! i18n 語言挑選、語言清單與字型選擇（部分對照 Python 匯出的 golden fixture）。
//!
//! 會切換全域介面語言，獨立成一個測試執行檔，檔內以鎖排隊。

mod common;

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

#[test]
fn app_name_follows_language() {
    let _language = common::language_lock();
    i18n::set_language("zh-TW");
    let (zh, zh_key) = (app_name(), i18n::t("app.name", &[]));
    i18n::set_language("en-US");
    let (en, en_key) = (app_name(), i18n::t("app.name", &[]));
    assert_eq!(zh, zh_key);
    assert_eq!(en, en_key);
    assert_ne!(zh, en);
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
    let _language = common::language_lock();
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
        Some(i18n::language_name(i18n::SOURCE_LANGUAGE).as_str())
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
