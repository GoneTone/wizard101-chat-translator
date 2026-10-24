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
#[derive(Clone, Debug, PartialEq)]
pub struct StoredGlossary {
    pub id: String,
    pub name: String,
    /// 引用的內建術語表代碼。
    pub builtin: Option<String>,
    pub entries: Vec<Entry>,
}

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
        Some(Self {
            id: str_field(v, "id").into(),
            name: str_field(v, "name").into(),
            builtin: map.get("builtin").and_then(Value::as_str).map(Into::into),
            entries,
        })
    }

    /// 寫回 config 的形狀；`entries` 是依序的平面物件。
    pub fn to_value(&self) -> Value {
        let entries: Map<String, Value> = self
            .entries
            .iter()
            .map(|e| (e.source.clone(), Value::String(e.target.clone())))
            .collect();
        let mut map = Map::new();
        map.insert("id".into(), self.id.clone().into());
        map.insert("name".into(), self.name.clone().into());
        map.insert("builtin".into(), self.builtin.clone().into());
        map.insert("entries".into(), Value::Object(entries));
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

/// 把一筆修成合法形狀（id、名稱、builtin、entries），`taken` 是已整理好的前面幾筆。
fn clean_one(raw: &Map<String, Value>, taken: &[Value]) -> Value {
    let raw_value = Value::Object(raw.clone());
    let mut id = str_field(&raw_value, "id").to_owned();
    if id.is_empty() || taken.iter().any(|g| str_field(g, "id") == id) {
        id = new_id(taken);
        log(format!(
            "[config] regenerated a missing or duplicate glossary id -> {id}"
        ));
    }
    let name = str_field(&raw_value, "name");
    let base = if pystr::strip(name).is_empty() {
        t("glossary.default_name", &[])
    } else {
        name.to_owned()
    };
    let unique = unique_name(&base, taken, None);
    if unique != name {
        log(format!(
            "[config] renamed a blank or duplicate glossary name -> {unique:?}"
        ));
    }
    let mut map = StoredGlossary::from_value(&raw_value)
        .expect("object")
        .to_value();
    map["id"] = id.into();
    map["name"] = unique.into();
    map
}

/// 就地整理 `glossaries` 與 `active_glossary`；回傳是否有變動（有就該寫回 config.json）。
pub fn normalize(cfg: &mut Config) -> bool {
    let before = cfg.get("glossaries").cloned().unwrap_or(Value::Null);
    let raw = match &before {
        Value::Array(items) => items.clone(),
        other => {
            if !other.is_null() {
                log("[config] glossaries is not a list; using an empty list");
            }
            Vec::new()
        }
    };
    let mut clean: Vec<Value> = Vec::new();
    for item in &raw {
        match item.as_object() {
            Some(map) => {
                let one = clean_one(map, &clean);
                clean.push(one);
            }
            None => log("[config] dropped a glossary entry that is not an object"),
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
    }
    let changed = Value::Array(clean.clone()) != before || wanted != active;
    cfg.insert("glossaries".into(), Value::Array(clean));
    cfg.insert("active_glossary".into(), wanted);
    changed
}

/// 目前啟用的術語表；沒啟用或條目全空回 `None`。
pub fn active(cfg: &Config) -> Option<Arc<Glossary>> {
    let id = cfg.get("active_glossary")?.as_str()?;
    let chosen = stored(cfg).into_iter().find(|g| g.id == id)?;
    let imported = chosen.builtin.as_deref().and_then(|code| {
        let found = builtin::builtin_entries(code);
        if found.is_none() {
            log(format!(
                "[glossary] active glossary {id} imports missing builtin {code}; ignoring it"
            ));
        }
        found
    });
    let glossary = Glossary::new(chosen.entries, imported.unwrap_or_default());
    (!glossary.is_empty()).then(|| Arc::new(glossary))
}
