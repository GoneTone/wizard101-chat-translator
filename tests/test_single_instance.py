import uuid

import win32api

from src.main import acquire_single_instance, focus_running_instance


def unique_name() -> str:
    """每個測試用自己的 mutex 名稱：借用正式名稱的話，使用者正在跑程式就會讓測試失敗。"""
    return f"wizard101-chat-translator-test-{uuid.uuid4()}"


def test_first_instance_acquires_the_mutex():
    name = unique_name()
    handle = acquire_single_instance(name)
    assert handle is not None
    win32api.CloseHandle(handle)


def test_second_instance_is_refused():
    name = unique_name()
    first = acquire_single_instance(name)
    try:
        assert acquire_single_instance(name) is None
    finally:
        win32api.CloseHandle(first)


def test_focus_reports_failure_when_no_window_matches():
    # 找不到既有視窗時要回 False，呼叫端才能照實記錄「沒帶到前景」
    assert focus_running_instance(f"no such window {uuid.uuid4()}") is False
