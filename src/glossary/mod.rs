//! 術語表條目與檔案：解析、匯出（含建立者）與比對用的正規化。

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

/// 術語表檔案裡記建立者的保留鍵；值是一段文字，可含 `[名字](網址)` 連結，多人寫在一起。
pub const CREATORS_KEY: &str = "glossary.creators";

/// 一筆術語：原文詞與指定譯名。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub source: String,
    pub target: String,
}

impl Entry {
    /// 兩側（去頭尾空白後）都是空的：設定介面忽略、存檔時丟掉。
    pub fn is_blank(&self) -> bool {
        pystr::strip(&self.source).is_empty() && pystr::strip(&self.target).is_empty()
    }

    /// 任一側（去頭尾空白後）是空的。
    pub fn is_incomplete(&self) -> bool {
        pystr::strip(&self.source).is_empty() || pystr::strip(&self.target).is_empty()
    }
}

/// 術語的顯示形式 `原文 <=> 譯名`，提示詞、log 與命中鍵共用。
impl std::fmt::Display for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} <=> {}", self.source, self.target)
    }
}

/// 比對用的正規化形式（去頭尾空白再 casefold）。
pub fn fold(s: &str) -> String {
    pystr::casefold(pystr::strip(s))
}

/// 一份術語表檔案：建立者與條目。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlossaryFile {
    pub creators: String,
    pub entries: Vec<Entry>,
}

/// 術語表檔案讀不懂的原因；`Display` 是給 log 的英文。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// 不是合法的 JSON，`line` 從 1 起算。
    Json { line: usize },
    /// 最外層不是物件。
    NotObject,
    /// 這個鍵的值不是文字。
    NotText { key: String },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json { line } => write!(f, "invalid JSON at line {line}"),
            Self::NotObject => write!(f, "top level is not an object"),
            Self::NotText { key } => write!(f, "value of {key:?} is not a string"),
        }
    }
}

/// 解析平面 `{"原文": "譯名"}`，依檔案順序；[`CREATORS_KEY`] 是建立者，不算條目。
///
/// 兩側去頭尾空白，任一側為空就略過。
pub fn parse_file(json: &str) -> Result<GlossaryFile, ParseError> {
    let value =
        serde_json::from_str::<Value>(json).map_err(|e| ParseError::Json { line: e.line() })?;
    let Value::Object(map) = value else {
        return Err(ParseError::NotObject);
    };
    let mut file = GlossaryFile::default();
    for (key, value) in map {
        let Value::String(text) = value else {
            return Err(ParseError::NotText { key });
        };
        if key == CREATORS_KEY {
            file.creators = pystr::strip(&text).into();
            continue;
        }
        let (source, target) = (pystr::strip(&key), pystr::strip(&text));
        if !source.is_empty() && !target.is_empty() {
            file.entries.push(Entry {
                source: source.into(),
                target: target.into(),
            });
        }
    }
    Ok(file)
}

/// 輸出縮排兩格的平面物件（有建立者時放第一個鍵），可被 `parse_file` 讀回。
pub fn file_to_json(file: &GlossaryFile) -> String {
    let mut map = Map::new();
    if !file.creators.is_empty() {
        map.insert(CREATORS_KEY.into(), file.creators.clone().into());
    }
    map.extend(
        file.entries
            .iter()
            .map(|e| (e.source.clone(), Value::String(e.target.clone()))),
    );
    serde_json::to_string_pretty(&map).expect("string map serializes")
}
