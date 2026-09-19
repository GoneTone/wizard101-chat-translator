"""fields 純邏輯測試：表單驗證、錯誤文案、模型欄位。"""
import gc
import tkinter as tk

import pytest

from src.i18n import t
from src.services import API_PROFILE_FIELDS, validate_endpoint_fields, validate_service
from src.translation.translator import (
    TranslatorConfigError,
    TranslatorNoModelList,
    TranslatorOffline,
)
from src.ui import form as form_module
from src.ui.form import friendly_error
from src.ui.model_field import ModelField, filter_models


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


def test_provider_labels_are_translated():
    from src import i18n
    from src.services import PROVIDERS

    before = i18n.current_language()
    try:
        i18n.set_language("en-US")
        assert t(PROVIDERS["custom"].label_key) == "Custom endpoint (OpenAI-compatible)"
        i18n.set_language("zh-CN")
        assert t(PROVIDERS["custom"].label_key) == "自定义端点（OpenAI API 兼容）"
    finally:
        i18n.set_language(before)


def test_common_languages_include_english():
    from src.ui.fields import COMMON_LANGUAGES

    assert "English" in COMMON_LANGUAGES


def test_ui_language_field_round_trips_language_code(root):
    from src.ui.fields import UiLanguageField

    field = UiLanguageField(root, "zh-TW")
    assert field.value() == "zh-TW"
    field.set_value("en-US")
    assert field.value() == "en-US"


def test_validate_requires_model():
    errs = validate_service({"provider": "openai", "model": "", "api_key": "k",
                              "base_url": "", "thinking": False})
    assert "error.need_model" in errs


def test_validate_requires_key_for_official_providers():
    errs = validate_service({"provider": "claude", "model": "claude-opus-5",
                              "api_key": "", "base_url": "", "thinking": False})
    assert "error.need_api_key" in errs


def test_validate_requires_base_url_for_custom_only():
    api = {"provider": "custom", "model": "m", "api_key": "", "base_url": "",
           "thinking": False}
    assert "error.need_base_url" in validate_service(api)
    api["base_url"] = "http://127.0.0.1:8000"
    assert validate_service(api) == []  # custom 不需金鑰


def test_friendly_error_guesses_from_status_when_api_gave_no_message():
    assert friendly_error(TranslatorConfigError(status=401)) == ("error.bad_key", {})
    assert friendly_error(TranslatorConfigError(status=404)) == \
        ("error.model_not_found", {})
    assert friendly_error(TranslatorConfigError(status=400)) == \
        ("error.api_http", {"status": 400})
    assert friendly_error(TranslatorOffline(status=503)) == ("error.offline", {})
    assert friendly_error(TranslatorOffline()) == ("error.offline", {})


def test_friendly_error_prefers_the_api_message_over_status_guess():
    # 狀態碼猜的提示會誤導（自架端點 404 常是網址錯而非模型錯），有 API 說明就用它
    assert friendly_error(TranslatorConfigError("Incorrect API key", status=401)) == \
        ("error.api_response", {"status": 401, "message": "Incorrect API key"})
    assert friendly_error(TranslatorOffline("upstream down", status=503)) == \
        ("error.api_response", {"status": 503, "message": "upstream down"})


def test_friendly_error_shows_connection_failure_reason():
    assert friendly_error(TranslatorOffline("[Errno 11001] getaddrinfo failed")) == \
        ("error.offline_detail", {"message": "[Errno 11001] getaddrinfo failed"})


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


def _api(provider: str = "custom", **fields) -> dict:
    """ModelField 只認目前生效的那家（扁平）：provider 加上那家自己的欄位。"""
    return {"provider": provider, **API_PROFILE_FIELDS[provider], **fields}


def _model_field(parent, api: dict | None = None):
    field = ModelField(parent, tk.StringVar(), lambda: api or _api(base_url="http://x"))
    field.pack(fill="x")
    parent.update()
    return field


def test_model_field_shows_fetched_models(root):
    field = _model_field(root)
    field.show_models(["gemma3", "qwen3"])
    assert field.options() == ["gemma3", "qwen3"]
    assert "2" in field.status()


def test_model_field_unsupported_endpoint_hints_manual_input(root):
    field = _model_field(root)
    field.show_error(TranslatorNoModelList("HTTP 404"))
    assert field.status() == t("hint.model_no_list")
    assert field.options() == []


def test_model_field_error_uses_friendly_message(root):
    field = _model_field(root)
    field.show_error(TranslatorOffline("refused"))
    assert field.status() == t("error.offline_detail", message="refused")


def test_refresh_result_reports_failures_for_providers_without_base_url(root):
    # 回歸：openai／claude 的設定檔沒有 base_url，失敗分支寫 log 時 KeyError，
    # 錯誤永遠顯示不出來、按鈕卡在「載入中」
    api = _api("openai", model="gpt-5.6-sol", api_key="sk-1")
    field = _model_field(root, api)
    field._on_refreshed(TranslatorConfigError("Incorrect API key", status=401), api)
    assert field.status() == t("error.api_response", status=401, message="Incorrect API key")


def test_model_field_error_makes_urls_clickable(root):
    field = _model_field(root)
    field.show_error(
        TranslatorConfigError("see https://a.example/docs for models", status=400))
    assert "https://a.example/docs" in field.status()
    assert field._status.links() == [
        ("https://a.example/docs", "https://a.example/docs")]


def test_model_field_typing_filters_fetched_options(root):
    field = _model_field(root)
    field.show_models(["gemma3", "qwen3-32b", "qwen3-8b"])
    field._var.set("qwen")
    field.refresh_options()
    assert field.options() == ["qwen3-32b", "qwen3-8b"]
    field._var.set("")
    field.refresh_options()
    assert field.options() == ["gemma3", "qwen3-32b", "qwen3-8b"]


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
    field = _model_field(root)
    field.show_models(["gemma3", "qwen3"])
    assert field.status() == t("hint.model_found", count=2)


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


def test_link_label_opens_the_url_on_click(root, monkeypatch):
    import tkinter as tk

    from src.ui.form import link_label
    from src.ui.richtext import LINK_COLOR

    opened = []
    monkeypatch.setattr(form_module.webbrowser, "open", opened.append)
    holder = tk.Frame(root)
    label = link_label(holder, "GoneTone", "https://example.invalid/author")

    assert label.cget("text") == "GoneTone"
    assert str(label.cget("foreground")) == LINK_COLOR
    assert "hand2" in str(label.cget("cursor"))
    label.event_generate("<Button-1>")
    root.update()
    assert opened == ["https://example.invalid/author"]
    holder.destroy()


def test_parse_link_markup_plain_text_has_no_links():
    from src.ui.richtext import parse_link_markup

    assert parse_link_markup("GoneTone、Someone") == [("GoneTone、Someone", None)]
    assert parse_link_markup("") == []


def test_parse_link_markup_splits_links_from_surrounding_text():
    from src.ui.richtext import parse_link_markup

    assert parse_link_markup("[A](https://a.example)、B") == [
        ("A", "https://a.example"), ("、B", None)]
    assert parse_link_markup("由 [A](http://a.example) 與 [B](https://b.example) 翻譯") == [
        ("由 ", None), ("A", "http://a.example"), (" 與 ", None),
        ("B", "https://b.example"), (" 翻譯", None)]


def test_parse_link_markup_rejects_non_http_urls():
    # 語言檔可能來自外部貢獻者：其他 scheme 只留文字、不給點，寧可少一條連結
    from src.ui.richtext import parse_link_markup

    assert parse_link_markup("[A](javascript:alert)") == [("A", None)]
    assert parse_link_markup("[A](file:///C:/x)") == [("A", None)]


def test_parse_link_markup_leaves_broken_syntax_as_text():
    from src.ui.richtext import parse_link_markup

    assert parse_link_markup("[A(https://a.example)") == [("[A(https://a.example)", None)]
    assert parse_link_markup("[A] (https://a.example)") == [
        ("[A] (https://a.example)", None)]


def test_linked_text_makes_only_the_link_segment_clickable(root, monkeypatch):
    from src.ui.form import linked_text
    from src.ui.richtext import LINK_COLOR

    opened = []
    monkeypatch.setattr(form_module.webbrowser, "open", opened.append)
    row = linked_text(root, "由 [A](https://a.example) 翻譯")
    plain, link, tail = row.pack_slaves()
    assert [w.cget("text") for w in (plain, link, tail)] == ["由 ", "A", " 翻譯"]
    assert str(link.cget("foreground")) == LINK_COLOR
    assert str(plain.cget("foreground")) != LINK_COLOR

    link.event_generate("<Button-1>")
    root.update()
    assert opened == ["https://a.example"]
