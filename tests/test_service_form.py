"""ServiceForm：單筆服務編輯表單的行為。"""
import tkinter as tk
from tkinter import ttk

import pytest

from src.i18n import t
from src.services import EFFORT_LOW, PROVIDERS, new_service
from src.translation.translator import TranslatorConfigError
from src.ui.service_form import ServiceForm


@pytest.fixture
def blank(root):
    return ServiceForm(root, new_service("openai", []))


def _field_texts(form) -> list[str]:
    """欄位區裡所有元件的文字（遞迴），用來斷言某一欄有沒有被畫出來。"""
    def walk(parent):
        for widget in parent.winfo_children():
            try:
                yield str(widget.cget("text"))
            except tk.TclError:
                pass
            yield from walk(widget)

    return list(walk(form._fields))


def _effort_combobox(form):
    """思考深度那個下拉：模型欄也是 Combobox，靠 textvariable 認人。"""
    stack = [form._fields]
    while stack:
        widget = stack.pop()
        if (isinstance(widget, ttk.Combobox)
                and str(widget.cget("textvariable")) == str(form._effort_shown)):
            return widget
        stack.extend(widget.winfo_children())
    return None


def test_new_form_starts_from_the_providers_defaults(blank):
    values = blank.values()
    assert values["provider"] == "openai"
    assert values["name"] == "ChatGPT"
    assert values["model"] == ""


def test_editing_keeps_the_id(root):
    service = new_service("claude", [])
    service.update(name="我的 Claude", model="claude-x", api_key="sk-ant")
    form = ServiceForm(root, service)
    assert form.values()["id"] == service["id"]
    assert form.values()["name"] == "我的 Claude"
    assert form.values()["model"] == "claude-x"


def test_values_only_carry_the_fields_that_provider_has(root):
    form = ServiceForm(root, new_service("claude", []))
    assert set(form.values()) == {"id", "name", "provider", "model", "api_key", "effort"}


def test_api_values_drop_the_bookkeeping_fields(blank):
    assert "id" not in blank.api_values() and "name" not in blank.api_values()


def test_switching_provider_rewrites_an_untouched_name(blank):
    blank.set_provider("claude")
    assert blank.values()["name"] == "Claude"


def test_switching_provider_keeps_a_name_the_user_typed(blank):
    blank.set_name("戰鬥用")
    blank.set_provider("claude")
    assert blank.values()["name"] == "戰鬥用"


def test_switching_provider_rewrites_a_suffixed_untouched_name(root):
    # 回歸：清單裡已經有一筆叫 ChatGPT，draft 的自動名稱是「ChatGPT (2)」——
    # 換服務商時這仍算「使用者沒改過名字」，要跟著換成新服務商的短名。
    existing = new_service("openai", [])
    draft = new_service("openai", [existing])
    form = ServiceForm(root, draft)
    form.set_provider("claude")
    assert form.values()["name"] == "Claude"


def test_a_blank_name_falls_back_to_the_provider_short_name(blank):
    blank.set_name("   ")
    assert blank.values()["name"] == "ChatGPT"


def test_switching_provider_does_not_carry_values_across(root):
    service = new_service("openai", [])
    service.update(model="gpt-x", api_key="sk-1")
    form = ServiceForm(root, service)
    form.set_provider("claude")
    assert form.values()["model"] == ""
    assert form.values()["api_key"] == ""


def test_claude_keeps_its_effort_choice(root):
    service = new_service("claude", [])
    service["effort"] = EFFORT_LOW
    form = ServiceForm(root, service)
    assert form.values()["effort"] == EFFORT_LOW


def test_switching_provider_clears_the_test_result(blank):
    # 顯示中的結果是對上一家端點測出來的，換了一家就不算數
    blank._show_test_result(True, "connected, sample translation")
    assert blank.test_passed
    blank.set_provider("claude")
    assert not blank.test_passed
    assert blank._test_result.text() == ""


def test_claude_shows_thinking_depth_instead_of_a_toggle(root):
    # Claude 沒有「完全不思考」，做成與另兩家一樣的勾選會誤導
    texts = _field_texts(ServiceForm(root, new_service("claude", [])))
    assert t("field.effort") in texts
    assert t("field.thinking") not in texts


def test_the_other_providers_keep_the_thinking_toggle(root):
    for provider in ("openai", "custom"):
        texts = _field_texts(ServiceForm(root, new_service(provider, [])))
        assert t("field.thinking") in texts, provider
        assert t("field.effort") not in texts, provider


def test_the_effort_dropdown_shows_the_saved_choice(root):
    # 下拉顯示的是譯文、存回設定的是代碼，兩者的對應要釘住
    service = new_service("claude", [])
    service["effort"] = EFFORT_LOW
    form = ServiceForm(root, service)
    assert form._effort_shown.get() == t("effort.low")


def test_choosing_an_effort_stores_its_code(root):
    form = ServiceForm(root, new_service("claude", []))
    combo = _effort_combobox(form)
    form._effort_shown.set(t("effort.low"))
    combo.event_generate("<<ComboboxSelected>>")
    root.update()
    assert form.values()["effort"] == EFFORT_LOW


def test_switching_provider_clears_the_fetched_model_list(blank):
    # 模型清單是上一家端點給的，換一家就不算數（靠 _model_row 每次重建新元件達成）
    blank._model_field.show_models(["gpt-5.6-sol", "gpt-5.6-luna"])
    blank.set_provider("claude")
    assert blank._model_field.options() == []


def test_a_successful_test_shows_the_sample_translation(blank):
    blank._on_tested("[Tester] 你好", blank.api_values())
    assert blank.test_passed
    assert blank._test_result.text() == "✓ " + t("test.success", sample="[Tester] 你好")


def test_a_failed_test_shows_the_friendly_error(blank):
    blank._on_tested(TranslatorConfigError("Incorrect API key", status=401),
                     blank.api_values())
    assert not blank.test_passed
    assert blank._test_result.text() == "✗ " + t("error.api_response", status=401,
                                                 message="Incorrect API key")


def test_the_test_target_language_defaults_to_the_ui_language(root):
    # 精靈不呼叫 set_target_language_fn：測試連線要退到介面語言的自稱，而不是炸掉
    from src import i18n

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        form = ServiceForm(root, new_service("openai", []))
        assert form._target_language_fn() == i18n.language_name("zh-TW")
        form.set_target_language_fn(lambda: "日本語")
        assert form._target_language_fn() == "日本語"
    finally:
        i18n.set_language(before)


def _descendants(parent):
    for widget in parent.winfo_children():
        yield widget
        yield from _descendants(widget)


def _change_button(form):
    """服務商那一列的〔更改〕按鈕（表單裡唯一一顆用這個文案的按鈕）。"""
    return next(w for w in _descendants(form)
                if isinstance(w, ttk.Button) and str(w.cget("text")) == t("button.change"))


def test_the_provider_row_shows_the_current_provider(blank):
    assert blank._provider_label.cget("text") == t(PROVIDERS["openai"].label_key)


def test_the_change_button_switches_the_provider(blank, monkeypatch):
    monkeypatch.setattr("src.ui.service_form.open_provider_picker", lambda parent: "claude")
    _change_button(blank).invoke()
    assert blank.values()["provider"] == "claude"
    assert blank.values()["name"] == "Claude"
    assert blank._provider_label.cget("text") == t(PROVIDERS["claude"].label_key)


def test_cancelling_the_provider_pick_changes_nothing(blank, monkeypatch):
    monkeypatch.setattr("src.ui.service_form.open_provider_picker", lambda parent: None)
    before = blank.values()
    _change_button(blank).invoke()
    assert blank.values() == before
    assert blank._provider_label.cget("text") == t(PROVIDERS["openai"].label_key)
