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

fn coverage(code: &str, source: &Catalog) -> f64 {
    if source.is_empty() {
        return 1.0;
    }
    let have = catalog(code).map_or(0, |c| source.keys().filter(|k| c.contains_key(*k)).count());
    have as f64 / source.len() as f64
}

/// 可選的介面語言：（語言碼，自稱），依語言碼字母序；完成度與自稱門檻同 Python。
pub fn available_languages() -> Vec<(String, String)> {
    static LANGUAGES: OnceLock<Vec<(String, String)>> = OnceLock::new();
    LANGUAGES
        .get_or_init(|| {
            let source = catalog(SOURCE_LANGUAGE).cloned().unwrap_or_default();
            let mut found = Vec::new();
            for (code, _) in CATALOG_SOURCES {
                let name = meta(code, META_NAME);
                if *code != SOURCE_LANGUAGE && *code != DEFAULT_LANGUAGE {
                    let coverage = coverage(code, &source) * 100.0;
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
                let name = if name.is_empty() { (*code).to_string() } else { name };
                found.push(((*code).to_string(), name));
            }
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

/// 取當前語言的文案並代入變數；缺字串或格式失敗往下一層語言退，全失敗回 key。
pub fn t(key: &str, args: &[(&str, &str)]) -> String {
    let current = current_language();
    let mut order = vec![current.as_str()];
    for code in [DEFAULT_LANGUAGE, SOURCE_LANGUAGE] {
        if !order.contains(&code) {
            order.push(code);
        }
    }
    for code in order {
        let Some(template) = catalog(code).and_then(|c| c.get(key)) else {
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
