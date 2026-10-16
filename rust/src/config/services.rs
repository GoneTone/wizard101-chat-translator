//! 翻譯服務：服務商欄位表、舊格式遷移與正規化、用途插槽的解析與必填欄位判定。
//!
//! 「服務」是使用者建立的一筆具名設定 —— 某家服務商加上金鑰、模型等欄位。

use std::hash::{BuildHasher, Hasher, RandomState};

use serde_json::{Map, Value};

use super::Config;
use super::py;
use crate::i18n::t;
use crate::logs::log;

/// 這一版認不得 provider 的服務搬去的頂層鍵；日後認得它的版本會自動搬回 services。
pub const UNSUPPORTED_SERVICES: &str = "unsupported_services";

enum Field {
    Text(&'static str),
    Flag(bool),
}

struct Provider {
    key: &'static str,
    brand: Option<&'static str>,
    fields: &'static [(&'static str, Field)],
}

impl Provider {
    fn has_field(&self, name: &str) -> bool {
        self.fields.iter().any(|(n, _)| *n == name)
    }

    fn short_name(&self) -> String {
        self.brand.map_or_else(
            || t(&format!("provider.{}_short", self.key), &[]),
            str::to_string,
        )
    }
}

impl Field {
    fn to_value(&self) -> Value {
        match self {
            Field::Text(s) => Value::String((*s).into()),
            Field::Flag(b) => Value::Bool(*b),
        }
    }

    fn accepts(&self, value: &Value) -> bool {
        matches!(
            (self, value),
            (Field::Text(_), Value::String(_)) | (Field::Flag(_), Value::Bool(_))
        )
    }
}

// 服務商清單與欄位的單一真實來源；Claude 的 effort 維持 auto —— Haiku 4.5 帶 effort 會 400。
const PROVIDERS: &[Provider] = &[
    Provider {
        key: "openai",
        brand: Some("OpenAI"),
        fields: &[
            ("model", Field::Text("")),
            ("api_key", Field::Text("")),
            ("thinking", Field::Flag(false)),
        ],
    },
    Provider {
        key: "claude",
        brand: Some("Claude"),
        fields: &[
            ("model", Field::Text("")),
            ("api_key", Field::Text("")),
            ("effort", Field::Text("auto")),
        ],
    },
    Provider {
        key: "gemini",
        brand: Some("Gemini"),
        fields: &[
            ("model", Field::Text("")),
            ("api_key", Field::Text("")),
            ("effort", Field::Text("low")),
        ],
    },
    Provider {
        key: "custom",
        brand: None,
        fields: &[
            ("base_url", Field::Text("")),
            ("model", Field::Text("")),
            ("api_key", Field::Text("")),
            ("thinking", Field::Flag(false)),
        ],
    },
];

const LEGACY_FLAT_FIELDS: &[&str] = &["model", "api_key", "thinking", "effort", "base_url"];
// 遷移時判定「這家使用者填過東西」的欄位；三個都空就不留空殼。
const FILLED_MARKERS: &[&str] = &["model", "api_key", "base_url"];

fn provider(key: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.key == key)
}

/// 用途插槽：三個用途可各自指定服務，未指定就跟隨預設。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceSlot {
    /// 收訊翻譯。
    Incoming,
    /// 發話翻譯。
    Outgoing,
    /// 框選畫面區域翻譯。
    Region,
}

impl ServiceSlot {
    /// 全部插槽，依 config.json 的鍵順序。
    pub const ALL: [ServiceSlot; 3] = [Self::Incoming, Self::Outgoing, Self::Region];

    /// 在 `service_slots` 裡的鍵名。
    pub fn key(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
            Self::Region => "region",
        }
    }
}

/// 某用途實際生效的服務；欄位不存在時為空字串或 `false`。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceSpec {
    /// 服務 id（尚未設定任何服務時為空）。
    pub id: String,
    /// 服務名稱。
    pub name: String,
    /// 服務商鍵名。
    pub provider: String,
    /// 模型名稱。
    pub model: String,
    /// API 金鑰。
    pub api_key: String,
    /// 自訂端點網址。
    pub base_url: String,
    /// OpenAI 相容端點的思考開關。
    pub thinking: bool,
    /// Claude／Gemini 的思考深度。
    pub effort: String,
}

impl ServiceSpec {
    fn from_service(service: &Value) -> Self {
        let text = |k: &str| {
            service
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        ServiceSpec {
            id: text("id"),
            name: text("name"),
            provider: text("provider"),
            model: text("model"),
            api_key: text("api_key"),
            base_url: text("base_url"),
            thinking: service
                .get("thinking")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            effort: text("effort"),
        }
    }
}

/// 檢查一筆服務的必填欄位，回傳錯誤文案 key 列表（空＝通過）。
pub fn validate_service(service: &Value) -> Vec<String> {
    let text = |k: &str| service.get(k).and_then(Value::as_str).unwrap_or("");
    let needs_base_url = provider(text("provider")).is_some_and(|p| p.has_field("base_url"));
    let mut errors = Vec::new();
    if py::is_blank(text("model")) {
        errors.push("error.need_model".to_string());
    }
    if !needs_base_url && py::is_blank(text("api_key")) {
        errors.push("error.need_api_key".to_string());
    }
    if needs_base_url && py::is_blank(text("base_url")) {
        errors.push("error.need_base_url".to_string());
    }
    errors
}

/// 依 id 取服務本體；找不到回 `None`。
pub fn find<'a>(cfg: &'a Config, id: Option<&str>) -> Option<&'a Value> {
    let id = id?;
    cfg.get("services")?
        .as_array()?
        .iter()
        .find(|s| s.get("id").and_then(Value::as_str) == Some(id))
}

/// 該用途實際生效的服務：插槽未指定就跟隨預設，連預設都沒有時回第一家服務商的空白設定。
pub fn resolve(cfg: &Config, slot: ServiceSlot) -> ServiceSpec {
    let slot_id = cfg
        .get("service_slots")
        .and_then(|s| s.get(slot.key()))
        .and_then(Value::as_str);
    let default_id = cfg.get("default_service").and_then(Value::as_str);
    match find(cfg, slot_id).or_else(|| find(cfg, default_id)) {
        Some(service) => ServiceSpec::from_service(service),
        None => {
            let fallback = &PROVIDERS[0];
            let mut blank = defaults(fallback);
            blank.insert("provider".into(), fallback.key.into());
            ServiceSpec::from_service(&Value::Object(blank))
        }
    }
}

fn defaults(p: &Provider) -> Map<String, Value> {
    p.fields
        .iter()
        .map(|(k, f)| ((*k).to_string(), f.to_value()))
        .collect()
}

fn ids_and_names<'a>(services: &'a [Value], key: &str) -> impl Iterator<Item = &'a str> {
    services
        .iter()
        .filter_map(move |s| s.get(key).and_then(Value::as_str))
}

fn new_id(services: &[Value]) -> String {
    loop {
        // RandomState 每次建構都換一組 SipHash 金鑰，空輸入的雜湊即一個亂數；id 只求不撞，不需密碼學強度
        let candidate = format!("{:08x}", RandomState::new().build_hasher().finish() as u32);
        if !ids_and_names(services, "id").any(|id| id == candidate) {
            return candidate;
        }
    }
}

/// 撞名就補序號：第一筆無後綴，之後 `名稱 (2)`、`名稱 (3)`。
fn unique_name(name: &str, services: &[Value]) -> String {
    let taken = |candidate: &str| ids_and_names(services, "name").any(|n| n == candidate);
    if !taken(name) {
        return name.to_string();
    }
    (2..)
        .map(|n| format!("{name} ({n})"))
        .find(|c| !taken(c))
        .unwrap_or_default()
}

fn new_service(p: &Provider, services: &[Value]) -> Map<String, Value> {
    let mut service = Map::new();
    service.insert("id".into(), new_id(services).into());
    service.insert("name".into(), unique_name(&p.short_name(), services).into());
    service.insert("provider".into(), p.key.into());
    service.extend(defaults(p));
    service
}

/// 服務清單欄位的取值：非 list 的原值整個當成一筆（最可能是貼歪的金鑰），缺值或 null 為空清單。
fn as_entries(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(items)) => items.clone(),
        None | Some(Value::Null) => Vec::new(),
        Some(other) => vec![other.clone()],
    }
}

/// 隔離區現有的內容；取值與 services 一致。
pub(crate) fn quarantined(cfg: &Config) -> Vec<Value> {
    as_entries(cfg.get(UNSUPPORTED_SERVICES))
}

fn known_provider(entry: &Value) -> Option<&'static Provider> {
    entry
        .get("provider")
        .and_then(Value::as_str)
        .and_then(provider)
}

fn is_migrated_service(entry: &Value) -> bool {
    known_provider(entry).is_some()
        && FILLED_MARKERS.iter().any(|k| {
            entry
                .get(*k)
                .and_then(Value::as_str)
                .is_some_and(|s| !py::is_blank(s))
        })
}

fn has_key(entry: &Value) -> &'static str {
    py::py_bool(py::truthy(entry.get("api_key")))
}

fn normalize_containers(cfg: &mut Config) -> bool {
    let mut changed = false;
    let services = cfg.get("services").cloned();
    if !matches!(services, Some(Value::Array(_))) {
        cfg.insert(
            "services".into(),
            Value::Array(as_entries(services.as_ref())),
        );
        // 缺鍵或 None 只是補一個空清單，沒有東西被保住，不算變動
        if let Some(value) = services.filter(|v| !v.is_null()) {
            log(format!(
                "[config] services is a {}, not a list; kept the value as a single entry",
                py::type_name(&value)
            ));
            changed = true;
        }
    }
    if let Some(parked) = cfg
        .get(UNSUPPORTED_SERVICES)
        .filter(|v| !v.is_null() && !v.is_array())
        .cloned()
    {
        log(format!(
            "[config] {UNSUPPORTED_SERVICES} is a {}, not a list; kept the value as a single entry",
            py::type_name(&parked)
        ));
        cfg.insert(UNSUPPORTED_SERVICES.into(), Value::Array(vec![parked]));
        changed = true;
    }
    changed
}

fn promote_known_services(cfg: &mut Config) -> bool {
    let (promoted, parked): (Vec<Value>, Vec<Value>) = quarantined(cfg)
        .into_iter()
        .partition(|e| known_provider(e).is_some());
    if promoted.is_empty() {
        return false;
    }
    cfg.insert(UNSUPPORTED_SERVICES.into(), Value::Array(parked));
    let mut services = as_entries(cfg.get("services"));
    services.extend(promoted.iter().cloned());
    cfg.insert("services".into(), Value::Array(services));
    for entry in &promoted {
        log(format!(
            "[config] promoted a quarantined {} service back into the service list (has_key={})",
            py::display(&entry["provider"]),
            has_key(entry)
        ));
    }
    true
}

fn quarantine_unknown_services(cfg: &mut Config) -> bool {
    let (known, unknown): (Vec<Value>, Vec<Value>) = as_entries(cfg.get("services"))
        .into_iter()
        .partition(|e| known_provider(e).is_some());
    if unknown.is_empty() {
        return false;
    }
    cfg.insert("services".into(), Value::Array(known));
    let mut parked = quarantined(cfg);
    parked.extend(unknown.iter().cloned());
    cfg.insert(UNSUPPORTED_SERVICES.into(), Value::Array(parked));
    for entry in &unknown {
        match entry.get("provider") {
            Some(Value::String(name)) => log(format!(
                "[config] quarantined a service with unknown provider {} into {UNSUPPORTED_SERVICES} \
                 (has_key={}); a build that supports it will restore it",
                py::str_repr(name),
                has_key(entry)
            )),
            // 形狀讀不懂的那些沒有任何一版認得，log 不能拿「日後會搬回來」誤導使用者
            _ => log(format!(
                "[config] quarantined an entry this build cannot read into {UNSUPPORTED_SERVICES} \
                 (type={}); it is kept verbatim, nothing will restore it, delete it yourself if it is junk",
                py::type_name(entry)
            )),
        }
    }
    true
}

/// 舊 api 區塊攤成（provider, 欄位）清單，連同選中的 provider；最舊的扁平格式先搬進所屬服務商。
fn api_sections(api: Map<String, Value>) -> (Option<Value>, Vec<(Value, Value)>) {
    if !LEGACY_FLAT_FIELDS.iter().any(|k| api.contains_key(*k)) {
        let selected = api.get("provider").cloned();
        let sections = api
            .into_iter()
            .filter(|(k, _)| k != "provider")
            .map(|(k, v)| (Value::String(k), v))
            .collect();
        return (selected, sections);
    }
    let name = api
        .get("provider")
        .cloned()
        .unwrap_or_else(|| "custom".into());
    // 認不得的 provider 沒有欄位表可篩，篩了就是丟掉金鑰 —— 整組留著給隔離區接手
    let fields = name.as_str().and_then(provider);
    let profile: Map<String, Value> = api
        .into_iter()
        .filter(|(k, _)| k != "provider" && fields.is_none_or(|p| p.has_field(k)))
        .collect();
    let mut keys: Vec<Value> = profile.keys().map(|k| Value::String(k.clone())).collect();
    keys.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    log(format!(
        "[config] unflattened the legacy api block into provider={} (fields={})",
        py::display(&name),
        py::repr(&Value::Array(keys))
    ));
    (Some(name.clone()), vec![(name, Value::Object(profile))])
}

fn quarantine_unknown_api_profiles(cfg: &mut Config, sections: &[(Value, Value)]) {
    for (name, profile) in sections {
        if name.as_str().and_then(provider).is_some() {
            continue;
        }
        let Some(profile) = profile.as_object().filter(|m| !m.is_empty()) else {
            continue;
        };
        let mut entry = Map::new();
        entry.insert("provider".into(), name.clone());
        entry.extend(profile.clone());
        let mut parked = quarantined(cfg);
        parked.push(Value::Object(entry));
        cfg.insert(UNSUPPORTED_SERVICES.into(), Value::Array(parked));
        log(format!(
            "[config] quarantined the api block's unknown provider {} into {UNSUPPORTED_SERVICES} \
             (has_key={}); a build that supports it will restore it",
            py::repr(name),
            py::py_bool(py::truthy(profile.get("api_key")))
        ));
    }
}

fn migrate_api_block(cfg: &mut Config, migrated_before: bool) -> bool {
    let Some(Value::Object(api)) = cfg.shift_remove("api") else {
        return false;
    };
    let (selected, sections) = api_sections(api);
    // 隔離要在早退之前：丟得掉的只有這一版看得懂、而且確定重複的東西
    quarantine_unknown_api_profiles(cfg, &sections);
    if migrated_before {
        log("[config] dropped a leftover api block; the service list already exists");
        return true;
    }
    let kept = as_entries(cfg.get("services"));
    let mut all = kept.clone();
    let mut migrated = 0;
    let mut default_id = None;
    for p in PROVIDERS {
        let Some(Value::Object(profile)) = sections
            .iter()
            .find(|(name, _)| name.as_str() == Some(p.key))
            .map(|(_, v)| v)
        else {
            continue;
        };
        let filled = FILLED_MARKERS.iter().any(|k| {
            profile
                .get(*k)
                .and_then(Value::as_str)
                .is_some_and(|s| !py::is_blank(s))
        });
        if !filled {
            continue;
        }
        // 新 id 要一起避開促轉回來的服務：撞號會讓 default_service 落到錯的那筆
        let mut service = new_service(p, &all);
        for (k, v) in profile.iter().filter(|(k, _)| p.has_field(k)) {
            service.insert(k.clone(), v.clone());
        }
        if selected.as_ref().and_then(Value::as_str) == Some(p.key) {
            default_id = Some(service["id"].clone());
        }
        all.push(Value::Object(service));
        migrated += 1;
    }
    // api 區塊的 provider 具權威性：對不上時也要退回這個區塊遷移出來的那批
    let fallback = all.get(kept.len()).map(|s| s["id"].clone()).or_else(|| {
        kept.first()
            .map(|k| k.get("id").cloned().unwrap_or(Value::Null))
    });
    let default = default_id.or(fallback).unwrap_or(Value::Null);
    log(format!(
        "[config] migrated the api block into {migrated} service(s); default={}",
        py::display(&default)
    ));
    cfg.insert("services".into(), Value::Array(all));
    cfg.insert("default_service".into(), default);
    if !kept.is_empty() {
        log(format!(
            "[config] the api block was migrated next to {} service(s) already in the list; \
             an account present in both stays as two entries",
            kept.len()
        ));
    }
    true
}

fn profile_field(entry: &Value, provider: &str, key: &str, default: &Field) -> Value {
    match entry.get(key) {
        None => default.to_value(),
        Some(value) if default.accepts(value) => value.clone(),
        Some(value) => {
            log(format!(
                "[config] {provider} service field {key} has type {}; using the default",
                py::type_name(value)
            ));
            default.to_value()
        }
    }
}

fn sanitize_services(cfg: &mut Config) -> bool {
    let before = Value::Array(as_entries(cfg.get("services")));
    let mut clean: Vec<Value> = Vec::new();
    for entry in before.as_array().into_iter().flatten() {
        let Some(p) = known_provider(entry) else {
            continue;
        };
        let text = |k: &str| {
            entry
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let mut service = Map::new();
        service.insert("id".into(), text("id").into());
        service.insert("name".into(), text("name").into());
        service.insert("provider".into(), p.key.into());
        for (key, default) in p.fields {
            service.insert((*key).into(), profile_field(entry, p.key, key, default));
        }
        let mut stale: Vec<&str> = entry
            .as_object()
            .into_iter()
            .flat_map(|m| m.keys())
            .filter(|k| !service.contains_key(*k))
            .map(String::as_str)
            .collect();
        if !stale.is_empty() {
            stale.sort();
            log(format!(
                "[config] dropped stale fields on a {} service: {}",
                p.key,
                stale.join(", ")
            ));
        }
        let id = text("id");
        if id.is_empty() || ids_and_names(&clean, "id").any(|other| other == id) {
            let fresh = new_id(&clean);
            log(format!(
                "[config] regenerated a missing or duplicate service id -> {fresh}"
            ));
            service.insert("id".into(), fresh.into());
        }
        // 每一筆都過一次去重：分派下拉只顯示名稱，同名會讓使用者選到另一筆服務
        let name = text("name");
        let unique = unique_name(
            if name.is_empty() {
                p.short_name()
            } else {
                name.clone()
            }
            .as_str(),
            &clean,
        );
        if unique != name {
            log(format!(
                "[config] renamed a missing or duplicate service name -> {}",
                py::str_repr(&unique)
            ));
        }
        service.insert("name".into(), unique.into());
        clean.push(Value::Object(service));
    }
    let clean = Value::Array(clean);
    let changed = !py::eq(&clean, &before);
    cfg.insert("services".into(), clean);
    changed
}

fn sanitize_pointers(cfg: &mut Config) -> bool {
    let services = as_entries(cfg.get("services"));
    let ids: Vec<&str> = ids_and_names(&services, "id").collect();
    let mut changed = false;
    let default = cfg.get("default_service").cloned().unwrap_or(Value::Null);
    let wanted = match default.as_str() {
        Some(id) if ids.contains(&id) => default.clone(),
        _ => ids
            .first()
            .map_or(Value::Null, |id| Value::String((*id).into())),
    };
    if !py::eq(&wanted, &default) {
        log(format!(
            "[config] default_service {} is unknown; using {}",
            py::repr(&default),
            py::repr(&wanted)
        ));
        cfg.insert("default_service".into(), wanted);
        changed = true;
    }
    let slots = cfg
        .get("service_slots")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut clean = Map::new();
    for slot in ServiceSlot::ALL {
        let mut value = slots.get(slot.key()).cloned().unwrap_or(Value::Null);
        if !value.is_null() && !value.as_str().is_some_and(|id| ids.contains(&id)) {
            log(format!(
                "[config] service_slots.{} points at unknown service {}; following the default instead",
                slot.key(),
                py::repr(&value)
            ));
            value = Value::Null;
        }
        clean.insert(slot.key().into(), value);
    }
    let clean = Value::Object(clean);
    if !py::eq(&clean, &Value::Object(slots)) {
        changed = true;
    }
    cfg.insert("service_slots".into(), clean);
    changed
}

/// 就地遷移舊格式、補齊欄位、清掉指不到的參照；回傳是否有變動（有就該寫回 config.json）。
pub fn normalize(cfg: &mut Config) -> bool {
    let mut changed = normalize_containers(cfg);
    // 遷移與否的證據要在促轉之前取樣：促轉之後清單裡會混進與 api 區塊無關的服務
    let migrated_before = as_entries(cfg.get("services"))
        .iter()
        .any(is_migrated_service);
    changed |= promote_known_services(cfg);
    changed |= quarantine_unknown_services(cfg);
    changed |= migrate_api_block(cfg, migrated_before);
    changed |= sanitize_services(cfg);
    sanitize_pointers(cfg) || changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unique_name_numbers_collisions_from_two() {
        let mut services = vec![json!({"id": "a", "name": "ChatGPT"})];
        assert_eq!(unique_name("ChatGPT", &services), "ChatGPT (2)");
        services.push(json!({"id": "b", "name": "ChatGPT (2)"}));
        assert_eq!(unique_name("ChatGPT", &services), "ChatGPT (3)");
        assert_eq!(unique_name("Claude", &services), "Claude");
    }

    #[test]
    fn new_service_fills_the_provider_defaults_and_a_name() {
        let service = new_service(provider("claude").unwrap(), &[]);
        assert_eq!(service["provider"], "claude");
        assert_eq!(service["name"], "Claude");
        assert_eq!(service["model"], "");
        assert_eq!(service["effort"], "auto");
        assert_eq!(service["id"].as_str().unwrap().len(), 8);
        assert_eq!(
            new_service(provider("gemini").unwrap(), &[])["effort"],
            "low"
        );
    }

    #[test]
    fn new_services_never_share_an_id() {
        let mut services = Vec::new();
        for _ in 0..50 {
            let service = new_service(provider("openai").unwrap(), &services);
            services.push(Value::Object(service));
        }
        let mut ids: Vec<&str> = ids_and_names(&services, "id").collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 50);
    }

    #[test]
    fn resolve_returns_the_blank_first_provider_without_services() {
        let spec = resolve(&Config::new(), ServiceSlot::Incoming);
        assert_eq!(spec.provider, "openai");
        assert_eq!(spec.model, "");
        assert!(!spec.thinking);
    }
}
