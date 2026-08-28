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
