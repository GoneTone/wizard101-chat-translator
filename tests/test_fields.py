"""fields 純邏輯測試：表單驗證、錯誤文案、熱鍵字串。"""
from src.translator import TranslatorConfigError, TranslatorOffline
from src.ui.fields import (
    PROVIDERS, ApiFields, friendly_error, validate_api_form,
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




def _initial(provider="openai", model="", base_url="", thinking=False):
    return {"provider": provider, "api_key": "", "model": model,
            "base_url": base_url, "thinking": thinking}


def test_switch_provider_resets_model_not_in_new_list(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-sol"))
    fields._provider.set("claude")
    fields._rebuild_fields()
    assert fields.get_values()["model"] == PROVIDERS["claude"].models[0]


def test_switch_provider_keeps_model_if_still_valid(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-terra"))
    fields._provider.set("openai")
    fields._rebuild_fields()
    assert fields.get_values()["model"] == "gpt-5.6-terra"


def test_switch_to_custom_keeps_users_model_input(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-sol"))
    fields._provider.set("custom")
    fields._rebuild_fields()
    assert fields.get_values()["model"] == "gpt-5.6-sol"  # custom 不清空使用者原輸入


def test_switch_provider_clears_test_result_label(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-sol"))
    fields._show_test_result(True, "連線成功　範例：hi")
    assert fields._test_result.cget("text") != ""
    fields._provider.set("claude")
    fields._rebuild_fields()
    assert fields._test_result.cget("text") == ""
