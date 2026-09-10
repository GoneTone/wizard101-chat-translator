"""fields 純邏輯測試：表單驗證、錯誤文案、熱鍵字串。"""
import copy
import gc
import tkinter as tk
from tkinter import ttk

import pytest

from src.config import API_PROVIDERS, DEFAULT_CONFIG, active_api
from src.i18n import t
from src.translation.translator import (
    TranslatorConfigError,
    TranslatorNoModelList,
    TranslatorOffline,
)
from src.ui.fields import (
    PROVIDERS,
    ApiFields,
    ModelField,
    filter_models,
    friendly_error,
    validate_api_form,
    validate_endpoint_fields,
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
    assert set(PROVIDERS) == set(API_PROVIDERS)  # 每家都要有自己的一份設定可存
    assert PROVIDERS["custom"].needs_base_url
    assert not PROVIDERS["openai"].needs_base_url


def test_provider_labels_are_translated():
    from src import i18n
    from src.ui.fields import PROVIDERS

    before = i18n.current_language()
    try:
        i18n.set_language("en")
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
    field.set_value("en")
    assert field.value() == "en"


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




def _initial(provider="openai", **profile):
    """ApiFields 吃的是整個 api 區塊：provider 加上每家各一份設定。"""
    api = copy.deepcopy(DEFAULT_CONFIG["api"])
    api["provider"] = provider
    api[provider].update(profile)
    return api


def _switch(fields, provider):
    fields._provider.set(provider)
    fields._rebuild_fields()


def test_switch_provider_shows_that_providers_own_values(root):
    # 模型 ID 跨服務商不通用：切過去看到的是那家自己的設定，不是上一家的殘值
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-sol"))
    _switch(fields, "claude")
    assert fields.active_values()["model"] == ""


def test_switch_back_restores_the_previous_provider_values(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-sol",
                                      api_key="sk-1", thinking=True))
    _switch(fields, "custom")
    fields._base_url.set("http://x")
    fields._model.set("qwen3")
    _switch(fields, "openai")
    assert fields.active_values() == {"provider": "openai", "model": "gpt-5.6-sol",
                                      "api_key": "sk-1", "thinking": True}


def test_get_values_keeps_every_provider_profile(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-sol",
                                      api_key="sk-1"))
    _switch(fields, "custom")
    fields._base_url.set("http://x")
    fields._model.set("qwen3")
    api = fields.get_values()
    assert api["provider"] == "custom"
    assert api["custom"]["base_url"] == "http://x"      # 目前欄位值也寫了回去
    assert api["openai"] == {"model": "gpt-5.6-sol", "api_key": "sk-1",
                             "thinking": False}


def test_rebuild_without_switching_keeps_model(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-terra"))
    fields._rebuild_fields()
    assert fields.active_values()["model"] == "gpt-5.6-terra"


def _widget_texts(parent):
    """遞迴收集元件上的文字，用來斷言某一欄有沒有被畫出來。"""
    texts = []
    for w in parent.winfo_children():
        try:
            texts.append(str(w.cget("text")))
        except tk.TclError:
            pass
        texts.extend(_widget_texts(w))
    return texts


def _effort_combobox(fields):
    """思考深度那個下拉：模型欄也是 Combobox，靠 textvariable 認人。"""
    target = str(fields._effort_shown)
    stack = [fields._fields]
    while stack:
        widget = stack.pop()
        if isinstance(widget, ttk.Combobox) and str(widget.cget("textvariable")) == target:
            return widget
        stack.extend(widget.winfo_children())
    return None


def test_claude_shows_thinking_depth_instead_of_a_toggle(root):
    # Claude 沒有「完全不思考」，做成與另兩家一樣的勾選會誤導
    fields = ApiFields(root, _initial(provider="claude", model="claude-opus-5"))
    texts = _widget_texts(fields._fields)
    assert t("field.effort") in texts
    assert t("field.thinking") not in texts


def test_openai_keeps_the_thinking_toggle(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5"))
    texts = _widget_texts(fields._fields)
    assert t("field.thinking") in texts
    assert t("field.effort") not in texts


def test_effort_dropdown_shows_the_saved_choice(root):
    from src.config import EFFORT_LOW

    fields = ApiFields(root, _initial(provider="claude", model="claude-opus-5",
                                      effort=EFFORT_LOW))
    assert fields._effort_shown.get() == t("effort.low")


def test_choosing_an_effort_stores_its_code(root):
    from src.config import EFFORT_LOW

    fields = ApiFields(root, _initial(provider="claude", model="claude-opus-5"))
    combo = _effort_combobox(fields)
    fields._effort_shown.set(t("effort.low"))
    combo.event_generate("<<ComboboxSelected>>")
    root.update()
    assert fields.active_values()["effort"] == EFFORT_LOW


def test_effort_survives_switching_providers(root):
    from src.config import EFFORT_LOW

    fields = ApiFields(root, _initial(provider="claude", model="claude-opus-5",
                                      effort=EFFORT_LOW))
    _switch(fields, "openai")
    _switch(fields, "claude")
    assert fields.active_values()["effort"] == EFFORT_LOW
    assert fields._effort_shown.get() == t("effort.low")


def test_switch_provider_clears_fetched_model_list(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-sol"))
    fields._model_field.show_models(["gpt-5.6-sol", "gpt-5.6-luna"])
    _switch(fields, "custom")
    assert fields._model_field.options() == []


def test_switch_provider_clears_test_result_label(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-sol"))
    fields._show_test_result(True, "連線成功　範例：hi")
    assert fields._test_result.text() != ""
    _switch(fields, "claude")
    assert fields._test_result.text() == ""


def test_test_result_makes_urls_in_the_api_message_clickable(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-sol"))
    fields._show_test_result(False, "HTTP 401: get a key at https://a.example/keys.")
    assert fields._test_result.text() == "✗ HTTP 401: get a key at https://a.example/keys."
    assert fields._test_result.links() == [("https://a.example/keys", "https://a.example/keys")]


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
    assert fields._model_field.status() == t("error.offline_detail", message="refused")


def test_refresh_worker_reports_failures_for_providers_without_base_url(root, monkeypatch):
    # 回歸：openai／claude 的設定檔沒有 base_url，失敗分支寫 log 時 KeyError，
    # 例外沒放進 queue，主執行緒的輪詢永遠等不到結果、按鈕卡在「載入中」
    from src.ui import fields as fields_module

    def rejected(api, client=None):
        raise TranslatorConfigError("Incorrect API key", status=401)

    monkeypatch.setattr(fields_module, "list_models", rejected)
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5.6-sol", api_key="sk-1"))
    field = fields._model_field
    field._refresh_worker(fields.active_values())
    result = field._queue.get_nowait()
    assert isinstance(result, TranslatorConfigError)


def test_model_field_error_makes_urls_clickable(root):
    fields = ApiFields(root, _initial(provider="custom", base_url="http://x"))
    fields._model_field.show_error(
        TranslatorConfigError("see https://a.example/docs for models", status=400))
    assert "https://a.example/docs" in fields._model_field.status()
    assert fields._model_field._status.links() == [
        ("https://a.example/docs", "https://a.example/docs")]


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
    # ModelField 只認目前生效的那家（扁平），不需要看到其他家的設定
    field = ModelField(parent, tk.StringVar(),
                       lambda: active_api({"api": _initial(provider="custom",
                                                           base_url="http://x")}))
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
    assert fields._model_field.status() == t("hint.model_found", count=2)


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

    from src.ui import fields
    from src.ui.fields import LINK_COLOR, link_label

    opened = []
    monkeypatch.setattr(fields.webbrowser, "open", opened.append)
    holder = tk.Frame(root)
    label = link_label(holder, "GoneTone", "https://example.invalid/author")

    assert label.cget("text") == "GoneTone"
    assert str(label.cget("foreground")) == LINK_COLOR
    assert "hand2" in str(label.cget("cursor"))
    label.event_generate("<Button-1>")
    root.update()
    assert opened == ["https://example.invalid/author"]
    holder.destroy()


def test_custom_endpoint_orders_fields_by_fill_in_sequence(root):
    """自訂端點的欄位依填寫順序排：網址 → 金鑰 → 模型。

    模型清單要靠網址與金鑰才取得到；官方端點本來就是金鑰在模型之前，兩者一致
    切換服務商時欄位才不會跳動。"""
    fields = ApiFields(root, _initial(provider="custom", base_url="http://x"))
    texts = _widget_texts(fields._fields)
    url_at = texts.index(t("field.base_url"))
    key_at = texts.index(t("field.api_key_optional"))
    model_at = texts.index(t("field.model"))
    assert url_at < key_at < model_at, f"欄位順序不對：{texts}"


def test_official_endpoint_keeps_key_before_model(root):
    fields = ApiFields(root, _initial(provider="openai"))
    texts = _widget_texts(fields._fields)
    assert texts.index(t("field.api_key")) < texts.index(t("field.model"))


def test_get_key_link_sits_under_the_api_key_field(root):
    """取金鑰的連結是金鑰欄的輔助說明，要緊貼在它底下。

    原本擺在所有欄位最後，與它要幫的欄位分家 —— 使用者卡在金鑰欄時視線不會落到那裡。"""
    for provider in ("openai", "claude"):
        fields = ApiFields(root, _initial(provider=provider))
        texts = _widget_texts(fields._fields)
        key_at = texts.index(t("field.api_key"))
        link_at = texts.index(t("link.get_key"))
        model_at = texts.index(t("field.model"))
        assert key_at < link_at < model_at, f"{provider} 的連結位置不對：{texts}"


def test_test_connection_defaults_to_the_ui_languages_name(root, monkeypatch):
    # 精靈不呼叫 set_target_language_fn：預設要退到介面語言的自稱，而不是炸掉
    from src import i18n
    from src.ui import fields as fields_module

    captured = {}

    def fake_test_translate(api, target_language):
        captured["target"] = target_language
        return "[Tester] 你好"

    monkeypatch.setattr(fields_module, "test_translate", fake_test_translate)
    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        fields = ApiFields(root, _initial(provider="openai", model="gpt-5", api_key="k"))
        fields._test_worker(fields.active_values(), fields._target_language_fn())
    finally:
        i18n.set_language(before)
    assert captured["target"] == i18n.language_name("zh-TW")


def test_parse_link_markup_plain_text_has_no_links():
    from src.ui.fields import parse_link_markup

    assert parse_link_markup("GoneTone、Someone") == [("GoneTone、Someone", None)]
    assert parse_link_markup("") == []


def test_parse_link_markup_splits_links_from_surrounding_text():
    from src.ui.fields import parse_link_markup

    assert parse_link_markup("[A](https://a.example)、B") == [
        ("A", "https://a.example"), ("、B", None)]
    assert parse_link_markup("由 [A](http://a.example) 與 [B](https://b.example) 翻譯") == [
        ("由 ", None), ("A", "http://a.example"), (" 與 ", None),
        ("B", "https://b.example"), (" 翻譯", None)]


def test_parse_link_markup_rejects_non_http_urls():
    # 語言檔可能來自外部貢獻者：其他 scheme 只留文字、不給點，寧可少一條連結
    from src.ui.fields import parse_link_markup

    assert parse_link_markup("[A](javascript:alert)") == [("A", None)]
    assert parse_link_markup("[A](file:///C:/x)") == [("A", None)]


def test_parse_link_markup_leaves_broken_syntax_as_text():
    from src.ui.fields import parse_link_markup

    assert parse_link_markup("[A(https://a.example)") == [("[A(https://a.example)", None)]
    assert parse_link_markup("[A] (https://a.example)") == [
        ("[A] (https://a.example)", None)]


def test_linked_text_makes_only_the_link_segment_clickable(root, monkeypatch):
    from src.ui import fields as fields_module
    from src.ui.fields import LINK_COLOR, linked_text

    opened = []
    monkeypatch.setattr(fields_module.webbrowser, "open", opened.append)
    row = linked_text(root, "由 [A](https://a.example) 翻譯")
    plain, link, tail = row.pack_slaves()
    assert [w.cget("text") for w in (plain, link, tail)] == ["由 ", "A", " 翻譯"]
    assert str(link.cget("foreground")) == LINK_COLOR
    assert str(plain.cget("foreground")) != LINK_COLOR

    link.event_generate("<Button-1>")
    root.update()
    assert opened == ["https://a.example"]


def test_test_result_from_a_previous_provider_is_discarded(root):
    # 測試連線還在跑時切換服務商：舊結果回來不能把新這家標成「已測過」
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5", api_key="k"))
    fields._test_btn.configure(state="disabled")
    session = fields._test_session
    _switch(fields, "claude")
    fields._on_tested((True, "連線成功"), session)
    assert fields.test_passed is False
    assert fields._test_result.text() == ""


def test_test_result_from_the_current_run_is_applied(root):
    fields = ApiFields(root, _initial(provider="openai", model="gpt-5", api_key="k"))
    fields._on_tested((True, "連線成功"), fields._test_session)
    assert fields.test_passed is True
