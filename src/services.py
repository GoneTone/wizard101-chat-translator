"""翻譯服務：服務商的欄位表與顯示資料，以及表單必填欄位的判定。

「服務」是使用者建立的一筆具名設定 —— 某家服務商加上金鑰、模型等欄位。
"""
from dataclasses import dataclass

from src.i18n import t

# 每家服務商有哪些欄位與預設值：官方端點的網址寫死在 translator，Claude 不吃 thinking
# 開關而是 effort。這張表是服務商清單與欄位的單一真實來源。
API_PROFILE_FIELDS: dict[str, dict] = {
    "openai": {"model": "", "api_key": "", "thinking": False},
    "claude": {"model": "", "api_key": "", "effort": "auto"},
    "custom": {"base_url": "", "model": "", "api_key": "", "thinking": False},
}

API_PROVIDERS = tuple(API_PROFILE_FIELDS)

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
    Provider(key="openai", label_key="provider.openai", brand="ChatGPT",
             key_url="https://platform.openai.com/api-keys"),
    Provider(key="claude", label_key="provider.claude", brand="Claude",
             key_url="https://console.anthropic.com/settings/keys"),
    Provider(key="custom", label_key="provider.custom"),
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
