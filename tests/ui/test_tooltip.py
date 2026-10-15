"""懸停提示：延遲顯示、離開即收起、文字每次顯示才現取。"""
import time
import tkinter as tk

import pytest

from src.ui.tooltip import Tooltip

_DELAY_MS = 1


def _pump_until_visible(widget: tk.Widget, tip: Tooltip, timeout: float = 5.0) -> None:
    """反覆 `update()` 直到延遲的 `after` 觸發顯示；比照 test_settings.py 的忙迴圈寫法，
    夾一個真實的 sleep 讓出 CPU，純 update() 忙等在 xdist 平行跑測試時可能長時間空轉。"""
    deadline = time.monotonic() + timeout
    while not tip.visible and time.monotonic() < deadline:
        widget.update()
        time.sleep(0.005)


@pytest.fixture
def label(root):
    # <Enter>／<Leave> 是真正的滑鼠穿越事件，root fixture 本身是 withdraw 的隱藏視窗，
    # 掛在它底下的控件收不到；另開一個（會被 conftest 停到螢幕外但保持 mapped 的）
    # Toplevel 才收得到，跟 OverlayWindow 的標題列按鈕、region card 的關閉鈕實際情境一致。
    win = tk.Toplevel(root)
    w = tk.Label(win, text="⛶")
    w.pack()
    win.update()
    yield w
    win.destroy()


def test_hovering_shows_the_tooltip_after_a_delay(label, root):
    tip = Tooltip(label, lambda: "文字", delay_ms=_DELAY_MS)
    assert tip.visible is False

    label.event_generate("<Enter>")
    _pump_until_visible(label, tip)

    assert tip.visible is True
    assert tip.text() == "文字"


def test_leaving_hides_the_tooltip(label, root):
    tip = Tooltip(label, lambda: "文字", delay_ms=_DELAY_MS)
    label.event_generate("<Enter>")
    _pump_until_visible(label, tip)
    assert tip.visible is True

    label.event_generate("<Leave>")
    root.update()

    assert tip.visible is False


def test_leaving_before_the_delay_elapses_cancels_the_show(label, root):
    tip = Tooltip(label, lambda: "文字", delay_ms=50)
    label.event_generate("<Enter>")
    label.event_generate("<Leave>")
    root.update()

    assert tip.visible is False
    assert tip.text() == ""


def test_text_fn_is_re_evaluated_on_each_show(label, root):
    current = ["第一次"]
    tip = Tooltip(label, lambda: current[0], delay_ms=_DELAY_MS)

    label.event_generate("<Enter>")
    _pump_until_visible(label, tip)
    assert tip.text() == "第一次"

    label.event_generate("<Leave>")
    root.update()
    current[0] = "第二次"
    label.event_generate("<Enter>")
    _pump_until_visible(label, tip)

    assert tip.text() == "第二次"


def test_button_press_cancels_and_hides(label, root):
    tip = Tooltip(label, lambda: "文字", delay_ms=_DELAY_MS)
    label.event_generate("<Enter>")
    _pump_until_visible(label, tip)
    assert tip.visible is True

    label.event_generate("<ButtonPress-1>")
    root.update()

    assert tip.visible is False


def test_tooltip_never_takes_keyboard_focus(label, root):
    tip = Tooltip(label, lambda: "文字", delay_ms=_DELAY_MS)
    before = root.focus_get()

    label.event_generate("<Enter>")
    _pump_until_visible(label, tip)

    assert root.focus_get() is before
