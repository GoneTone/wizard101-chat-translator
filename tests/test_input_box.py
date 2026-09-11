import queue
import tkinter as tk

import pytest

from src.ui import input_box as input_box_module
from src.ui.geometry import anchored_position
from src.ui.input_box import InputBox


def test_stale_session_discarded_on_cancel(root):
    """Cancelled translations must not call on_translated.

    Enter starts a session, Esc bumps it, then the stale _finish sees the mismatch.
    """
    ui_queue = queue.Queue()
    on_translated = []

    def fake_translate(text):
        return "translated: " + text

    def on_translated_callback(english, hwnd):
        on_translated.append((english, hwnd))

    box = InputBox(root, fake_translate, ui_queue, on_translated_callback)

    # Simulate show()
    box._win = tk.Toplevel(root)  # fake window to pass is not None check
    box._session += 1
    session_started = box._session
    assert session_started == 1

    # Simulate Esc cancellation
    box._session += 1
    session_cancelled = box._session
    assert session_cancelled == 2

    # Simulate queued _finish with stale session
    box._finish("english text", None, session_started)

    assert on_translated == [], "Stale result should be discarded"


def _status(win, text):
    """測試直接替換的狀態列：與 InputBox 用的是同一種元件（RichLabel）。"""
    from src.ui.richtext import RichLabel
    label = RichLabel(win, fg="#9a9aa8", bg="#1a1a24", font=("Segoe UI", 9))
    label.set(text)
    return label


def test_error_message_urls_are_clickable(root):
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None)
    box.show()
    try:
        box._show_error("HTTP 401: see https://a.example/keys for a key", box._session)
        assert box._status.links() == [("https://a.example/keys", "https://a.example/keys")]
    finally:
        box.close()


def test_finish_over_limit_keeps_window_and_blocks_send(root):
    from src.ui.input_box import GAME_INPUT_MAX_CHARS
    sent = []
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: sent.append(a))
    box.show()
    session = box._session
    box._finish("x" * (GAME_INPUT_MAX_CHARS + 1), None, session)
    assert box._win is not None          # 不關閉，讓使用者刪減重送
    assert sent == []                    # 不鍵入遊戲
    assert str(GAME_INPUT_MAX_CHARS + 1) in box._status.text()
    assert str(box._entry.cget("state")) == "normal"  # 輸入欄恢復可編輯
    box.close()


def test_finish_within_limit_sends_and_closes(root):
    from src.ui.input_box import GAME_INPUT_MAX_CHARS
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

    # Simulate show()
    box._win = tk.Toplevel(root)
    box._session += 1
    session_started = box._session
    assert session_started == 1

    # Simulate successful _finish with matching session (no close called first)
    box._finish("english text", 12345, session_started)

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

    box._win = tk.Toplevel(root)
    box._entry = tk.Entry(box._win)
    box._status = _status(box._win, "test")
    box._session += 1
    session_started = box._session

    # Simulate cancellation
    box._session += 1

    # Simulate error with stale session
    box._show_error("Error message", session_started)

    # Entry and status stay untouched because the session was stale
    assert box._status.text() == "test"
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

    box._win = tk.Toplevel(root)
    box._entry = tk.Entry(box._win)
    box._entry.configure(state="disabled")
    box._status = _status(box._win, "original")
    box._session += 1
    current_session = box._session

    box._show_error("boom", current_session)

    assert box._status.text() == "boom"
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
    box._status = _status(box._win, "original")
    session = box._session

    box._worker("你好", None, session)  # 直接呼叫：走 except 分支、排入錯誤回呼
    callback = ui_queue.get_nowait()
    callback()  # 修正前此處會 NameError: name 'exc' is not defined

    assert "boom" in box._status.text()


_AREA = (0, 0, 1920, 1040)  # 工作區（去掉工作列）


def test_anchored_position_sits_below_the_anchor():
    assert anchored_position((749, 893, 732, 44), 460, 84, _AREA, gap=4) == (749, 941)


def test_anchored_position_flips_above_when_no_room_below():
    # 遊戲全螢幕時聊天輸入框貼底：下方放不下就翻到上方
    assert anchored_position((749, 1000, 732, 44), 460, 84, _AREA, gap=4) == (749, 912)


def test_anchored_position_clamps_into_the_work_area():
    assert anchored_position((1800, -50, 732, 44), 460, 84, _AREA, gap=4) == (1460, 0)


# Windows 11、96 DPI 實測：外框左側 7 px 隱形邊框；可見寬＝client 寬 + 2、可見高＝client 高 + 32
_CHROME = (7, 0, 2, 32)


@pytest.mark.real_position
def test_show_places_the_box_below_the_anchor(root, monkeypatch):
    monkeypatch.setattr(input_box_module, "work_area_at", lambda x, y: _AREA)
    monkeypatch.setattr(input_box_module, "visible_chrome", lambda hwnd: _CHROME)
    box = InputBox(root, lambda t: t, queue.Queue(), lambda e, h: None)
    box.set_anchor((300, 400, 500, 40))
    box.show()
    box._win.update_idletasks()
    # 看得見的外框要與錨點同寬、左緣對齊：client 寬扣掉可見邊框，x 往左補隱形邊框
    assert box._win.geometry().startswith("498x")
    assert box._win.geometry().endswith(f"+293+{400 + 40 + input_box_module.ANCHOR_GAP}")
    box.close()


@pytest.mark.real_position
def test_show_flips_above_using_the_visible_frame_height(root, monkeypatch):
    # 翻到上方時要用含標題列的可見高度，否則標題列會蓋到遊戲輸入框
    monkeypatch.setattr(input_box_module, "work_area_at", lambda x, y: _AREA)
    monkeypatch.setattr(input_box_module, "visible_chrome", lambda hwnd: _CHROME)
    box = InputBox(root, lambda t: t, queue.Queue(), lambda e, h: None)
    box.set_anchor((300, 1000, 500, 40))
    box.show()
    box._win.update_idletasks()
    visible_h = box._win.winfo_reqheight() + 32
    assert box._win.geometry().endswith(f"+293+{1000 - input_box_module.ANCHOR_GAP - visible_h}")
    box.close()


_CURSOR = (640, 500)


@pytest.mark.real_position
def test_show_without_anchor_sits_at_the_cursor(root, monkeypatch):
    # 遊戲聊天框沒開、按熱鍵：左上角貼在游標右下方，寬度用預設
    monkeypatch.setattr(input_box_module, "work_area_at", lambda x, y: _AREA)
    monkeypatch.setattr(input_box_module, "visible_chrome", lambda hwnd: _CHROME)
    monkeypatch.setattr(input_box_module, "cursor_position", lambda: _CURSOR)
    box = InputBox(root, lambda t: t, queue.Queue(), lambda e, h: None)
    box.show()
    box._win.update_idletasks()
    assert box._win.geometry().startswith("460x")
    assert box._win.geometry().endswith(f"+{640 - 7}+{500 + input_box_module.ANCHOR_GAP}")
    box.close()


@pytest.mark.real_position
def test_clear_anchor_makes_show_use_the_cursor(root, monkeypatch):
    # 遊戲聊天框關掉後錨點就失效，不能沿用舊位置
    monkeypatch.setattr(input_box_module, "work_area_at", lambda x, y: _AREA)
    monkeypatch.setattr(input_box_module, "visible_chrome", lambda hwnd: _CHROME)
    monkeypatch.setattr(input_box_module, "cursor_position", lambda: _CURSOR)
    box = InputBox(root, lambda t: t, queue.Queue(), lambda e, h: None)
    box.set_anchor((300, 400, 500, 40))
    box.clear_anchor()
    box.show()
    box._win.update_idletasks()
    assert box._win.geometry().endswith(f"+{640 - 7}+{500 + input_box_module.ANCHOR_GAP}")
    box.close()


@pytest.mark.real_position
def test_anchor_set_while_open_applies_on_the_next_show(root, monkeypatch):
    monkeypatch.setattr(input_box_module, "work_area_at", lambda x, y: _AREA)
    monkeypatch.setattr(input_box_module, "visible_chrome", lambda hwnd: _CHROME)
    monkeypatch.setattr(input_box_module, "cursor_position", lambda: _CURSOR)
    box = InputBox(root, lambda t: t, queue.Queue(), lambda e, h: None)
    box.show()
    box.set_anchor((300, 400, 500, 40))
    box.close()
    box.show()
    box._win.update_idletasks()
    assert box._win.geometry().endswith(f"+293+{400 + 40 + input_box_module.ANCHOR_GAP}")
    box.close()


def test_show_uses_the_anchor_width(root, monkeypatch):
    # 寬度跟著遊戲輸入框：探測值 1920×1080 下容器寬 732 px；client 寬扣掉可見邊框
    monkeypatch.setattr(input_box_module, "work_area_at", lambda x, y: _AREA)
    monkeypatch.setattr(input_box_module, "visible_chrome", lambda hwnd: _CHROME)
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None)
    box.set_anchor((300, 400, 732, 44))
    box.show()
    box._win.update_idletasks()
    assert box._win.winfo_width() == 730
    box.close()


@pytest.mark.real_position
def test_shown_box_visible_frame_matches_the_anchor(root, monkeypatch):
    """不假造邊框：用 DWM 讀回實際可見邊界，左緣與寬度都要和錨點一致。
    （邊框要在 HWND 套上樣式後才量得到，太早量會全是 0 而看似對齊）"""
    import ctypes
    from ctypes import wintypes

    from src.ui.winstyle import root_hwnd
    monkeypatch.setattr(input_box_module, "work_area_at", lambda x, y: (9000, 9000, 4000, 3000))
    monkeypatch.setattr(input_box_module, "force_foreground", lambda hwnd: None)
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None)
    box.set_anchor((10000, 10000, 732, 44))
    box.show()
    box._win.update_idletasks()
    frame = wintypes.RECT()
    ctypes.windll.dwmapi.DwmGetWindowAttribute(root_hwnd(box._win), 9, ctypes.byref(frame),
                                               ctypes.sizeof(frame))
    box.close()
    assert (frame.left, frame.right - frame.left) == (10000, 732)
    assert frame.top == 10000 + 44 + input_box_module.ANCHOR_GAP


@pytest.mark.real_position
def test_box_placed_above_the_anchor_grows_upward(root, monkeypatch):
    # 放在錨點上方時，提示／錯誤文字換行長高要往上長、底邊釘住，否則會蓋到遊戲輸入框
    monkeypatch.setattr(input_box_module, "work_area_at", lambda x, y: _AREA)
    monkeypatch.setattr(input_box_module, "visible_chrome", lambda hwnd: _CHROME)
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None)
    box.set_anchor((300, 1000, 500, 40))
    box.show()
    box._win.update()
    height = box._win.winfo_height()
    bottom = box._win.winfo_y() + height
    box._status.set("很長的錯誤訊息 " * 30)
    box._win.update()  # RichLabel 排到 idle 才量行數並回呼 _fit_height
    assert box._win.winfo_height() > height
    assert box._win.winfo_y() + box._win.winfo_height() == bottom
    box.close()


def test_visible_chrome_measures_the_frame_of_a_hidden_window(root):
    # 視窗還沒顯示就要量得到，show() 才能在定位前扣掉邊框
    from src.ui.winstyle import root_hwnd, visible_chrome
    win = tk.Toplevel(root)
    win.withdraw()
    win.geometry("400x100")
    win.update_idletasks()
    left, top, extra_w, extra_h = visible_chrome(root_hwnd(win))
    win.destroy()
    assert left >= 0 and top >= 0 and extra_w >= 0
    assert extra_h > extra_w  # 高度多出的是標題列，一定比左右邊框厚


def test_show_clamps_the_anchor_width_to_minimum(root, monkeypatch):
    from src.ui.input_box import MIN_WIDTH
    monkeypatch.setattr(input_box_module, "work_area_at", lambda x, y: _AREA)
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None)
    box.set_anchor((300, 400, 200, 44))
    box.show()
    box._win.update_idletasks()
    assert box._win.winfo_width() == MIN_WIDTH
    box.close()


def test_show_without_any_anchor_uses_the_default_width(root, monkeypatch):
    from src.ui.input_box import DEFAULT_WIDTH
    monkeypatch.setattr(input_box_module, "cursor_position", lambda: _CURSOR)
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None)
    box.show()
    box._win.update_idletasks()
    assert box._win.winfo_width() == DEFAULT_WIDTH
    box.close()


def test_fit_height_shrinks_when_the_status_needs_fewer_lines(root):
    # 狀態文字從多行換回一行 → 高度要跟著貼回內容，不能停在較高的那次
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None)
    box.show()
    box._win.update()
    box._status.set("很長的訊息 " * 30)
    box._win.update()
    taller = box._win.winfo_height()
    box._status.set("短")
    box._win.update()
    assert box._win.winfo_height() < taller
    assert box._win.winfo_height() == box._win.winfo_reqheight()
    box.close()


def test_box_is_not_resizable(root):
    box = InputBox(root, lambda t: t, queue.Queue(), lambda *a: None)
    box.show()
    assert tuple(int(v) for v in box._win.resizable()) == (0, 0)
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
        box._status = _status(box._win, "original")
        box._worker("hello", None, box._session)
        ui_queue.get_nowait()()
        assert box._status.text().startswith("Translation failed:")
    finally:
        i18n.set_language(before)


def test_is_open_tracks_window_lifecycle(root):
    box = InputBox(root, lambda text: text, queue.Queue(), lambda *_: None)
    assert not box.is_open
    box._win = tk.Toplevel(root)
    assert box.is_open
    box._win.destroy()
    box._win = None
    assert not box.is_open


def test_outgoing_translation_failure_is_logged(root, monkeypatch):
    # 收訊失敗由 pool 記錄；發話失敗只顯示在輸入框，app.log 得留一行才查得到
    from src.ui import input_box as input_box_module
    logged = []
    monkeypatch.setattr(input_box_module, "log", logged.append)

    def failing(text):
        raise RuntimeError("HTTP 500: upstream down")

    box = InputBox(root, failing, queue.Queue(), lambda *a: None)
    box._worker("hello", None, box._session)
    assert any("outgoing translation failed" in line and "upstream down" in line
               for line in logged)
