//! 術語表條目：解析、匯出與比對用的正規化。

pub mod builtin;
mod matcher;
mod review;

pub use matcher::{Glossary, hits_key};
pub use review::{
    Conflict, MergePlan, RowIssue, apply_merge, overridden, plan_merge, review,
    without_builtin_duplicates,
};

use serde_json::{Map, Value};

use crate::py::pystr;

const NOT_STRING_OBJECT: &str = "expected an object of string values";

/// 一筆術語：原文詞與指定譯名。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub source: String,
    pub target: String,
}

/// 術語的顯示形式 `原文 <=> 譯名`，提示詞、log 與命中鍵共用。
impl std::fmt::Display for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} <=> {}", self.source, self.target)
    }
}

/// 比對用的正規化形式（casefold）。
pub fn fold(s: &str) -> String {
    pystr::casefold(s)
}

/// 解析平面 `{"原文": "譯名"}`，依檔案順序；兩側去頭尾空白，任一側為空就略過。
pub fn parse_entries(json: &str) -> Result<Vec<Entry>, String> {
    let Value::Object(map) = serde_json::from_str::<Value>(json).map_err(|e| e.to_string())? else {
        return Err(NOT_STRING_OBJECT.into());
    };
    let mut entries = Vec::new();
    for (source, target) in map {
        let Value::String(target) = target else {
            return Err(NOT_STRING_OBJECT.into());
        };
        let (source, target) = (pystr::strip(&source), pystr::strip(&target));
        if !source.is_empty() && !target.is_empty() {
            entries.push(Entry {
                source: source.into(),
                target: target.into(),
            });
        }
    }
    Ok(entries)
}

/// 輸出縮排兩格的平面物件，可被 `parse_entries` 讀回。
pub fn entries_to_json(entries: &[Entry]) -> String {
    let map: Map<String, Value> = entries
        .iter()
        .map(|e| (e.source.clone(), Value::String(e.target.clone())))
        .collect();
    serde_json::to_string_pretty(&map).expect("string map serializes")
}
