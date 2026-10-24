//! 內建術語表：`src/glossary/<語系代碼>.json` 編進 exe，key 是英文（遊戲語言）術語，value 是該語系譯名。

use std::sync::LazyLock;

use super::{Entry, fold, parse_entries};
use crate::logs::log;

/// 編進 exe 的內建檔，依代碼字母序；清單由 `build.rs` 掃描本資料夾產生，新增語系只要放檔案。
const BUILTINS: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/glossary_builtins.rs"));

static LOADED: LazyLock<Vec<(&'static str, Vec<Entry>)>> = LazyLock::new(|| {
    let mut loaded = Vec::new();
    for (code, raw) in BUILTINS {
        let all = parse_entries(raw).unwrap_or_default().len();
        let entries = load(raw);
        log(format!(
            "[glossary] builtin {code}: {} entries ({} untranslated dropped)",
            entries.len(),
            all - entries.len()
        ));
        if !entries.is_empty() {
            loaded.push((*code, entries));
        }
    }
    loaded
});

/// 有內建術語表的語系代碼（濾完非空），依代碼排序。
pub fn builtin_codes() -> Vec<&'static str> {
    let mut codes: Vec<_> = LOADED.iter().map(|(code, _)| *code).collect();
    codes.sort_unstable();
    codes
}

/// 取某語系的內建條目；不存在或濾完為空回 `None`。
pub fn builtin_entries(code: &str) -> Option<Vec<Entry>> {
    LOADED
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, entries)| entries.clone())
}

/// 解析內建檔並濾掉未翻譯（原文與譯名相同）的條目；解析失敗回空並留 log。
pub fn load(raw: &str) -> Vec<Entry> {
    match parse_entries(raw) {
        Ok(entries) => entries
            .into_iter()
            .filter(|e| fold(&e.source) != fold(&e.target))
            .collect(),
        Err(err) => {
            log(format!("[glossary] builtin parse failed: {err}"));
            Vec::new()
        }
    }
}
