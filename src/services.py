"""翻譯服務：服務商的欄位表與顯示資料，以及表單必填欄位的判定。

「服務」是使用者建立的一筆具名設定 —— 某家服務商加上金鑰、模型等欄位。
"""
import copy
import uuid
from dataclasses import dataclass

from src.i18n import t
from src.log import log

# 每家服務商有哪些欄位與預設值：官方端點的網址寫死在 translator，Claude 不吃 thinking
# 開關而是 effort。這張表是服務商清單與欄位的單一真實來源。
API_PROFILE_FIELDS: dict[str, dict] = {
    "openai": {"model": "", "api_key": "", "thinking": False},
    "claude": {"model": "", "api_key": "", "effort": "auto"},
    "custom": {"base_url": "", "model": "", "api_key": "", "thinking": False},
}

API_PROVIDERS = tuple(API_PROFILE_FIELDS)

# 這一版認不得 provider 的服務搬去的頂層鍵。丟棄等於使用者跑過一次舊版就失去金鑰，
# 而隔離區裡的東西日後由認得它的版本自動搬回 services。
UNSUPPORTED_SERVICES = "unsupported_services"

# Claude 的思考深度：auto＝不帶參數、由模型自行決定；low＝壓到最低。
# Claude 沒有「完全不思考」這個選項，故意不與另兩家的 thinking 開關共用欄位名。
EFFORT_AUTO = "auto"
EFFORT_LOW = "low"
API_EFFORTS = (EFFORT_AUTO, EFFORT_LOW)


def needs_base_url(provider: str) -> bool:
    """這家服務商要不要自己填端點網址（官方端點的網址寫死在 translator）。"""
    return "base_url" in API_PROFILE_FIELDS[provider]


@dataclass(frozen=True)
class Provider:
    """服務商的顯示資料。該畫哪些欄位一律問 API_PROFILE_FIELDS（has_field）。"""
    key: str
    label_key: str
    desc_key: str
    key_url: str | None = None
    brand: str | None = None

    def has_field(self, name: str) -> bool:
        return name in API_PROFILE_FIELDS[self.key]

    @property
    def needs_base_url(self) -> bool:
        return needs_base_url(self.key)

    @property
    def short_name(self) -> str:
        """新建一筆服務時的預設名稱；品牌名不進語言檔，只有自訂端點要翻譯。"""
        return self.brand or t(f"provider.{self.key}_short")


PROVIDERS: dict[str, Provider] = {p.key: p for p in (
    Provider(key="openai", label_key="provider.openai",
             desc_key="provider.openai_desc", brand="ChatGPT",
             key_url="https://platform.openai.com/api-keys"),
    Provider(key="claude", label_key="provider.claude",
             desc_key="provider.claude_desc", brand="Claude",
             key_url="https://console.anthropic.com/settings/keys"),
    Provider(key="custom", label_key="provider.custom",
             desc_key="provider.custom_desc"),
)}


def validate_endpoint_fields(api: dict) -> list[str]:
    """檢查連上端點所需的欄位（不含模型），回傳錯誤文案 key 列表（空＝通過）。
    取模型清單時模型欄本來就還沒填，故與 validate_service 分開。"""
    errors = []
    provider = PROVIDERS[api["provider"]]
    if not provider.needs_base_url and not api["api_key"].strip():
        errors.append("error.need_api_key")
    if provider.needs_base_url and not api["base_url"].strip():
        errors.append("error.need_base_url")
    return errors


def validate_service(api: dict) -> list[str]:
    """檢查一筆服務的必填欄位，回傳錯誤文案 key 列表（空＝通過）。"""
    errors = [] if api["model"].strip() else ["error.need_model"]
    return errors + validate_endpoint_fields(api)


# 用途插槽：三個用途可各自指定服務，None＝跟隨預設。命名用方向而非語言。
SLOT_INCOMING = "incoming"
SLOT_OUTGOING = "outgoing"
SLOT_REGION = "region"
SLOTS = (SLOT_INCOMING, SLOT_OUTGOING, SLOT_REGION)

_ID_LENGTH = 8


def new_id(services: list[dict]) -> str:
    """產生一個不與現有服務相撞的 id。"""
    taken = {s.get("id") for s in services}
    while True:
        candidate = uuid.uuid4().hex[:_ID_LENGTH]
        if candidate not in taken:
            return candidate


def unique_name(name: str, services: list[dict], ignore_id: str | None = None) -> str:
    """撞名就補序號：第一筆無後綴，之後 `ChatGPT (2)`、`ChatGPT (3)`。
    括號一律半形 —— 它是識別用的序號，不隨介面語言換形。"""
    # 取值與隔壁的 new_id 一致用 .get()：隔離區原樣保留的項目不保證有 name，
    # 促轉回來後會直接進到這裡，漏掉的那筆貢獻 None，不會與真實名稱相撞
    taken = {s.get("name") for s in services if s.get("id") != ignore_id}
    if name not in taken:
        return name
    number = 2
    while f"{name} ({number})" in taken:
        number += 1
    return f"{name} ({number})"


def is_auto_name(name: str, base: str) -> bool:
    """`name` 是否可能是 `unique_name(base, ...)` 生成的形狀（緊鄰放置：改後綴格式兩處都要改）。"""
    if name == base:
        return True
    prefix = f"{base} ("
    if not (name.startswith(prefix) and name.endswith(")")):
        return False
    number = name[len(prefix):-1]
    if not (number.isascii() and number.isdigit()):
        return False
    # 序號從 2 起跳、不帶前導零 —— 對齊 unique_name 的值域，不然 "(1)" 這種使用者可能
    # 自己打的名字會被誤判成自動產生
    return number[0] != "0" and int(number) >= 2


def new_service(provider: str, services: list[dict]) -> dict:
    """新的一筆服務：新 id、依服務商短名自動命名、欄位填該家的預設值。"""
    return {"id": new_id(services),
            "name": unique_name(PROVIDERS[provider].short_name, services),
            "provider": provider,
            **copy.deepcopy(API_PROFILE_FIELDS[provider])}


def find(cfg: dict, service_id: str | None) -> dict | None:
    """依 id 取服務本體（不是副本）；找不到回 None。"""
    if service_id is None:
        return None
    return next((s for s in cfg["services"] if s["id"] == service_id), None)


def resolve(cfg: dict, slot: str) -> dict:
    """該用途實際生效的服務，攤平成 Translator 吃的形狀（含 provider，不含 id／name）。
    插槽未指定就跟隨預設；連預設都沒有（精靈尚未完成）時回一份空白設定。"""
    service = find(cfg, cfg["service_slots"].get(slot)) or find(cfg, cfg["default_service"])
    if service is None:
        fallback = API_PROVIDERS[0]
        return {"provider": fallback, **copy.deepcopy(API_PROFILE_FIELDS[fallback])}
    return {key: value for key, value in copy.deepcopy(service).items()
            if key not in ("id", "name")}


def describe(service: dict) -> str:
    """服務卡片的副標：服務商短名與模型。"""
    return f"{PROVIDERS[service['provider']].short_name} · {service['model']}"


_LEGACY_FLAT_FIELDS = {key for fields in API_PROFILE_FIELDS.values() for key in fields}
# 遷移時判定「這家使用者填過東西」的欄位；三個都空就不留空殼。
_FILLED_MARKERS = ("model", "api_key", "base_url")


def _unflatten_legacy(api: dict) -> dict:
    """最舊的 api 區塊是扁平的：設定欄位與 provider 並排。整組搬進所屬服務商的子區塊，
    認得的服務商只留它表上的欄位；連 provider 都沒有的一律視為自訂端點。"""
    provider = api.get("provider", "custom")
    # 認不得的 provider 沒有欄位表可篩，篩了就是丟掉金鑰 —— 整組留著給隔離區接手
    fields = API_PROFILE_FIELDS.get(provider)
    profile = {key: value for key, value in api.items()
               if key != "provider" and (fields is None or key in fields)}
    log(f"[config] unflattened the legacy api block into provider={provider} "
        f"(fields={sorted(profile)})")
    return {"provider": provider, provider: profile}


def _quarantine_unknown_api_profiles(cfg: dict, api: dict) -> None:
    """api 區塊裡這一版不認得的 provider 子區塊原樣搬進隔離區。
    服務清單那一側早就這樣做了，只有這一側照丟的話，舊版寫下的金鑰會在遷移時蒸發。"""
    for provider, profile in api.items():
        if provider == "provider" or provider in API_PROFILE_FIELDS:
            continue
        if not isinstance(profile, dict) or not profile:
            continue
        cfg[UNSUPPORTED_SERVICES] = [*quarantined(cfg),
                                     {"provider": provider, **profile}]
        log(f"[config] quarantined the api block's unknown provider {provider!r} into "
            f"{UNSUPPORTED_SERVICES} (has_key={bool(profile.get('api_key'))}); a build "
            f"that supports it will restore it")


def _migrate_api_block(cfg: dict, migrated_before: bool) -> bool:
    """把舊的 api 區塊（扁平或 per-provider）換成服務清單，並移除該區塊。
    `migrated_before`＝這份設定本來就有這一版認得、而且填過欄位的服務，也就是遷移過了。
    呼叫前 _quarantine_unknown_services 必須先跑過：清單裡每一筆都得是 dict。"""
    api = cfg.pop("api", None)
    if not isinstance(api, dict):
        return False
    if any(key in api for key in _LEGACY_FLAT_FIELDS):
        api = _unflatten_legacy(api)
    # 隔離要在早退之前：丟得掉的只有這一版看得懂、而且確定重複的東西
    _quarantine_unknown_api_profiles(cfg, api)
    if migrated_before:
        log("[config] dropped a leftover api block; the service list already exists")
        return True
    kept = _as_entries(cfg.get("services"))
    services: list[dict] = []
    default_id = None
    for provider in API_PROVIDERS:
        profile = api.get(provider)
        if not isinstance(profile, dict):
            continue
        if not any(isinstance(profile.get(key), str) and profile.get(key).strip()
                   for key in _FILLED_MARKERS):
            continue
        # 新 id 要一起避開促轉回來的服務：撞號會讓 default_service 落到錯的那筆
        service = new_service(provider, [*kept, *services])
        service.update({key: value for key, value in profile.items()
                        if key in API_PROFILE_FIELDS[provider]})
        services.append(service)
        if api.get("provider") == provider:
            default_id = service["id"]
    cfg["services"] = [*kept, *services]
    # api 區塊的 provider 具權威性：對不上時也要退回這個區塊遷移出來的那批，
    # 促轉回來的服務與它無關
    fallback = services[0]["id"] if services else (kept[0].get("id") if kept else None)
    cfg["default_service"] = default_id or fallback
    log(f"[config] migrated the api block into {len(services)} service(s); "
        f"default={cfg['default_service']}")
    if kept:
        log(f"[config] the api block was migrated next to {len(kept)} service(s) already "
            f"in the list; an account present in both stays as two entries")
    return True


def _text(value) -> str:
    """字串欄位的取值：型別不符一律當成沒填（手改的 config.json 什麼都可能塞）。"""
    return value if isinstance(value, str) else ""


def _profile_field(entry: dict, provider: str, key: str, default):
    """一個服務商欄位的取值：型別與預設值不同就退回預設。
    JSON 是使用者手改的，型別不檢查會讓後面的 `.strip()` 在開窗前把程式帶掉。"""
    value = entry.get(key, default)
    # 預設值是 None 的欄位不做型別比對：None 對不上任何真值，會把整欄（含金鑰）清掉
    if default is not None and type(value) is not type(default):
        log(f"[config] {provider} service field {key} has type "
            f"{type(value).__name__}; using the default")
        return default
    return value


def _as_entries(value) -> list:
    """服務清單欄位的取值：手改成 list 以外的型別時整個當成一筆，不替使用者刪 ——
    最可能的形狀是貼歪的裸金鑰字串。不是 dict 的那筆隨後會被隔離區接住。"""
    if isinstance(value, list):
        return value
    return [] if value is None else [value]


def _is_known(entry) -> bool:
    """這一筆是不是這一版看得懂的服務（是 dict 且 provider 認得）。"""
    provider = entry.get("provider") if isinstance(entry, dict) else None
    return isinstance(provider, str) and provider in API_PROFILE_FIELDS


def _is_migrated_service(entry) -> bool:
    """這一筆算不算「api 區塊已經遷移過」的證據：provider 認得、而且填過至少一個欄位。
    空殼也算證據的話，舊 api 區塊會被當成殘留丟掉，裡面的金鑰跟著消失。"""
    return _is_known(entry) and any(_text(entry.get(key)).strip()
                                    for key in _FILLED_MARKERS)


def quarantined(cfg: dict) -> list:
    """隔離區現有的內容；取值與 services 一致，非 list 的原值整個當成一筆。"""
    return _as_entries(cfg.get(UNSUPPORTED_SERVICES))


def _normalize_containers(cfg: dict) -> bool:
    """開場先把 services 與隔離區各正規化成 list：之後每一步看到的都保證是 list，
    手改成非 list 的原值（最可能是漏了中括號或貼歪的金鑰）不會在某一步無聲消失。"""
    changed = False
    services = cfg.get("services")
    if not isinstance(services, list):
        cfg["services"] = _as_entries(services)
        # 缺鍵或 None 只是補一個空清單，沒有東西被保住，不算變動也不值得留一行 log
        if services is not None:
            log(f"[config] services is a {type(services).__name__}, not a list; kept the "
                f"value as a single entry")
            changed = True
    parked = cfg.get(UNSUPPORTED_SERVICES)
    if parked is not None and not isinstance(parked, list):
        cfg[UNSUPPORTED_SERVICES] = _as_entries(parked)
        log(f"[config] {UNSUPPORTED_SERVICES} is a {type(parked).__name__}, not a list; "
            f"kept the value as a single entry")
        changed = True
    return changed


def _promote_known_services(cfg: dict) -> bool:
    """隔離區裡這一版已經認得的服務搬回 services，接著照常過一次補值與去重。
    新增服務商的版本靠這段自動接回使用者的設定，不必知道隔離區的存在。"""
    parked = quarantined(cfg)
    promoted = [entry for entry in parked if _is_known(entry)]
    if not promoted:
        return False
    cfg[UNSUPPORTED_SERVICES] = [entry for entry in parked if not _is_known(entry)]
    cfg["services"] = [*_as_entries(cfg.get("services")), *promoted]
    for entry in promoted:
        log(f"[config] promoted a quarantined {entry['provider']} service back into the "
            f"service list (has_key={bool(entry.get('api_key'))})")
    return True


def _quarantine_unknown_services(cfg: dict) -> bool:
    """認不得 provider 的服務原樣搬進隔離區，一個欄位都不動 —— 這一版看不懂它，
    也就沒資格改寫它。搬走之後 _sanitize_services 只會看到認得的服務。"""
    services = _as_entries(cfg.get("services"))
    unknown = [entry for entry in services if not _is_known(entry)]
    if not unknown:
        return False
    cfg["services"] = [entry for entry in services if _is_known(entry)]
    cfg[UNSUPPORTED_SERVICES] = [*quarantined(cfg), *unknown]
    for entry in unknown:
        provider = entry.get("provider") if isinstance(entry, dict) else None
        # 形狀讀不懂的那些沒有任何一版認得，log 不能拿「日後會搬回來」誤導使用者
        if isinstance(provider, str):
            log(f"[config] quarantined a service with unknown provider {provider!r} into "
                f"{UNSUPPORTED_SERVICES} (has_key={bool(entry.get('api_key'))}); a build "
                f"that supports it will restore it")
        else:
            log(f"[config] quarantined an entry this build cannot read into "
                f"{UNSUPPORTED_SERVICES} (type={type(entry).__name__}); it is kept "
                f"verbatim, nothing will restore it, delete it yourself if it is junk")
    return True


def _sanitize_services(cfg: dict) -> bool:
    """逐筆補齊欄位、刪掉過期欄位、補上缺漏或重複的 id 與名稱。
    認不得 provider 的服務已由 _quarantine_unknown_services 先搬走，容器也已由
    _normalize_containers 正規化成 list。"""
    before = cfg["services"]
    clean: list[dict] = []
    for entry in before:
        provider = entry["provider"]
        service = {"id": _text(entry.get("id")),
                   "name": _text(entry.get("name")),
                   "provider": provider,
                   **{key: _profile_field(entry, provider, key, default)
                      for key, default in API_PROFILE_FIELDS[provider].items()}}
        stale = sorted(key for key in entry if key not in service)
        if stale:
            log(f"[config] dropped stale fields on a {provider} service: "
                f"{', '.join(stale)}")
        if not service["id"] or any(service["id"] == s["id"] for s in clean):
            service["id"] = new_id(clean)
            log(f"[config] regenerated a missing or duplicate service id "
                f"-> {service['id']}")
        # 每一筆都過一次去重：分派下拉只顯示名稱，同名會讓使用者選到另一筆服務
        name = unique_name(service["name"] or PROVIDERS[provider].short_name, clean)
        if name != service["name"]:
            log(f"[config] renamed a missing or duplicate service name -> {name!r}")
        service["name"] = name
        clean.append(service)
    cfg["services"] = clean
    return clean != before


def _sanitize_pointers(cfg: dict) -> bool:
    """讓 default_service 與三個插槽只指向存在的服務。"""
    ids = {s["id"] for s in cfg["services"]}
    changed = False
    default = cfg.get("default_service")
    # 先確認是字串再比對：手改成 list 之類的東西，`in` 會直接拋 TypeError
    wanted = default if isinstance(default, str) and default in ids else (
        cfg["services"][0]["id"] if cfg["services"] else None)
    if wanted != default:
        log(f"[config] default_service {default!r} is unknown; using {wanted!r}")
        cfg["default_service"] = wanted
        changed = True
    slots = cfg.get("service_slots")
    slots = slots if isinstance(slots, dict) else {}
    clean = {}
    for slot in SLOTS:
        value = slots.get(slot)
        if value is not None and not (isinstance(value, str) and value in ids):
            log(f"[config] service_slots.{slot} points at unknown service {value!r}; "
                f"following the default instead")
            value = None
        clean[slot] = value
    if clean != slots:
        changed = True
    cfg["service_slots"] = clean
    return changed


def normalize(cfg: dict) -> bool:
    """就地遷移舊格式、補齊欄位、清掉指不到的參照；回傳是否有變動。
    有變動代表呼叫端該把整理後的結果寫回 config.json。"""
    changed = _normalize_containers(cfg)
    # 遷移與否的證據要在促轉之前取樣：促轉之後清單裡會混進與 api 區塊無關的服務
    migrated_before = any(_is_migrated_service(entry) for entry in cfg["services"])
    changed = _promote_known_services(cfg) or changed
    changed = _quarantine_unknown_services(cfg) or changed
    changed = _migrate_api_block(cfg, migrated_before) or changed
    changed = _sanitize_services(cfg) or changed
    return _sanitize_pointers(cfg) or changed
