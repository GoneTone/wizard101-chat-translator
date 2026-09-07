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
        wizard._on_language_change("en")
        assert cfg["ui_language"] == "en"
        assert i18n.current_language() == "en"
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
        wizard._on_language_change("en")
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
        wizard._on_language_change("en")
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
        language = bootstrap_language(cfg, config_existed=False, detect=lambda: "en")
        i18n.set_language(language)
        assert cfg["target_language"] == i18n.language_name("en")

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

    language = bootstrap_language(cfg, config_existed=False, detect=lambda: "en")

    assert language == "en"
    assert cfg["target_language"] == i18n.language_name("en")


def test_bootstrap_language_upgrading_user_keeps_target_language():
    """回歸測試：升級使用者的 config 已存在，只是舊版沒有 ui_language（被回填成 None），
    不代表沒走過精靈：介面語言仍依偵測，但使用者選好的 target_language 不可被覆蓋。"""
    from src.main import bootstrap_language

    cfg = {"ui_language": None, "target_language": "日本語"}

    language = bootstrap_language(cfg, config_existed=True, detect=lambda: "en")

    assert language == "en"
    assert cfg["target_language"] == "日本語"


def test_bootstrap_language_returning_user_uses_saved_language():
    """回頭使用者：config 已有真實的 ui_language，直接採用、不呼叫 detect，target_language 不動。"""
    from src.main import bootstrap_language

    cfg = {"ui_language": "zh-CN", "target_language": "日本語"}
    calls = []

    language = bootstrap_language(cfg, config_existed=True, detect=lambda: calls.append(1) or "en")

    assert language == "zh-CN"
    assert calls == []
    assert cfg["target_language"] == "日本語"


def test_language_step_shows_the_translators(root, monkeypatch):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.i18n import t
    from src.ui import fields as fields_module
    from src.ui.wizard import STEP_PREFS, SetupWizard

    monkeypatch.setattr(fields_module, "translators",
                        lambda code: "[A](https://a.example)")
    wizard = SetupWizard(root, copy.deepcopy(DEFAULT_CONFIG))
    label, names = wizard._translators_row.pack_slaves()
    assert label.cget("text") == t("credit.translators")
    assert [w.cget("text") for w in names.pack_slaves()] == ["A"]

    # 掛名只屬於語言頁：往後的步驟不該還留著那一列
    wizard._step = STEP_PREFS
    wizard._show_step()
    assert wizard._translators_row is None
    wizard._win.destroy()


def test_language_step_without_translators_shows_no_row(root, monkeypatch):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.ui import fields as fields_module
    from src.ui.wizard import SetupWizard

    monkeypatch.setattr(fields_module, "translators", lambda code: "")
    wizard = SetupWizard(root, copy.deepcopy(DEFAULT_CONFIG))
    assert wizard._translators_row is None
    wizard._win.destroy()
