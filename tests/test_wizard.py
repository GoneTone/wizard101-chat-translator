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
    """回歸測試（main.bootstrap_language 修好前的死路徑）：

    英文系統首次啟動時，main.bootstrap_language() 現在會把 target_language
    設成 DEFAULT_TARGET_LANGUAGE["en"]（"English"），而不是留在 config.py 的
    原始預設值「繁體中文（台灣）」。本測試從這個已修正的啟動狀態出發，
    驗證精靈頁再切到 zh-CN 時，_on_language_change 的
    `target_language == old_default` 判斷式現在真的比對得到，能接著把
    target_language 帶到 zh-CN 的預設值。修復前 target_language 永遠停在
    「繁體中文（台灣）」，此判斷式因 old_default 是 "English" 而永遠不成立，
    這條分支形同死碼。"""
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.main import bootstrap_language
    from src.ui.fields import DEFAULT_TARGET_LANGUAGE
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        assert cfg["ui_language"] is None  # 首次執行
        language = bootstrap_language(cfg, config_existed=False, detect=lambda: "en")
        i18n.set_language(language)
        assert cfg["target_language"] == DEFAULT_TARGET_LANGUAGE["en"]

        wizard = SetupWizard(root, cfg)
        wizard._on_language_change("zh-CN")
        assert cfg["target_language"] == DEFAULT_TARGET_LANGUAGE["zh-CN"]
        assert cfg["target_language"] == "简体中文（中国）"
    finally:
        i18n.set_language(before)


def test_bootstrap_language_first_run_no_config_file():
    """真正首次執行（config 檔案原本不存在）：介面語言依偵測，target_language 也跟著換。"""
    import copy

    from src.config import DEFAULT_CONFIG
    from src.main import bootstrap_language
    from src.ui.fields import DEFAULT_TARGET_LANGUAGE

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    assert cfg["ui_language"] is None

    language = bootstrap_language(cfg, config_existed=False, detect=lambda: "en")

    assert language == "en"
    assert cfg["target_language"] == DEFAULT_TARGET_LANGUAGE["en"]


def test_bootstrap_language_upgrading_user_keeps_target_language():
    """回歸測試：升級使用者的 config.json 已存在，只是 pre-i18n 版本沒有 ui_language 欄位，
    load_config()／_merge 會把它回填成 None（見 tests/test_config.py），不代表沒走過精靈。
    此情境下介面語言仍可依偵測決定，但使用者原本選好的 target_language 不可被覆蓋，
    也不該在每次啟動時被悄悄改回系統預設值。"""
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
