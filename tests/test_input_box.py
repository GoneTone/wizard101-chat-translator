import queue
import tkinter as tk

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
    box._show_error("翻譯失敗:timeout", current_session)

    # Status should be updated to error message
    assert box._status.cget("text") == "翻譯失敗:timeout"
    assert box._status.cget("fg") == "#ff5f5f"
    # Entry should be re-enabled for user retry
    assert box._entry.cget("state") == "normal"


def test_worker_failure_error_callback_runs(root):
    """翻譯失敗時排入 queue 的錯誤回呼要能執行(不因 except 的 exc 出範圍而 NameError)。"""
    import httpx

    ui_queue = queue.Queue()

    def failing_translate(text):
        raise httpx.HTTPError("boom")

    box = InputBox(root, failing_translate, ui_queue, lambda e, h: None)
    box._win = tk.Toplevel(root)
    box._entry = tk.Entry(box._win)
    box._status = tk.Label(box._win, text="original")
    session = box._session

    box._worker("你好", None, session)  # 直接呼叫:走 except 分支、排入錯誤回呼
    callback = ui_queue.get_nowait()
    callback()  # 修正前此處會 NameError: name 'exc' is not defined

    assert box._status.cget("text").startswith("翻譯失敗")


def test_input_box_restores_saved_position(root):
    box = InputBox(root, lambda t: t, queue.Queue(), lambda e, h: None,
                   position={"x": 321, "y": 210})
    box.show()
    box._win.update_idletasks()
    assert box._win.geometry().startswith("460x84+321+210")
    box.close()


def test_input_box_saves_position_on_close(root):
    saved = []
    box = InputBox(root, lambda t: t, queue.Queue(), lambda e, h: None,
                   position={"x": 150, "y": 160}, on_move=lambda x, y: saved.append((x, y)))
    box.show()
    box._win.update_idletasks()
    box.close()
    assert len(saved) == 1
    assert all(isinstance(v, int) for v in saved[0])
