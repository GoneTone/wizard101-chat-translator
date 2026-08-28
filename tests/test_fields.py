"""fields 純邏輯測試：表單驗證、錯誤文案、熱鍵字串。"""
import gc
import tkinter as tk

import pytest

from src.i18n import t
from src.translator import (
    TranslatorConfigError, TranslatorNoModelList, TranslatorOffline,
)
from src.ui.fields import (
    PROVIDERS, ApiFields, ModelField, filter_models, friendly_error,
    validate_api_form, validate_endpoint_fields,
)


@pytest.fixture
def offscreen(root):
    """真的顯示出來的小視窗：下拉清單只有在螢幕內的視窗上才會展開（Windows 行為）。"""
    win = tk.Toplevel(root)
    win.geometry("360x90+0+0")
    win.update()
    yield win
    win.destroy()
    # 趁 root 還在時回收視窗底下的 Tk 變數：留到之後才 GC，Variable.__del__ 會
    # 對著已銷毀的視窗呼叫 Tk 而噴 unraisable exception。
    gc.collect()


def test_providers_metadata():
    assert set(PROVIDERS) == {"openai", "claude", "custom"}
    assert PROVIDERS["custom"].needs_base_url
    assert not PROVIDERS["openai"].needs_base_url


def test_validate_requires_model():
    errs = validate_api_form({"provider": "openai", "model": "", "api_key": "k",
                              "base_url": "", "thinking": False})
    assert "error.need_model" in errs


def test_validate_requires_key_for_official_providers():
    errs = validate_api_form({"provider": "claude", "model": "claude-opus-5",
                              "api_key": "", "base_url": "", "thinking": False})
    assert "error.need_api_key" in errs


def test_validate_requires_base_url_for_custom_only():
    api = {"provider": "custom", "model": "m", "api_key": "", "base_url": "",
           "thinking": False}
    assert "error.need_base_url" in validate_api_form(api)
    api["base_url"] = "http://127.0.0.1:8000"
    assert validate_api_form(api) == []  # custom 不需金鑰


def test_friendly_error_messages():
    assert friendly_error(TranslatorConfigError("HTTP 401", status=401)) == \
        ("error.bad_key", {})
    assert friendly_error(TranslatorConfigError("HTTP 404", status=404)) == \
        ("error.model_not_found", {})
    assert friendly_error(TranslatorConfigError("HTTP 500", status=500)) == \
        ("error.api_http", {"status": 500})
    assert friendly_error(TranslatorOffline("refused")) == ("error.offline", {})




def _initial(provider="openai", model="", base_url="", thinking=False):
    return {"provider": provider, "api_key": "", "model": model,
            "base_url": base_url, "thinking": thinking}


def test_switch_provider_clears_model(root):
    # 模型 ID 跨服務商不通用，留著上一家的值只會誤導
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-sol"))
    fields._provider.set("claude")
    fields._rebuild_fields()
    assert fields.get_values()["model"] == ""


def test_rebuild_without_switching_keeps_model(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-terra"))
    fields._rebuild_fields()
    assert fields.get_values()["model"] == "gpt-5.6-terra"


def test_switch_provider_clears_fetched_model_list(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-sol"))
    fields._model_field.show_models(["gpt-5.6-sol", "gpt-5.6-luna"])
    fields._provider.set("custom")
    fields._rebuild_fields()
    assert fields._model_field.options() == []


def test_switch_provider_clears_test_result_label(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-sol"))
    fields._show_test_result(True, "連線成功　範例：hi")
    assert fields._test_result.cget("text") != ""
    fields._provider.set("claude")
    fields._rebuild_fields()
    assert fields._test_result.cget("text") == ""


def test_filter_models_is_case_insensitive_substring():
    models = ["Qwen3-32B", "gemma-3-27b", "llama-4"]
    assert filter_models(models, "qwen") == ["Qwen3-32B"]
    assert filter_models(models, "3") == ["Qwen3-32B", "gemma-3-27b"]


def test_filter_models_blank_query_returns_all():
    models = ["a", "b"]
    assert filter_models(models, "  ") == models


def test_validate_endpoint_fields_ignores_model():
    # 取模型清單前只需端點資訊，模型欄本來就還沒填
    assert validate_endpoint_fields({"provider": "custom", "model": "",
                                     "api_key": "", "base_url": "http://x",
                                     "thinking": False}) == []
    errs = validate_endpoint_fields({"provider": "openai", "model": "", "api_key": "",
                                     "base_url": "", "thinking": False})
    assert "error.need_api_key" in errs


def test_model_field_shows_fetched_models(root):
    fields = ApiFields(root, _initial(provider="custom", base_url="http://x"))
    fields._model_field.show_models(["gemma3", "qwen3"])
    assert fields._model_field.options() == ["gemma3", "qwen3"]
    assert "2" in fields._model_field.status()


def test_model_field_unsupported_endpoint_hints_manual_input(root):
    fields = ApiFields(root, _initial(provider="custom", base_url="http://x"))
    fields._model_field.show_error(TranslatorNoModelList("HTTP 404"))
    assert fields._model_field.status() == t("hint.model_no_list")
    assert fields._model_field.options() == []


def test_model_field_error_uses_friendly_message(root):
    fields = ApiFields(root, _initial(provider="custom", base_url="http://x"))
    fields._model_field.show_error(TranslatorOffline("refused"))
    assert fields._model_field.status() == t("error.offline")


def test_model_field_typing_filters_fetched_options(root):
    fields = ApiFields(root, _initial(provider="custom", base_url="http://x"))
    fields._model_field.show_models(["gemma3", "qwen3-32b", "qwen3-8b"])
    fields._model.set("qwen")
    fields._model_field.refresh_options()
    assert fields._model_field.options() == ["qwen3-32b", "qwen3-8b"]
    fields._model.set("")
    fields._model_field.refresh_options()
    assert fields._model_field.options() == ["gemma3", "qwen3-32b", "qwen3-8b"]


def _model_field(parent):
    field = ModelField(parent, tk.StringVar(), lambda: _initial(provider="custom",
                                                               base_url="http://x"))
    field.pack(fill="x")
    parent.update()
    return field


def test_model_field_hint_aligns_with_input(offscreen):
    field = _model_field(offscreen)
    assert field._status.winfo_x() == field._combo.winfo_x()


def test_model_field_posted_dropdown_keeps_no_grab(offscreen):
    # 原生 popdown 會 grab 住滑鼠：點回輸入框改關鍵字就把清單收起來
    field = _model_field(offscreen)
    field.show_models(["alpha", "beta"])
    field.post_options()
    offscreen.update()
    assert field.is_posted()
    assert not offscreen.tk.call("grab", "current")


def test_model_field_unpost_closes_dropdown(offscreen):
    field = _model_field(offscreen)
    field.show_models(["alpha", "beta"])
    field.post_options()
    field.unpost_options()
    offscreen.update()
    assert not field.is_posted()


def test_model_field_dropdown_closes_on_outside_click(offscreen):
    field = _model_field(offscreen)
    elsewhere = tk.Button(offscreen, text="別處")
    elsewhere.pack()
    field.show_models(["alpha", "beta"])
    field.post_options()
    offscreen.update()
    assert field.is_posted()
    elsewhere.event_generate("<Button-1>", x=1, y=1)
    offscreen.update()
    assert not field.is_posted()


def test_model_field_status_says_custom_name_is_allowed(root):
    # 抓到清單後也要讓使用者知道清單外的模型名稱一樣能自己打
    fields = ApiFields(root, _initial(provider="custom", base_url="http://x"))
    fields._model_field.show_models(["gemma3", "qwen3"])
    assert "自行輸入" in fields._model_field.status()


def test_model_field_unpost_after_destroy_is_safe(offscreen):
    # 關窗時輸入框的 FocusOut 會排一次延遲收合，那時元件可能已經沒了
    field = _model_field(offscreen)
    field.destroy()
    field.unpost_options()
    field._unpost_if_left()


def _listbox_items(field):
    return list(field.tk.call(field._listbox(), "get", 0, "end"))


def test_posted_dropdown_reloads_while_typing(offscreen):
    # ttk 只在展開當下填一次清單內容：邊打字邊篩選必須自己重填
    field = _model_field(offscreen)
    field.show_models(["alpha", "beta"])
    field.post_options()
    offscreen.update()
    assert _listbox_items(field) == ["alpha", "beta"]
    field._var.set("be")
    field.refresh_options()
    offscreen.update()
    assert _listbox_items(field) == ["beta"]


def test_dropdown_closes_when_nothing_matches(offscreen):
    # 沒有相符項目時清單只會剩一個空白小框
    field = _model_field(offscreen)
    field.show_models(["alpha", "beta"])
    field.post_options()
    offscreen.update()
    field._var.set("zzz")
    field.refresh_options()
    offscreen.update()
    assert not field.is_posted()
