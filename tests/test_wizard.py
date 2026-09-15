"""精靈步驟門檻的純邏輯測試（UI 互動靠實機驗證）。"""
from src.ui.wizard import STEP_API, STEP_PREFS, can_advance


def test_step_api_requires_valid_form():
    assert not can_advance(STEP_API, api_test_passed=True, api_errors=["error.need_api_key"])


def test_step_api_requires_passed_or_skipped_test():
    assert not can_advance(STEP_API, api_test_passed=False, api_errors=[])
    assert can_advance(STEP_API, api_test_passed=True, api_errors=[])


def test_step_prefs_always_advances():
    assert can_advance(STEP_PREFS, api_test_passed=False, api_errors=[])


def test_can_advance_allows_language_step():
    from src.ui.wizard import STEP_LANG, can_advance

    # 語言頁沒有必填欄位，永遠可以往下一步
    assert can_advance(STEP_LANG, api_test_passed=False, api_errors=["error.need_model"])


def test_language_change_marks_restart_and_applies_language(root):
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        wizard = SetupWizard(root, cfg)
        wizard._on_language_change("en-US")
        assert cfg["ui_language"] == "en-US"
        assert i18n.current_language() == "en-US"
        assert wizard.restart is True
    finally:
        i18n.set_language(before)


def test_language_change_updates_untouched_target_language(root):
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        cfg["target_language"] = "繁體中文（台灣）"   # 使用者還沒動過
        wizard = SetupWizard(root, cfg)
        wizard._on_language_change("en-US")
        assert cfg["target_language"] == "English"
    finally:
        i18n.set_language(before)


def test_language_change_keeps_customised_target_language(root):
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        cfg["target_language"] = "日本語"   # 使用者自己選過了
        wizard = SetupWizard(root, cfg)
        wizard._on_language_change("en-US")
        assert cfg["target_language"] == "日本語"
    finally:
        i18n.set_language(before)


def test_language_change_from_en_bootstrap_follows_to_zh_cn(root):
    """回歸測試：英文系統首次啟動時 bootstrap_language 把 target_language 設成介面語言的
    自稱（"English"），精靈再切到 zh-CN 時 `target_language == old_default` 才比對得到。
    修復前 target_language 永遠停在「繁體中文（台灣）」，這條分支形同死碼。"""
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.main import bootstrap_language
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        assert cfg["ui_language"] is None  # 首次執行
        language = bootstrap_language(cfg, config_existed=False, detect=lambda: "en-US")
        i18n.set_language(language)
        assert cfg["target_language"] == i18n.language_name("en-US")

        wizard = SetupWizard(root, cfg)
        wizard._on_language_change("zh-CN")
        assert cfg["target_language"] == i18n.language_name("zh-CN")
        assert cfg["target_language"] == "简体中文（中国）"
    finally:
        i18n.set_language(before)


def test_bootstrap_language_first_run_no_config_file():
    """真正首次執行（config 檔案原本不存在）：介面語言依偵測，target_language 也跟著換。"""
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.main import bootstrap_language

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    assert cfg["ui_language"] is None

    language = bootstrap_language(cfg, config_existed=False, detect=lambda: "en-US")

    assert language == "en-US"
    assert cfg["target_language"] == i18n.language_name("en-US")


def test_bootstrap_language_upgrading_user_keeps_target_language():
    """回歸測試：升級使用者的 config 已存在，只是舊版沒有 ui_language（被回填成 None），
    不代表沒走過精靈：介面語言仍依偵測，但使用者選好的 target_language 不可被覆蓋。"""
    from src.main import bootstrap_language

    cfg = {"ui_language": None, "target_language": "日本語"}

    language = bootstrap_language(cfg, config_existed=True, detect=lambda: "en-US")

    assert language == "en-US"
    assert cfg["target_language"] == "日本語"


def test_bootstrap_language_returning_user_uses_saved_language():
    """回頭使用者：config 已有真實的 ui_language，直接採用、不呼叫 detect，target_language 不動。"""
    from src.main import bootstrap_language

    cfg = {"ui_language": "zh-CN", "target_language": "日本語"}
    calls = []

    language = bootstrap_language(cfg, config_existed=True, detect=lambda: calls.append(1) or "en-US")

    assert language == "zh-CN"
    assert calls == []
    assert cfg["target_language"] == "日本語"


def test_language_step_shows_the_translators(root, monkeypatch):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.i18n import t
    from src.ui import form as form_module
    from src.ui.wizard import STEP_PREFS, SetupWizard

    monkeypatch.setattr(form_module, "translators",
                        lambda code: "[A](https://a.example)")
    wizard = SetupWizard(root, copy.deepcopy(DEFAULT_CONFIG))
    label, names = wizard._translators_row.pack_slaves()
    assert label.cget("text") == t("credit.translators")
    assert [w.cget("text") for w in names.pack_slaves()] == ["A"]
    order = [str(w) for w in wizard._body.pack_slaves()]
    assert (order.index(str(wizard._help_translate_link))
            == order.index(str(wizard._translators_row)) + 1)

    # 掛名與邀請連結只屬於語言頁：往後的步驟不該還留著
    wizard._step = STEP_PREFS
    wizard._show_step()
    assert wizard._translators_row is None
    assert wizard._help_translate_link is None
    wizard._win.destroy()


def test_language_step_without_translators_shows_no_row(root, monkeypatch):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.ui import form as form_module
    from src.ui.wizard import SetupWizard

    monkeypatch.setattr(form_module, "translators", lambda code: "")
    wizard = SetupWizard(root, copy.deepcopy(DEFAULT_CONFIG))
    assert wizard._translators_row is None
    wizard._win.destroy()


def test_language_step_links_to_crowdin_even_without_translators(root, monkeypatch):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.i18n import t
    from src.ui import form as form_module
    from src.ui.wizard import SetupWizard
    from src.updater import CROWDIN_URL

    monkeypatch.setattr(form_module, "translators", lambda code: "")
    opened = []
    monkeypatch.setattr(form_module.webbrowser, "open", opened.append)
    wizard = SetupWizard(root, copy.deepcopy(DEFAULT_CONFIG))
    link = wizard._help_translate_link
    assert link.cget("text") == t("credit.help_translate")
    order = [str(w) for w in wizard._body.pack_slaves()]
    assert order.index(str(link)) == order.index(str(wizard._ui_language)) + 1
    link.event_generate("<Button-1>")
    assert opened == [CROWDIN_URL]
    wizard._win.destroy()


def test_finish_stores_the_region_hotkey(root):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.ui.wizard import SetupWizard

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    wizard = SetupWizard(root, cfg)
    wizard._step = STEP_PREFS
    wizard._show_step()
    assert wizard._region_hotkey.value() == "ctrl+shift+space"
    wizard._region_hotkey.set_value("ctrl+alt+r")
    wizard._finish()
    assert wizard.completed is True
    assert cfg["region_hotkey"] == "ctrl+alt+r"


def test_finish_rejects_identical_hotkeys(root, monkeypatch):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.i18n import t
    from src.ui import wizard as wizard_module
    from src.ui.wizard import SetupWizard

    warnings = []
    monkeypatch.setattr(wizard_module.messagebox, "showwarning",
                        lambda title, message, parent=None: warnings.append(message))
    cfg = copy.deepcopy(DEFAULT_CONFIG)
    wizard = SetupWizard(root, cfg)
    wizard._step = STEP_PREFS
    wizard._show_step()
    wizard._region_hotkey.set_value(cfg["hotkey"])
    wizard._finish()
    assert wizard.completed is False
    assert warnings == [t("error.hotkeys_same")]
    assert cfg["region_hotkey"] == "ctrl+shift+space"
    wizard._win.destroy()
