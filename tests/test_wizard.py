"""精靈步驟門檻的純邏輯測試（UI 互動靠實機驗證）。"""
from src.ui.wizard import STEP_API, STEP_PREFS, can_advance


def test_step_api_requires_valid_form():
    assert not can_advance(STEP_API, api_test_passed=True, api_errors=["error.need_api_key"])


def test_step_api_requires_passed_or_skipped_test():
    assert not can_advance(STEP_API, api_test_passed=False, api_errors=[])
    assert can_advance(STEP_API, api_test_passed=True, api_errors=[])


def test_step_prefs_always_advances():
    assert can_advance(STEP_PREFS, api_test_passed=False, api_errors=[])
