//! 術語表設定：config 裡的自訂術語表清單、載入時的正規化，以及目前啟用的那一份。

use std::sync::Arc;

use serde_json::{Map, Value};

use super::Config;
use super::services::{new_id, str_field, unique_name};
use crate::glossary::{Entry, Glossary, builtin};
use crate::i18n::t;
use crate::logs::log;
use crate::py::pystr;

/// config 裡存的一份術語表：自訂條目，可選擇同時引用一份內建術語表。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoredGlossary {
    pub id: String,
    pub name: String,
    /// 建立者，選填；格式同 [`CREATORS_KEY`](crate::glossary::CREATORS_KEY) 的值。
    pub creators: String,
    /// 引用的內建術語表代碼。
    pub builtin: Option<String>,
    pub entries: Vec<Entry>,
    /// 不認得的其他鍵，原樣寫回。
    pub extra: Map<String, Value>,
}

const KNOWN_KEYS: [&str; 5] = ["id", "name", "creators", "builtin", "entries"];

impl StoredGlossary {
    /// 從 config 的一筆讀出；不是物件回 `None`，非字串的條目值略過。
    pub fn from_value(v: &Value) -> Option<Self> {
        let map = v.as_object()?;
        let entries = map
            .get("entries")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(source, target)| {
                Some(Entry {
                    source: source.clone(),
                    target: target.as_str()?.to_owned(),
                })
            })
            .collect();
        let extra = map
            .iter()
            .filter(|(key, _)| !KNOWN_KEYS.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        Some(Self {
            id: str_field(v, "id").into(),
            name: str_field(v, "name").into(),
            creators: str_field(v, "creators").into(),
            builtin: map.get("builtin").and_then(Value::as_str).map(Into::into),
            entries,
            extra,
        })
    }

    /// 寫回 config 的形狀；`entries` 是依序的平面物件，建立者空白時省略，不認得的鍵接在後面。
    pub fn to_value(&self) -> Value {
        let entries: Map<String, Value> = self
            .entries
            .iter()
            .map(|e| (e.source.clone(), Value::String(e.target.clone())))
            .collect();
        let mut map = Map::new();
        map.insert("id".into(), self.id.clone().into());
        map.insert("name".into(), self.name.clone().into());
        // 沒填就不寫，沒用到建立者的舊設定檔才會原封不動
        if !self.creators.is_empty() {
            map.insert("creators".into(), self.creators.clone().into());
        }
        map.insert("builtin".into(), self.builtin.clone().into());
        map.insert("entries".into(), Value::Object(entries));
        map.extend(self.extra.clone());
        Value::Object(map)
    }
}

/// config 裡的全部術語表，依存放順序。
pub fn stored(cfg: &Config) -> Vec<StoredGlossary> {
    cfg.get("glossaries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(StoredGlossary::from_value)
        .collect()
}

/// 把一筆修成合法形狀（id、名稱、builtin、entries），`taken` 是已整理好的前面幾筆；第二個值表示有沒有實質修正。
fn clean_one(raw: &Value, taken: &[Value]) -> (Value, bool) {
    let mut repaired = false;
    let mut id = str_field(raw, "id").to_owned();
    if id.is_empty() || taken.iter().any(|g| str_field(g, "id") == id) {
        id = new_id(taken);
        repaired = true;
        log(format!(
            "[config] regenerated a missing or duplicate glossary id -> {id}"
        ));
    }
    let name = str_field(raw, "name");
    let base = if pystr::strip(name).is_empty() {
        t("glossary.default_name", &[])
    } else {
        name.to_owned()
    };
    let unique = unique_name(&base, taken, None);
    if unique != name {
        repaired = true;
        log(format!(
            "[config] renamed a blank or duplicate glossary name -> {unique:?}"
        ));
    }
    if raw
        .get("builtin")
        .is_some_and(|b| !b.is_null() && !b.is_string())
    {
        repaired = true;
        log(format!("[config] glossary {id}: dropped malformed builtin"));
    }
    if raw.get("creators").is_some_and(|c| !c.is_string()) {
        repaired = true;
        log(format!(
            "[config] glossary {id}: dropped malformed creators"
        ));
    }
    let raw_entries = raw.get("entries");
    let mut glossary = StoredGlossary::from_value(raw).unwrap_or_default();
    let kept = glossary.entries.len();
    let total = raw_entries.and_then(Value::as_object).map_or(0, Map::len);
    if raw_entries.is_some_and(|e| !e.is_object()) {
        repaired = true;
        log(format!("[config] glossary {id}: dropped malformed entries"));
    } else if kept < total {
        repaired = true;
        log(format!(
            "[config] glossary {id}: dropped {} malformed entries",
            total - kept
        ));
    }
    glossary.entries.retain(|e| !e.is_incomplete());
    let incomplete = kept - glossary.entries.len();
    if incomplete > 0 {
        repaired = true;
        log(format!(
            "[config] glossary {id}: dropped {incomplete} incomplete entries"
        ));
    }
    glossary.id = id;
    glossary.name = unique;
    (glossary.to_value(), repaired)
}

/// 就地整理 `glossaries` 與 `active_glossary`；回傳是否有變動（有就該寫回 config.json）。
pub fn normalize(cfg: &mut Config) -> bool {
    let before = cfg.get("glossaries").cloned().unwrap_or(Value::Null);
    let mut changed = false;
    let raw = match &before {
        Value::Array(items) => items.clone(),
        other => {
            if !other.is_null() {
                log("[config] glossaries is not a list; using an empty list");
                changed = true;
            }
            Vec::new()
        }
    };
    let mut clean: Vec<Value> = Vec::new();
    for item in &raw {
        if item.is_object() {
            let (one, repaired) = clean_one(item, &clean);
            changed |= repaired;
            clean.push(one);
        } else {
            log("[config] dropped a glossary entry that is not an object");
            changed = true;
        }
    }
    let ids: Vec<&str> = clean.iter().map(|g| str_field(g, "id")).collect();
    let active = cfg.get("active_glossary").cloned().unwrap_or(Value::Null);
    let wanted = match active.as_str() {
        Some(id) if ids.contains(&id) => active.clone(),
        _ => Value::Null,
    };
    if wanted != active {
        log(format!(
            "[config] active_glossary {active} is unknown; turning the glossary off"
        ));
        changed = true;
    }
    cfg.insert("glossaries".into(), Value::Array(clean));
    cfg.insert("active_glossary".into(), wanted);
    changed
}

/// 目前啟用的術語表；沒啟用或條目全空回 `None`。
pub fn active(cfg: &Config) -> Option<Arc<Glossary>> {
    let id = cfg.get("active_glossary").and_then(Value::as_str);
    let Some(chosen) = id.and_then(|id| stored(cfg).into_iter().find(|g| g.id == id)) else {
        log("[glossary] no active glossary");
        return None;
    };
    let id = &chosen.id;
    let imported = chosen.builtin.as_deref().and_then(|code| {
        let found = builtin::builtin_entries(code);
        if found.is_none() {
            log(format!(
                "[glossary] active glossary {id} imports missing builtin {code}; ignoring it"
            ));
        }
        found
    });
    let imported = imported.unwrap_or_default();
    log(format!(
        "[glossary] active glossary {id}: {} custom + {} builtin entries",
        chosen.entries.len(),
        imported.len()
    ));
    let glossary = Glossary::new(chosen.entries, imported);
    (!glossary.is_empty()).then(|| Arc::new(glossary))
}
