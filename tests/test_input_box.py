import queue
import tkinter as tk

import pytest

from src.composer.input_box import InputBox


def test_stale_session_discarded_on_cancel(root):
    """Test that cancelled translations don't call on_translated.

    Simulates: user presses Enter (session starts) → user presses Esc
    (session incremented) → translation completes → queued lambda runs
    but sees session mismatch and discards result.
    """
    ui_queue = queue.Queue()
    on_translated = []  # list to capture call

    def fake_translate(text):
        return "translated: " + text

    def on_translated_callback(english, hwnd):
        on_translated.append((english, hwnd))

    box = InputBox(root, fake_translate, ui_queue, on_translated_callback)

    # Simulate show() — increments session to 1
    box._win = tk.Toplevel(root)  # fake window to pass is not None check
    box._session += 1
    session_started = box._session
    assert session_started == 1

    # Simulate Esc cancellation — increments session to 2
    box._session += 1
    session_cancelled = box._session
    assert session_cancelled == 2

    # Simulate queued _finish with stale session
    # Direct call to _finish with old session
    box._finish("english text", None, session_started)

    # on_translated should NOT have been called
    assert on_translated == [], "Stale result should be discarded"


def test_finish_over_limit_keeps_window_and_blocks_send(root):
    from src.composer.input_box import GAME_INPUT_MAX_CHARS
    sent = []
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: sent.append(a))
    box.show()
    session = box._session
    box._finish("x" * (GAME_INPUT_MAX_CHARS + 1), None, session)
    assert box._win is not None          # 不關閉，讓使用者刪減重送
    assert sent == []                    # 不鍵入遊戲
    assert str(GAME_INPUT_MAX_CHARS + 1) in box._status.cget("text")
    assert str(box._entry.cget("state")) == "normal"  # 輸入欄恢復可編輯
    box.close()


def test_finish_within_limit_sends_and_closes(root):
    from src.composer.input_box import GAME_INPUT_MAX_CHARS
    sent = []
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: sent.append(a))
    box.show()
    box._finish("x" * GAME_INPUT_MAX_CHARS, None, box._session)
    assert box._win is None
    assert len(sent) == 1


def test_close_with_stale_target_hwnd_does_not_crash(root):
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None)
    box.show()
    box._target_hwnd = 0x7FFFFFFF        # 已不存在的視窗
    box.close()
    assert box._win is None


def test_enter_on_empty_input_closes_window(root):
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None)
    box.show()
    assert box._win is not None
    box._on_enter(None)          # 空白按 Enter → 關閉（等同 Esc）
    assert box._win is None


def test_non_cancelled_translation_succeeds(root):
    """Test that non-cancelled translations still call on_translated."""
    ui_queue = queue.Queue()
    on_translated = []

    def fake_translate(text):
        return "translated: " + text

    def on_translated_callback(english, hwnd):
        on_translated.append((english, hwnd))

    box = InputBox(root, fake_translate, ui_queue, on_translated_callback)

    # Simulate show() — increments session to 1
    box._win = tk.Toplevel(root)  # fake window
    box._session += 1
    session_started = box._session
    assert session_started == 1

    # Simulate successful _finish with matching session (no close called first)
    box._finish("english text", 12345, session_started)

    # on_translated SHOULD have been called
    assert on_translated == [("english text", 12345)]


def test_stale_error_discarded_on_cancel(root):
    """Test that error messages from stale sessions are discarded."""
    ui_queue = queue.Queue()
    on_translated = []

    def fake_translate(text):
        return "translated: " + text

    def on_translated_callback(english, hwnd):
        on_translated.append((english, hwnd))

    box = InputBox(root, fake_translate, ui_queue, on_translated_callback)

    # Setup
    box._win = tk.Toplevel(root)
    box._entry = tk.Entry(box._win)
    box._status = tk.Label(box._win, text="test")
    box._session += 1
    session_started = box._session

    # Simulate cancellation
    box._session += 1

    # Simulate error with stale session
    box._show_error("Error message", session_started)

    # Entry and status should not be updated (because session was stale)
    # The entry/status should still be in their original state
    assert box._status.cget("text") == "test"
    assert box._status.cget("fg") != "#ff5f5f"


def test_current_error_shown_on_error(root):
    """Test that error messages with matching session are shown."""
    ui_queue = queue.Queue()
    on_translated = []

    def fake_translate(text):
        return "translated: " + text

    def on_translated_callback(english, hwnd):
        on_translated.append((english, hwnd))

    box = InputBox(root, fake_translate, ui_queue, on_translated_callback)

    # Setup
    box._win = tk.Toplevel(root)
    box._entry = tk.Entry(box._win)
    box._entry.configure(state="disabled")
    box._status = tk.Label(box._win, text="original")
    box._session += 1
    current_session = box._session

    # Show error with matching session
    box._show_error("boom", current_session)

    # Status should be updated to error message
    assert box._status.cget("text") == "boom"
    assert box._status.cget("fg") == "#ff5f5f"
    # Entry should be re-enabled for user retry
    assert box._entry.cget("state") == "normal"


def test_worker_failure_error_callback_runs(root):
    """翻譯失敗時排入 queue 的錯誤回呼要能執行（不因 except 的 exc 出範圍而 NameError）。"""
    import httpx

    ui_queue = queue.Queue()

    def failing_translate(text):
        raise httpx.HTTPError("boom")

    box = InputBox(root, failing_translate, ui_queue, lambda e, h: None)
    box._win = tk.Toplevel(root)
    box._entry = tk.Entry(box._win)
    box._status = tk.Label(box._win, text="original")
    session = box._session

    box._worker("你好", None, session)  # 直接呼叫：走 except 分支、排入錯誤回呼
    callback = ui_queue.get_nowait()
    callback()  # 修正前此處會 NameError: name 'exc' is not defined

    assert "boom" in box._status.cget("text")


@pytest.mark.real_position
def test_input_box_restores_saved_position(root):
    box = InputBox(root, lambda t: t, queue.Queue(), lambda e, h: None,
                   position={"x": 321, "y": 210})
    box.show()
    box._win.update_idletasks()
    # 高度由 _fit_height 依內容決定，這裡只釘寬度與位置
    assert box._win.geometry().startswith("460x")
    assert box._win.geometry().endswith("+321+210")
    box.close()


def test_input_box_saves_position_on_close(root):
    saved = []
    box = InputBox(root, lambda t: t, queue.Queue(), lambda e, h: None,
                   position={"x": 150, "y": 160},
                   on_geometry_change=lambda x, y, w: saved.append((x, y, w)))
    box.show()
    box._win.update_idletasks()
    box.close()
    assert len(saved) == 1
    assert all(isinstance(v, int) for v in saved[0])


def test_show_restores_remembered_width(root):
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None, width=620)
    box.show()
    box._win.update_idletasks()
    assert box._win.winfo_width() == 620
    box.close()


def test_show_clamps_remembered_width_to_minimum(root):
    from src.composer.input_box import MIN_WIDTH
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None, width=80)
    box.show()
    box._win.update_idletasks()
    assert box._win.winfo_width() == MIN_WIDTH
    box.close()


@pytest.mark.real_position
def test_close_reports_position_and_width(root):
    reported = []
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None,
                   on_geometry_change=lambda x, y, w: reported.append((x, y, w)))
    box.show()
    box._win.geometry("700x84+120+140")
    box._win.update_idletasks()
    box.close()
    assert reported == [(120, 140, 700)]


def test_fit_height_keeps_user_width(root):
    # 高度自適應不得把使用者拖出來的寬度打回預設值
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None)
    box.show()
    box._win.geometry("700x84+10+10")
    box._win.update_idletasks()
    box._fit_height()
    box._win.update_idletasks()
    assert box._win.winfo_width() == 700
    box.close()


def test_fit_height_shrinks_when_hint_needs_fewer_lines(root):
    # 拉寬 → 提示文字行數變少 → 高度要跟著貼回內容，不能停在開窗時的高度
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None)
    box.show()
    box._win.update_idletasks()
    box._win.geometry(f"1000x{box._win.winfo_height()}")
    box._win.update()
    assert box._win.winfo_height() == box._win.winfo_reqheight()
    box.close()


def test_error_message_follows_language(root):
    import httpx

    from src import i18n

    before = i18n.current_language()
    try:
        i18n.set_language("en")
        ui_queue = queue.Queue()

        def failing_translate(text):
            raise httpx.HTTPError("boom")

        box = InputBox(root, failing_translate, ui_queue, lambda e, h: None)
        box._win = tk.Toplevel(root)
        box._entry = tk.Entry(box._win)
        box._status = tk.Label(box._win, text="original")
        box._worker("hello", None, box._session)
        ui_queue.get_nowait()()
        assert box._status.cget("text").startswith("Translation failed:")
    finally:
        i18n.set_language(before)
