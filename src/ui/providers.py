"""服務商的 UI 資料與 API 表單的純驗證邏輯（不碰 tkinter）。
該畫哪些欄位一律問 config 的欄位表（API_PROFILE_FIELDS），這裡只補顯示名稱與
申請金鑰的連結，以及「哪些欄位必填」的判定。
"""
from dataclasses import dataclass

from src.config import API_PROFILE_FIELDS, needs_base_url


@dataclass(frozen=True)
class Provider:
    """服務商的 UI 資料。該畫哪些欄位一律問 config 的欄位表（has_field），
    這裡只補純 UI 的部分：顯示名稱與申請金鑰的連結。"""
    key: str
    label_key: str
    key_url: str | None = None

    def has_field(self, name: str) -> bool:
        return name in API_PROFILE_FIELDS[self.key]

    @property
    def needs_base_url(self) -> bool:
        return needs_base_url(self.key)


PROVIDERS: dict[str, Provider] = {p.key: p for p in (
    # 前兩家是品牌名，不進語言檔；只有「自訂端點」需要翻譯。
    Provider(key="openai", label_key="provider.openai",
             key_url="https://platform.openai.com/api-keys"),
    Provider(key="claude", label_key="provider.claude",
             key_url="https://console.anthropic.com/settings/keys"),
    Provider(key="custom", label_key="provider.custom"),
)}


def validate_endpoint_fields(api: dict) -> list[str]:
    """檢查連上端點所需的欄位（不含模型），回傳錯誤文案 key 列表（空＝通過）。
    取模型清單時模型欄本來就還沒填，故與 validate_api_form 分開。"""
    errors = []
    provider = PROVIDERS[api["provider"]]
    if not provider.needs_base_url and not api["api_key"].strip():
        errors.append("error.need_api_key")
    if provider.needs_base_url and not api["base_url"].strip():
        errors.append("error.need_base_url")
    return errors


def validate_api_form(api: dict) -> list[str]:
    """檢查 API 表單必填欄位，回傳錯誤文案 key 列表（空＝通過）。"""
    errors = [] if api["model"].strip() else ["error.need_model"]
    return errors + validate_endpoint_fields(api)
