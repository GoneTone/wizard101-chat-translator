"""翻譯服務資料層測試。"""
from src import i18n
from src.config import is_configured
from src.services import (
    API_PROFILE_FIELDS,
    API_PROVIDERS,
    PROVIDERS,
    needs_base_url,
    validate_endpoint_fields,
    validate_service,
)
from tests.config_helpers import configured_cfg


def test_configured_cfg_passes_the_completeness_check():
    assert is_configured(configured_cfg())
    assert is_configured(configured_cfg("openai"))
    assert is_configured(configured_cfg("claude"))


def test_configured_cfg_returns_an_independent_copy():
    first = configured_cfg()
    first["api"]["custom"]["model"] = "mutated"
    assert configured_cfg()["api"]["custom"]["model"] == "m"


def test_every_provider_has_ui_metadata():
    assert set(PROVIDERS) == set(API_PROVIDERS)
    assert needs_base_url("custom")
    assert not needs_base_url("openai")


def test_short_names_are_brands_except_the_custom_endpoint():
    before = i18n.current_language()
    try:
        i18n.set_language("en-US")
        assert PROVIDERS["openai"].short_name == "ChatGPT"
        assert PROVIDERS["claude"].short_name == "Claude"
        # 自訂端點沒有品牌名，短名要走語言檔（此處只確認它被翻過，不釘字面）
        assert PROVIDERS["custom"].short_name != "provider.custom_short"
    finally:
        i18n.set_language(before)


def test_validate_service_reports_every_missing_required_field():
    errors = validate_service({"provider": "openai", "model": "", "api_key": ""})
    assert set(errors) == {"error.need_model", "error.need_api_key"}


def test_validate_endpoint_fields_ignores_the_model():
    assert validate_endpoint_fields(
        {"provider": "custom", "base_url": "http://x", "model": ""}) == []


def test_profile_fields_cover_what_the_translator_takes():
    assert set(API_PROFILE_FIELDS["claude"]) == {"model", "api_key", "effort"}
    assert "base_url" not in API_PROFILE_FIELDS["openai"]
