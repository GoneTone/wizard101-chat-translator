"""fields 純邏輯測試：表單驗證、錯誤文案、熱鍵字串。"""
from src.translator import TranslatorConfigError, TranslatorOffline
from src.ui.fields import (
    PROVIDERS, friendly_error, hotkey_from_event, validate_api_form,
)


def test_providers_metadata():
    assert set(PROVIDERS) == {"openai", "claude", "custom"}
    assert PROVIDERS["custom"].needs_base_url
    assert not PROVIDERS["openai"].needs_base_url
    assert PROVIDERS["openai"].models  # 有預設模型清單
    assert PROVIDERS["claude"].models[0] == "claude-opus-5"


def test_validate_requires_model():
    errs = validate_api_form({"provider": "openai", "model": "", "api_key": "k",
                              "base_url": "", "thinking": False})
    assert any("模型" in e for e in errs)


def test_validate_requires_key_for_official_providers():
    errs = validate_api_form({"provider": "claude", "model": "claude-opus-5",
                              "api_key": "", "base_url": "", "thinking": False})
    assert any("金鑰" in e for e in errs)


def test_validate_requires_base_url_for_custom_only():
    api = {"provider": "custom", "model": "m", "api_key": "", "base_url": "",
           "thinking": False}
    assert any("網址" in e for e in validate_api_form(api))
    api["base_url"] = "http://127.0.0.1:8000"
    assert validate_api_form(api) == []  # custom 不需金鑰


def test_friendly_error_messages():
    assert "金鑰" in friendly_error(TranslatorConfigError("HTTP 401", status=401))
    assert "模型" in friendly_error(TranslatorConfigError("HTTP 404", status=404))
    assert "連線" in friendly_error(TranslatorOffline("refused"))


def test_hotkey_from_event():
    assert hotkey_from_event("space", 0x4) == "ctrl+space"
    assert hotkey_from_event("F8", 0) == "f8"
    assert hotkey_from_event("x", 0x4 | 0x20000) == "ctrl+alt+x"
    assert hotkey_from_event("Control_L", 0x4) is None  # 純修飾鍵不成立
