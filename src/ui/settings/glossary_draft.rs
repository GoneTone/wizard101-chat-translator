//! 術語表分頁編輯中的草稿：術語表清單與啟用中的那一份，以及它們的增刪與存檔前檢查。

use serde_json::Value;

use super::window::SaveError;
use crate::config::Config;
use crate::config::glossaries::{StoredGlossary, stored};
use crate::config::services::{new_id, unique_name};
use crate::glossary::{Entry, GlossaryFile, RowIssue, builtin, review, without_builtin_duplicates};
use crate::i18n::t;
use crate::logs::log;
use crate::py::pystr;

/// 術語表分頁編輯中的兩個設定鍵：`glossaries` 與 `active_glossary`；`write_into` 寫回設定。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlossaryDraft {
    pub glossaries: Vec<StoredGlossary>,
    pub active: Option<String>,
}

/// 這份術語表引入的內建條目；沒引入或內建檔不存在時為空。
pub(super) fn builtin_of(glossary: &StoredGlossary) -> Vec<Entry> {
    glossary
        .builtin
        .as_deref()
        .and_then(builtin::builtin_entries)
        .unwrap_or_default()
}

/// 要不要讓這份術語表顯示引入內建術語表的選項；一份內建表都沒有時不顯示，
/// 但引用了已不存在的內建表時仍顯示，才看得到提示、能改回不引入。
pub(super) fn builtin_offered(glossary: &StoredGlossary) -> bool {
    glossary.builtin.is_some() || !builtin::builtin_codes().is_empty()
}

impl GlossaryDraft {
    /// 從設定取出兩個鍵。
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            glossaries: stored(cfg),
            active: cfg
                .get("active_glossary")
                .and_then(Value::as_str)
                .map(Into::into),
        }
    }

    /// 寫回設定：名稱、建立者與條目去頭尾空白，去掉兩側都空白與跟內建完全相同的條目，撞名的補序號。
    pub fn write_into(&self, cfg: &mut Config) {
        let mut written: Vec<Value> = Vec::new();
        for glossary in &self.glossaries {
            let mut glossary = glossary.clone();
            let entries: Vec<Entry> = glossary
                .entries
                .iter()
                .filter(|e| !e.is_blank())
                .map(|e| Entry {
                    source: pystr::strip(&e.source).into(),
                    target: pystr::strip(&e.target).into(),
                })
                .collect();
            glossary.entries = without_builtin_duplicates(&entries, &builtin_of(&glossary));
            let removed = entries.len() - glossary.entries.len();
            if removed > 0 {
                log(format!(
                    "[settings] glossary {}: removed {removed} entries identical to the built-in glossary",
                    glossary.id
                ));
            }
            glossary.name = pystr::strip(&glossary.name).into();
            glossary.creators = pystr::strip(&glossary.creators).into();
            let name = unique_name(&glossary.name, &written, None);
            if name != glossary.name {
                log(format!(
                    "[settings] glossary {} renamed to {name:?} to avoid a duplicate name",
                    glossary.id
                ));
                glossary.name = name;
            }
            written.push(glossary.to_value());
        }
        cfg.insert("glossaries".into(), Value::Array(written));
        cfg.insert("active_glossary".into(), self.active.clone().into());
    }

    /// 擋下儲存的原因（錯誤文案 key 與變數），依序為各份的名稱空白，再來是來源重複、條目只填一側。
    pub fn errors(&self) -> Vec<SaveError> {
        let mut blank = Vec::new();
        let mut duplicate = Vec::new();
        let mut incomplete = Vec::new();
        for (i, g) in self.glossaries.iter().enumerate() {
            let name = pystr::strip(&g.name);
            if name.is_empty() {
                blank.push((
                    "error.glossary_name_blank".into(),
                    vec![("index", (i + 1).to_string())],
                ));
            }
            let filled: Vec<Entry> = g
                .entries
                .iter()
                .filter(|e| !e.is_blank())
                .cloned()
                .collect();
            let issues = review(&filled, &builtin_of(g));
            // 名稱空白時引號裡會是空的，改用第幾份稱呼
            let named = |key: &str, unnamed: &str| -> SaveError {
                if name.is_empty() {
                    (unnamed.into(), vec![("index", (i + 1).to_string())])
                } else {
                    (key.into(), vec![("name", name.to_owned())])
                }
            };
            if issues.contains(&RowIssue::DuplicateSource) {
                duplicate.push(named(
                    "error.glossary_duplicate_source",
                    "error.glossary_duplicate_source_unnamed",
                ));
            }
            if issues.contains(&RowIssue::Incomplete) {
                incomplete.push(named(
                    "error.glossary_incomplete_entry",
                    "error.glossary_incomplete_entry_unnamed",
                ));
            }
        }
        [blank, duplicate, incomplete].concat()
    }

    /// 新增一份空白術語表（名稱用預設名，撞名補序號）；回傳它的 id。
    pub fn add_blank(&mut self) -> String {
        self.add_from_file(&t("glossary.default_name", &[]), GlossaryFile::default())
    }

    /// 新增一份帶 `file` 建立者與條目的術語表（撞名補序號）；回傳它的 id。
    pub fn add_from_file(&mut self, name: &str, file: GlossaryFile) -> String {
        self.push(StoredGlossary {
            name: name.into(),
            creators: file.creators,
            entries: file.entries,
            ..Default::default()
        })
    }

    /// 複製一份（名稱「原名 的副本」，條目與引入的內建一起帶過去）；回傳新的 id，找不到 `id` 回 `None`。
    pub fn duplicate(&mut self, id: &str) -> Option<String> {
        let source = self.find(id)?.clone();
        let name = t("glossary.copy_name", &[("name", &source.name)]);
        Some(self.push(StoredGlossary { name, ..source }))
    }

    /// 刪除一份；刪到啟用中的就改為不使用。
    pub fn remove(&mut self, id: &str) {
        self.glossaries.retain(|g| g.id != id);
        log(format!("[settings] glossary {id} deleted"));
        if self.active.as_deref() == Some(id) {
            self.active = None;
            log("[settings] the active glossary was deleted; glossary turned off");
        }
    }

    /// 啟用 `id` 那份；`None` 是不使用。
    pub fn set_active(&mut self, id: Option<String>) {
        log(format!(
            "[settings] active glossary set to {}",
            id.as_deref().unwrap_or("none")
        ));
        self.active = id;
    }

    pub fn find(&self, id: &str) -> Option<&StoredGlossary> {
        self.glossaries.iter().find(|g| g.id == id)
    }

    /// 配新 id、名稱去重後加在最後。
    fn push(&mut self, mut glossary: StoredGlossary) -> String {
        let taken: Vec<Value> = self
            .glossaries
            .iter()
            .map(StoredGlossary::to_value)
            .collect();
        glossary.id = new_id(&taken);
        glossary.name = unique_name(&glossary.name, &taken, None);
        log(format!(
            "[settings] glossary {} added ({} entries)",
            glossary.id,
            glossary.entries.len()
        ));
        let id = glossary.id.clone();
        self.glossaries.push(glossary);
        id
    }
}
