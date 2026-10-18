//! 介面文案資源：依當前介面語言提供字串，缺字串先退英文再退來源語言（繁中）。

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};

use crate::logs::log;

const META_NAME: &str = "language.name";
const META_TRANSLATORS: &str = "language.translators";
/// 文案來源語言：缺字串時的最後一層退路。
pub const SOURCE_LANGUAGE: &str = "zh-TW";
const DEFAULT_LANGUAGE: &str = "en-US";
const MINIMUM_COVERAGE: f64 = 0.6;

/// 編進 exe 的語言檔，依語言碼字母序。
const CATALOG_SOURCES: &[(&str, &str)] = &[
    ("af-ZA", include_str!("../../../src/i18n/af-ZA.json")),
    ("ar-SA", include_str!("../../../src/i18n/ar-SA.json")),
    ("ca-ES", include_str!("../../../src/i18n/ca-ES.json")),
    ("cs-CZ", include_str!("../../../src/i18n/cs-CZ.json")),
    ("da-DK", include_str!("../../../src/i18n/da-DK.json")),
    ("de-DE", include_str!("../../../src/i18n/de-DE.json")),
    ("el-GR", include_str!("../../../src/i18n/el-GR.json")),
    ("en-US", include_str!("../../../src/i18n/en-US.json")),
    ("es-ES", include_str!("../../../src/i18n/es-ES.json")),
    ("fi-FI", include_str!("../../../src/i18n/fi-FI.json")),
    ("fr-FR", include_str!("../../../src/i18n/fr-FR.json")),
    ("he-IL", include_str!("../../../src/i18n/he-IL.json")),
    ("hu-HU", include_str!("../../../src/i18n/hu-HU.json")),
    ("it-IT", include_str!("../../../src/i18n/it-IT.json")),
    ("ja-JP", include_str!("../../../src/i18n/ja-JP.json")),
    ("ko-KR", include_str!("../../../src/i18n/ko-KR.json")),
    ("nl-NL", include_str!("../../../src/i18n/nl-NL.json")),
    ("no-NO", include_str!("../../../src/i18n/no-NO.json")),
    ("pl-PL", include_str!("../../../src/i18n/pl-PL.json")),
    ("pt-BR", include_str!("../../../src/i18n/pt-BR.json")),
    ("pt-PT", include_str!("../../../src/i18n/pt-PT.json")),
    ("ro-RO", include_str!("../../../src/i18n/ro-RO.json")),
    ("ru-RU", include_str!("../../../src/i18n/ru-RU.json")),
    ("sr-SP", include_str!("../../../src/i18n/sr-SP.json")),
    ("sv-SE", include_str!("../../../src/i18n/sv-SE.json")),
    ("tr-TR", include_str!("../../../src/i18n/tr-TR.json")),
    ("uk-UA", include_str!("../../../src/i18n/uk-UA.json")),
    ("vi-VN", include_str!("../../../src/i18n/vi-VN.json")),
    ("zh-CN", include_str!("../../../src/i18n/zh-CN.json")),
    ("zh-TW", include_str!("../../../src/i18n/zh-TW.json")),
];

type Catalog = HashMap<String, String>;

struct Strict(Catalog);

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Strict;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a flat string-to-string object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Strict, A::Error> {
                let mut out = Catalog::new();
                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    if out.contains_key(&key) {
                        return Err(serde::de::Error::custom(format!("duplicate key: {key}")));
                    }
                    out.insert(key, value);
                }
                Ok(Strict(out))
            }
        }
        deserializer.deserialize_map(V)
    }
}

/// 解析一份扁平 key-value 語言檔；重複 key 直接拒絕而非靜默覆蓋。
pub fn parse_catalog(text: &str) -> Result<HashMap<String, String>, String> {
    serde_json::from_str::<Strict>(text)
        .map(|s| s.0)
        .map_err(|e| e.to_string())
}

fn catalogs() -> &'static HashMap<&'static str, Catalog> {
    static CATALOGS: OnceLock<HashMap<&'static str, Catalog>> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        CATALOG_SOURCES
            .iter()
            .map(|(code, text)| {
                let catalog = parse_catalog(text).unwrap_or_else(|error| {
                    log(format!("[i18n] catalog unreadable: {code} error={error}"));
                    Catalog::new()
                });
                (*code, catalog)
            })
            .collect()
    })
}

fn catalog(code: &str) -> Option<&'static Catalog> {
    catalogs().get(code)
}

fn meta(code: &str, key: &str) -> String {
    catalog(code)
        .and_then(|c| c.get(key))
        .cloned()
        .unwrap_or_default()
}

fn coverage(catalog: &Catalog, source: &Catalog) -> f64 {
    if source.is_empty() {
        return 1.0;
    }
    let have = source.keys().filter(|k| catalog.contains_key(*k)).count();
    have as f64 / source.len() as f64
}

/// 從語言檔挑出語言選單要列的語言：來源與預設語言一律列，其餘須宣告自稱且完成度達門檻。
fn offered_languages(catalogs: &[(&str, &Catalog)]) -> Vec<(String, String)> {
    let empty = Catalog::new();
    let source = catalogs
        .iter()
        .find(|(code, _)| *code == SOURCE_LANGUAGE)
        .map_or(&empty, |(_, catalog)| *catalog);
    let mut found = Vec::new();
    for (code, catalog) in catalogs {
        let name = catalog.get(META_NAME).cloned().unwrap_or_default();
        if *code != SOURCE_LANGUAGE && *code != DEFAULT_LANGUAGE {
            let coverage = coverage(catalog, source) * 100.0;
            if name.is_empty() {
                log(format!(
                    "[i18n] catalog without {META_NAME}, not offered: {code} coverage={coverage:.0}%"
                ));
                continue;
            }
            if coverage < MINIMUM_COVERAGE * 100.0 {
                log(format!(
                    "[i18n] catalog below {:.0}%, not offered: {code} coverage={coverage:.0}%",
                    MINIMUM_COVERAGE * 100.0
                ));
                continue;
            }
        }
        let name = if name.is_empty() {
            (*code).to_string()
        } else {
            name
        };
        found.push(((*code).to_string(), name));
    }
    found
}

/// 可選的介面語言：（語言碼，自稱），依語言碼字母序；完成度與自稱門檻同 Python。
pub fn available_languages() -> Vec<(String, String)> {
    static LANGUAGES: OnceLock<Vec<(String, String)>> = OnceLock::new();
    LANGUAGES
        .get_or_init(|| {
            let all: Vec<(&str, &Catalog)> = CATALOG_SOURCES
                .iter()
                .filter_map(|(code, _)| catalog(code).map(|c| (*code, c)))
                .collect();
            let found = offered_languages(&all);
            let codes: Vec<&str> = found.iter().map(|(c, _)| c.as_str()).collect();
            log(format!("[i18n] catalogs offered: {}", codes.join(",")));
            found
        })
        .clone()
}

/// 語言的自稱；語言檔沒宣告就回語言碼本身。
pub fn language_name(code: &str) -> String {
    available_languages()
        .into_iter()
        .find(|(c, _)| c == code)
        .map(|(_, name)| name)
        .unwrap_or_else(|| code.to_string())
}

/// 這份譯文的譯者掛名；沒宣告回空字串。
pub fn translators(code: &str) -> String {
    meta(code, META_TRANSLATORS)
}

fn current() -> &'static Mutex<String> {
    static CURRENT: OnceLock<Mutex<String>> = OnceLock::new();
    CURRENT.get_or_init(|| Mutex::new(DEFAULT_LANGUAGE.to_string()))
}

/// 目前的介面語言碼。
pub fn current_language() -> String {
    current().lock().unwrap().clone()
}

/// 切換介面語言；語言碼不認得時退回預設語言。
pub fn set_language(code: &str) {
    let code = if available_languages().iter().any(|(c, _)| c == code) {
        code
    } else {
        log(format!(
            "[i18n] unknown language: {code}, using {DEFAULT_LANGUAGE}"
        ));
        DEFAULT_LANGUAGE
    };
    *current().lock().unwrap() = code.to_string();
    log(format!("[i18n] language set: {code}"));
}

/// 代入 `{name}` 具名變數並處理 `{{`／`}}` 跳脫；變數缺漏或括號不成對回 Err。
fn format_template(template: &str, args: &[(&str, &str)]) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(ch) => name.push(ch),
                        None => return Err("unclosed placeholder".into()),
                    }
                }
                match args.iter().find(|(k, _)| *k == name) {
                    Some((_, value)) => out.push_str(value),
                    None => return Err(format!("missing argument: {name}")),
                }
            }
            '}' => return Err("single '}' in template".into()),
            _ => out.push(c),
        }
    }
    Ok(out)
}

/// 查詢文案的語言順序：目前語言 → 預設語言 → 來源語言（去重）。
fn fallback_order(current: &str) -> Vec<&str> {
    let mut order = vec![current];
    for code in [DEFAULT_LANGUAGE, SOURCE_LANGUAGE] {
        if !order.contains(&code) {
            order.push(code);
        }
    }
    order
}

/// 依 `order` 逐語言查 `key` 並代入變數；缺字串或格式失敗往下一層退，全失敗回 key。
fn translate<'a>(
    order: &[&str],
    lookup: impl Fn(&str) -> Option<&'a Catalog>,
    key: &str,
    args: &[(&str, &str)],
) -> String {
    for code in order {
        let Some(template) = lookup(code).and_then(|c| c.get(key)) else {
            log(format!("[i18n] missing key: {key} lang={code}"));
            continue;
        };
        match format_template(template, args) {
            Ok(text) => return text,
            Err(error) => log(format!(
                "[i18n] format failed: key={key} lang={code} error={error}"
            )),
        }
    }
    key.to_string()
}

/// 取當前語言的文案並代入變數；缺字串或格式失敗往下一層語言退，全失敗回 key。
pub fn t(key: &str, args: &[(&str, &str)]) -> String {
    let current = current_language();
    translate(&fallback_order(&current), catalog, key, args)
}

/// 中文標籤的字體系：`Some(true)` 繁體、`Some(false)` 簡體、非中文 `None`。
pub(crate) fn chinese_is_traditional(tag: &str) -> Option<bool> {
    let lower = tag.to_ascii_lowercase().replace('_', "-");
    let parts: Vec<&str> = lower.split('-').collect();
    if !matches!(parts[0], "zh" | "cmn" | "yue" | "wuu") {
        return None;
    }
    let script = parts.iter().skip(1).find(|p| p.len() == 4);
    Some(match script {
        Some(&s) => s == "hant",
        None => {
            parts.contains(&"yue")
                || parts
                    .iter()
                    .skip(1)
                    .any(|p| matches!(*p, "tw" | "hk" | "mo"))
        }
    })
}

fn match_tag(tag: &str, codes: &[String]) -> Option<String> {
    if let Some(traditional) = chinese_is_traditional(tag) {
        let region = if traditional { "-TW" } else { "-CN" };
        return codes
            .iter()
            .find(|c| c.starts_with("zh") && c.ends_with(region))
            .cloned();
    }
    let lower = tag.to_ascii_lowercase().replace('_', "-");
    let lang = lower.split('-').next().unwrap_or("");
    codes
        .iter()
        .find(|c| {
            c.split('-')
                .next()
                .is_some_and(|l| l.eq_ignore_ascii_case(lang))
        })
        .cloned()
}

/// 偏好語言標籤（依偏好排序）→ 最接近的介面語言碼，都不夠近則回預設語言。
///
/// 不移植 CLDR 距離資料：中文依字體系與地區、其餘依主語言比對。
pub fn best_match(tags: &[String]) -> String {
    let codes: Vec<String> = available_languages().into_iter().map(|(c, _)| c).collect();
    for tag in tags {
        if tag.is_empty() {
            log(format!("[i18n] unparsable language tag: {tag:?}"));
            continue;
        }
        match match_tag(tag, &codes) {
            Some(code) => {
                log(format!("[i18n] language tag matched: {tag} -> {code}"));
                return code;
            }
            None => log(format!("[i18n] no catalog close enough: {tag}")),
        }
    }
    DEFAULT_LANGUAGE.to_string()
}

fn preferred_ui_languages() -> std::io::Result<Vec<String>> {
    use windows_sys::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
    let mut count = 0u32;
    let mut size = 0u32;
    // SAFETY: 兩段式呼叫，第一次只取所需長度，第二次的緩衝區依該長度配置。
    let buffer = unsafe {
        let null = std::ptr::null_mut();
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, null, &mut size) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut buffer = vec![0u16; size as usize];
        let ptr = buffer.as_mut_ptr();
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, ptr, &mut size) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        buffer.truncate(size as usize);
        buffer
    };
    Ok(buffer
        .split(|&u| u == 0)
        .filter(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect())
}

/// 偵測 Windows 使用者介面語言；任何失敗都退預設語言並留下 log。
pub fn detect_system_language() -> String {
    let tags = match preferred_ui_languages() {
        Ok(tags) => tags,
        Err(error) => {
            log(format!("[i18n] system language detection failed: {error}"));
            return DEFAULT_LANGUAGE.to_string();
        }
    };
    let code = best_match(&tags);
    let preferred = if tags.is_empty() {
        "(none)".to_string()
    } else {
        tags.join(",")
    };
    log(format!(
        "[i18n] system language detected: preferred={preferred} -> {code}"
    ));
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_catalog_parses_without_duplicates() {
        for (code, text) in CATALOG_SOURCES {
            parse_catalog(text).unwrap_or_else(|e| panic!("{code}: {e}"));
        }
    }

    fn catalog_of(pairs: &[(&str, &str)]) -> Catalog {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn embedded(code: &str) -> &'static Catalog {
        catalog(code).unwrap_or_else(|| panic!("no embedded catalog: {code}"))
    }

    fn placeholders(text: &str) -> std::collections::BTreeSet<String> {
        let mut names = std::collections::BTreeSet::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '{' {
                continue;
            }
            if chars.peek() == Some(&'{') {
                chars.next();
                continue;
            }
            let name: String = chars.by_ref().take_while(|&ch| ch != '}').collect();
            names.insert(name);
        }
        names
    }

    #[test]
    fn no_language_file_has_keys_the_source_language_lacks() {
        let source = embedded(SOURCE_LANGUAGE);
        for (code, _) in available_languages() {
            let extra: Vec<&String> = embedded(&code)
                .keys()
                .filter(|k| !source.contains_key(*k))
                .collect();
            assert!(
                extra.is_empty(),
                "{code} has keys the source lacks: {extra:?}"
            );
        }
    }

    #[test]
    fn the_hand_maintained_catalogs_carry_the_same_keys() {
        let source = embedded(SOURCE_LANGUAGE);
        for code in [SOURCE_LANGUAGE, DEFAULT_LANGUAGE, "zh-CN"] {
            let mut missing: Vec<&String> = source
                .keys()
                .filter(|k| !embedded(code).contains_key(*k))
                .collect();
            let mut extra: Vec<&String> = embedded(code)
                .keys()
                .filter(|k| !source.contains_key(*k))
                .collect();
            missing.sort();
            extra.sort();
            assert!(
                missing.is_empty() && extra.is_empty(),
                "{code} differs from {SOURCE_LANGUAGE}: missing {missing:?}, extra {extra:?}"
            );
        }
    }

    #[test]
    fn placeholders_match_across_languages() {
        let source = embedded(SOURCE_LANGUAGE);
        for (code, _) in available_languages() {
            for (key, template) in source {
                let Some(translated) = embedded(&code).get(key) else {
                    continue;
                };
                assert_eq!(
                    placeholders(translated),
                    placeholders(template),
                    "{code} {key}: placeholders differ from the source language"
                );
            }
        }
    }

    #[test]
    fn zh_cn_keeps_its_own_wording_for_chat_and_advanced() {
        let zh_cn = embedded("zh-CN");
        assert_eq!(zh_cn["slot.incoming"], "聊天接收");
        assert!(zh_cn["service.slots_title"].contains("高级"));
    }

    #[test]
    fn the_zh_cn_readme_speaks_the_same_words_as_the_catalog() {
        let readme = include_str!("../../../README_ZH-CN.md");
        assert!(readme.contains(embedded("zh-CN")["slot.incoming"].as_str()));
    }

    #[test]
    fn fallback_order_is_current_then_english_then_source() {
        assert_eq!(fallback_order("zh-CN"), ["zh-CN", "en-US", "zh-TW"]);
        assert_eq!(fallback_order("en-US"), ["en-US", "zh-TW"]);
        assert_eq!(fallback_order("zh-TW"), ["zh-TW", "en-US"]);
    }

    fn lookup_in<'a>(
        catalogs: &'a HashMap<&'static str, Catalog>,
    ) -> impl Fn(&str) -> Option<&'a Catalog> {
        |code| catalogs.get(code)
    }

    #[test]
    fn t_formats_named_placeholders() {
        let catalogs = HashMap::from([("zh-TW", catalog_of(&[("test.greet", "你好 {name}")]))]);
        let text = translate(
            &["zh-TW"],
            lookup_in(&catalogs),
            "test.greet",
            &[("name", "Amy")],
        );
        assert_eq!(text, "你好 Amy");
    }

    #[test]
    fn missing_key_falls_back_to_english_before_source() {
        let catalogs = HashMap::from([
            ("zh-CN", catalog_of(&[])),
            ("en-US", catalog_of(&[("test.partial", "English copy")])),
            ("zh-TW", catalog_of(&[("test.partial", "繁中文案")])),
        ]);
        let order = fallback_order("zh-CN");
        assert_eq!(
            translate(&order, lookup_in(&catalogs), "test.partial", &[]),
            "English copy"
        );
    }

    #[test]
    fn missing_key_falls_back_to_source_language_last() {
        let catalogs = HashMap::from([
            ("zh-CN", catalog_of(&[])),
            ("en-US", catalog_of(&[])),
            (
                "zh-TW",
                catalog_of(&[("test.only_source", "只有來源語言有")]),
            ),
        ]);
        let order = fallback_order("zh-CN");
        assert_eq!(
            translate(&order, lookup_in(&catalogs), "test.only_source", &[]),
            "只有來源語言有"
        );
    }

    #[test]
    fn missing_everywhere_returns_the_key_itself() {
        let catalogs = HashMap::from([("en-US", catalog_of(&[]))]);
        let order = fallback_order("en-US");
        assert_eq!(
            translate(&order, lookup_in(&catalogs), "test.nowhere", &[]),
            "test.nowhere"
        );
    }

    #[test]
    fn broken_placeholder_falls_back_to_the_next_language() {
        let catalogs = HashMap::from([
            ("zh-CN", catalog_of(&[("test.count", "共 {conut} 条")])),
            ("en-US", catalog_of(&[("test.count", "{count} in total")])),
        ]);
        let order = fallback_order("zh-CN");
        assert_eq!(
            translate(
                &order,
                lookup_in(&catalogs),
                "test.count",
                &[("count", "3")]
            ),
            "3 in total"
        );
    }

    #[test]
    fn system_preferences_are_valid_language_tags() {
        let tags = preferred_ui_languages().unwrap();
        assert!(!tags.is_empty(), "the system always has a UI language");
        for tag in &tags {
            assert!(
                !tag.is_empty() && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "not a BCP-47 style tag: {tag:?}"
            );
        }
        assert!(
            available_languages()
                .iter()
                .any(|(c, _)| *c == detect_system_language())
        );
    }

    #[test]
    fn a_language_without_an_endonym_is_not_offered() {
        let en = catalog_of(&[("language.name", "English")]);
        let ja = catalog_of(&[("app.name", "ウィザード")]);
        let offered = offered_languages(&[("en-US", &en), ("ja-JP", &ja)]);
        assert_eq!(offered, [("en-US".to_string(), "English".to_string())]);
    }

    #[test]
    fn a_barely_translated_language_is_not_offered() {
        let mut source = catalog_of(&[("language.name", "繁體中文（台灣）")]);
        let mut korean = catalog_of(&[("language.name", "한국어")]);
        for n in 0..9 {
            source.insert(format!("key.{n}"), "來源".into());
        }
        for n in 0..8 {
            korean.insert(format!("key.{n}"), "번역".into());
        }
        let japanese = catalog_of(&[
            ("language.name", "日本語"),
            ("key.0", "ひとつ"),
            ("key.1", "ふたつ"),
        ]);
        let offered =
            offered_languages(&[("ja-JP", &japanese), ("ko-KR", &korean), ("zh-TW", &source)]);
        let codes: Vec<&str> = offered.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(codes, ["ko-KR", "zh-TW"]);
        assert_eq!(offered[0].1, "한국어");
    }

    #[test]
    fn the_source_and_default_languages_are_always_offered() {
        let source: Catalog = (0..10)
            .map(|n| (format!("key.{n}"), "來源".into()))
            .collect();
        let default = Catalog::new();
        let offered = offered_languages(&[("en-US", &default), ("zh-TW", &source)]);
        let codes: Vec<&str> = offered.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(codes, ["en-US", "zh-TW"]);
    }

    #[test]
    fn translators_are_never_borrowed_from_another_language() {
        for (code, _) in CATALOG_SOURCES {
            let own = embedded(code).get(META_TRANSLATORS).cloned();
            assert_eq!(translators(code), own.unwrap_or_default(), "{code}");
        }
        assert_eq!(translators("xx-XX"), "");
    }

    #[test]
    fn format_handles_escapes_and_missing_args() {
        assert_eq!(
            format_template("a {{x}} {n}", &[("n", "1")]).unwrap(),
            "a {x} 1"
        );
        assert!(format_template("{n}", &[]).is_err());
        assert!(format_template("{n", &[("n", "1")]).is_err());
    }
}
