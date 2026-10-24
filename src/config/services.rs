//! 翻譯服務：服務商欄位表、舊格式遷移與正規化、用途插槽的解析與必填欄位判定。
//!
//! 「服務」是使用者建立的一筆具名設定 —— 某家服務商加上金鑰、模型等欄位。

use std::fmt;

use serde_json::{Map, Value};

use super::Config;
use super::py;
use crate::i18n::t;
use crate::logs::log;
use crate::py::pyrepr::repr_str;
use crate::py::pystr;
use crate::util::random_u64;

/// 這一版認不得 provider 的服務搬去的頂層鍵；日後認得它的版本會自動搬回 services。
pub const UNSUPPORTED_SERVICES: &str = "unsupported_services";

enum Field {
    Text(&'static str),
    Flag(bool),
}

/// 服務商的顯示資料與欄位表；該畫哪些欄位一律問 `has_field`。
pub struct Provider {
    /// 存在 config.json 的鍵名。
    pub key: &'static str,
    /// 服務商名稱的語言檔 key。
    pub label_key: &'static str,
    /// 服務商說明的語言檔 key。
    pub desc_key: &'static str,
    /// 取得 API 金鑰的網址；自訂端點沒有。
    pub key_url: Option<&'static str>,
    brand: Option<&'static str>,
    fields: &'static [(&'static str, Field)],
    /// 該家 API 的思考深度等級，依顯示順序（不含 [`EFFORT_AUTO`]）。
    pub efforts: &'static [&'static str],
}

impl Provider {
    /// 這家服務商有沒有該欄位。
    pub fn has_field(&self, name: &str) -> bool {
        self.fields.iter().any(|(n, _)| *n == name)
    }

    /// 這家要不要自己填端點網址（官方端點的網址寫死在 translator）。
    pub fn needs_base_url(&self) -> bool {
        self.has_field("base_url")
    }

    /// 該家的欄位與預設值，依欄位表順序。
    pub fn defaults(&self) -> Map<String, Value> {
        self.fields
            .iter()
            .map(|(k, f)| ((*k).to_string(), f.to_value()))
            .collect()
    }

    /// 新建服務時的預設名稱；品牌名不進語言檔，只有自訂端點要翻譯。
    pub fn short_name(&self) -> String {
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

/// 思考深度「自動」：不帶 effort 相關參數，維持模型預設。
pub const EFFORT_AUTO: &str = "auto";
/// 思考深度「低」。
pub const EFFORT_LOW: &str = "low";

const OPENAI_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "max"];

/// `value` 是 [`EFFORT_AUTO`] 或該家的等級就原樣回傳，否則（例如手改 config）當成自動。
pub fn known_effort(provider: &Provider, value: &str) -> &'static str {
    provider
        .efforts
        .iter()
        .copied()
        .find(|level| *level == value)
        .unwrap_or(EFFORT_AUTO)
}

// 服務商清單與欄位的單一真實來源
const PROVIDERS: &[Provider] = &[
    Provider {
        key: "openai",
        label_key: "provider.openai",
        desc_key: "provider.openai_desc",
        key_url: Some("https://platform.openai.com/api-keys"),
        brand: Some("OpenAI"),
        fields: &[
            ("model", Field::Text("")),
            ("api_key", Field::Text("")),
            ("thinking", Field::Flag(false)),
            ("effort", Field::Text(EFFORT_AUTO)),
        ],
        efforts: OPENAI_EFFORTS,
    },
    Provider {
        key: "claude",
        label_key: "provider.claude",
        desc_key: "provider.claude_desc",
        key_url: Some("https://console.anthropic.com/settings/keys"),
        brand: Some("Claude"),
        fields: &[
            ("model", Field::Text("")),
            ("api_key", Field::Text("")),
            ("thinking", Field::Flag(true)),
            // 不知道模型時不帶 effort：Haiku 4.5 帶 effort 會 400
            ("effort", Field::Text(EFFORT_AUTO)),
        ],
        efforts: &["low", "medium", "high", "xhigh", "max"],
    },
    Provider {
        key: "gemini",
        label_key: "provider.gemini",
        desc_key: "provider.gemini_desc",
        key_url: Some("https://aistudio.google.com/apikey"),
        brand: Some("Gemini"),
        fields: &[
            ("model", Field::Text("")),
            ("api_key", Field::Text("")),
            ("thinking", Field::Flag(true)),
            ("effort", Field::Text(EFFORT_LOW)),
        ],
        efforts: &["minimal", "low", "medium", "high"],
    },
    Provider {
        key: "custom",
        label_key: "provider.custom",
        desc_key: "provider.custom_desc",
        key_url: None,
        brand: None,
        fields: &[
            ("base_url", Field::Text("")),
            ("model", Field::Text("")),
            ("api_key", Field::Text("")),
            ("thinking", Field::Flag(false)),
            ("effort", Field::Text(EFFORT_AUTO)),
        ],
        efforts: OPENAI_EFFORTS,
    },
];

const LEGACY_FLAT_FIELDS: &[&str] = &["model", "api_key", "thinking", "effort", "base_url"];
// 遷移時判定「這家使用者填過東西」的欄位；三個都空就不留空殼。
const FILLED_MARKERS: &[&str] = &["model", "api_key", "base_url"];

/// 字串欄位的值；缺值或不是字串時為空字串。
pub(crate) fn str_field<'a>(entry: &'a Value, key: &str) -> &'a str {
    entry.get(key).and_then(Value::as_str).unwrap_or("")
}

/// 使用者是否填過這筆的任一 `FILLED_MARKERS` 欄位。
fn any_filled(entry: &Value) -> bool {
    FILLED_MARKERS
        .iter()
        .any(|k| !pystr::strip(str_field(entry, k)).is_empty())
}

/// 全部服務商，依顯示順序。
pub fn providers() -> &'static [Provider] {
    PROVIDERS
}

/// 依鍵名取服務商；不認得回 `None`。
pub fn provider(key: &str) -> Option<&'static Provider> {
    providers().iter().find(|p| p.key == key)
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
#[derive(Clone, Default, PartialEq, Eq)]
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
    /// 思考開關。
    pub thinking: bool,
    /// 思考深度（[`EFFORT_AUTO`] 或該家的等級）。
    pub effort: String,
}

// 手寫 Debug：金鑰只顯示有沒有，`{:?}` 進 log 也不會外洩（裁示 R7）
impl fmt::Debug for ServiceSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceSpec")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("has_key", &!self.api_key.is_empty())
            .field("base_url", &self.base_url)
            .field("thinking", &self.thinking)
            .field("effort", &self.effort)
            .finish()
    }
}

impl ServiceSpec {
    /// 從一筆服務（或去掉 id、名稱的 API 欄位）建立；缺的欄位取空字串或 `false`。
    pub fn from_service(service: &Value) -> Self {
        let text = |k: &str| str_field(service, k).to_owned();
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

/// 檢查連上端點所需的欄位（不含模型），回傳錯誤文案 key 列表（空＝通過）。
///
/// 取模型清單時模型欄還沒填，故與 [`validate_service`] 分開。
pub fn validate_endpoint_fields(service: &Value) -> Vec<String> {
    let text = |k: &str| str_field(service, k);
    let needs_base_url = provider(text("provider")).is_some_and(Provider::needs_base_url);
    let mut errors = Vec::new();
    if !needs_base_url && pystr::strip(text("api_key")).is_empty() {
        errors.push("error.need_api_key".to_string());
    }
    if needs_base_url && pystr::strip(text("base_url")).is_empty() {
        errors.push("error.need_base_url".to_string());
    }
    errors
}

/// 檢查一筆服務的必填欄位，回傳錯誤文案 key 列表（空＝通過）。
pub fn validate_service(service: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    if pystr::strip(str_field(service, "model")).is_empty() {
        errors.push("error.need_model".to_string());
    }
    errors.extend(validate_endpoint_fields(service));
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

/// 該用途指定的服務 id；未指定（或不是字串）回 `None`。
pub fn slot_id(cfg: &Config, slot: ServiceSlot) -> Option<&str> {
    cfg.get("service_slots")?.get(slot.key())?.as_str()
}

/// 該用途實際生效的服務：插槽未指定就跟隨預設，連預設都沒有時回第一家服務商的空白設定。
pub fn resolve(cfg: &Config, slot: ServiceSlot) -> ServiceSpec {
    let default_id = cfg.get("default_service").and_then(Value::as_str);
    match find(cfg, slot_id(cfg, slot)).or_else(|| find(cfg, default_id)) {
        Some(service) => ServiceSpec::from_service(service),
        None => {
            let fallback = &PROVIDERS[0];
            let mut blank = fallback.defaults();
            blank.insert("provider".into(), fallback.key.into());
            ServiceSpec::from_service(&Value::Object(blank))
        }
    }
}

fn ids_and_names<'a>(services: &'a [Value], key: &str) -> impl Iterator<Item = &'a str> {
    services
        .iter()
        .filter_map(move |s| s.get(key).and_then(Value::as_str))
}

pub(crate) fn new_id(services: &[Value]) -> String {
    loop {
        let candidate = format!("{:08x}", random_u64() as u32);
        if !ids_and_names(services, "id").any(|id| id == candidate) {
            return candidate;
        }
    }
}

/// 這筆的 id 是不是 `ignore_id`（Python 的 `id == ignore_id`；缺欄位、null 都當 `None`，非字串的 id 不會相等）。
fn is_ignored(id: Option<&Value>, ignore_id: Option<&str>) -> bool {
    match id {
        None | Some(Value::Null) => ignore_id.is_none(),
        Some(Value::String(id)) => Some(id.as_str()) == ignore_id,
        Some(_) => false,
    }
}

/// 撞名就補序號：第一筆無後綴，之後 `名稱 (2)`、`名稱 (3)`；`ignore_id` 那筆（編輯中的自己）不算。
///
/// 括號一律半形：它是識別用的序號，不隨介面語言換形。
pub fn unique_name(name: &str, services: &[Value], ignore_id: Option<&str>) -> String {
    let taken = |candidate: &str| {
        services
            .iter()
            .filter(|s| !is_ignored(s.get("id"), ignore_id))
            .any(|s| s.get("name").and_then(Value::as_str) == Some(candidate))
    };
    if !taken(name) {
        return name.to_string();
    }
    (2..)
        .map(|n| format!("{name} ({n})"))
        .find(|c| !taken(c))
        .unwrap_or_default()
}

/// `unique_name` 生成的名稱形狀：`base` 本身，或序號 2 起、不帶前導零的 `base (N)`。
///
/// 緊鄰 `unique_name`：改後綴格式兩處都要改。
pub fn is_auto_name(name: &str, base: &str) -> bool {
    if name == base {
        return true;
    }
    let number = name
        .strip_prefix(base)
        .and_then(|rest| rest.strip_prefix(" ("))
        .and_then(|rest| rest.strip_suffix(')'));
    // 序號從 2 起跳：(0)／(1)／前導零比較像使用者自己打的名字，寧可漏判也不誤蓋
    number.is_some_and(|n| {
        !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && !n.starts_with('0') && n != "1"
    })
}

fn service_map(p: &Provider, services: &[Value]) -> Map<String, Value> {
    let mut service = Map::new();
    service.insert("id".into(), new_id(services).into());
    service.insert(
        "name".into(),
        unique_name(&p.short_name(), services, None).into(),
    );
    service.insert("provider".into(), p.key.into());
    service.extend(p.defaults());
    service
}

/// 新的一筆服務：新 id、依服務商短名自動命名、欄位填該家的預設值。
///
/// `provider` 必須是 [`providers`] 裡的鍵。
pub fn new_service(provider: &str, services: &[Value]) -> Value {
    let p = self::provider(provider).unwrap_or_else(|| panic!("unknown provider {provider:?}"));
    Value::Object(service_map(p, services))
}

/// 服務卡片的副標：服務商短名與模型。
pub fn describe(service: &Value) -> String {
    let key = str_field(service, "provider");
    let short = provider(key).map_or_else(|| key.to_owned(), Provider::short_name);
    format!("{short} · {}", str_field(service, "model"))
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
    known_provider(entry).is_some() && any_filled(entry)
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
                repr_str(name),
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
        let Some(entry @ Value::Object(profile)) = sections
            .iter()
            .find(|(name, _)| name.as_str() == Some(p.key))
            .map(|(_, v)| v)
        else {
            continue;
        };
        if !any_filled(entry) {
            continue;
        }
        // 新 id 要一起避開促轉回來的服務：撞號會讓 default_service 落到錯的那筆
        let mut service = service_map(p, &all);
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
        let mut service = Map::new();
        service.insert("id".into(), str_field(entry, "id").into());
        service.insert("name".into(), str_field(entry, "name").into());
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
        let id = str_field(entry, "id");
        if id.is_empty() || ids_and_names(&clean, "id").any(|other| other == id) {
            let fresh = new_id(&clean);
            log(format!(
                "[config] regenerated a missing or duplicate service id -> {fresh}"
            ));
            service.insert("id".into(), fresh.into());
        }
        // 每一筆都過一次去重：分派下拉只顯示名稱，同名會讓使用者選到另一筆服務
        let name = str_field(entry, "name");
        let unique = unique_name(
            if name.is_empty() {
                p.short_name()
            } else {
                name.to_owned()
            }
            .as_str(),
            &clean,
            None,
        );
        if unique != name {
            log(format!(
                "[config] renamed a missing or duplicate service name -> {}",
                repr_str(&unique)
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

    #[test]
    fn debug_output_never_shows_the_api_key() {
        let spec = ServiceSpec {
            api_key: "sk-SECRET-KEY".into(),
            model: "m".into(),
            ..Default::default()
        };
        let text = format!("{spec:?} {spec:#?}");
        assert!(!text.contains("sk-SECRET-KEY"), "{text}");
        assert!(text.contains("has_key: true"), "{text}");
    }

    #[test]
    fn resolve_returns_the_blank_first_provider_without_services() {
        let spec = resolve(&Config::new(), ServiceSlot::Incoming);
        assert_eq!(spec.provider, "openai");
        assert_eq!(spec.model, "");
        assert!(!spec.thinking);
    }
}
