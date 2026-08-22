"""設定視窗純邏輯與 overlay set_limits／set_alpha 測試。"""
import tkinter as tk

from src.reader.overlay import OverlayWindow
from src.config import clamp_advanced
from src.ui.settings import parse_advanced_values


def test_clamp_advanced_limits_ranges():
    v = clamp_advanced({"poll_interval": 0.01, "fade_seconds": -5,
                        "max_messages": 99999, "type_delay": 9.0,
                        "overlay_alpha": 0.01})
    assert v == {"poll_interval": 0.1, "fade_seconds": 0,
                 "max_messages": 1000, "type_delay": 0.5,
                 "overlay_alpha": 0.3}


def test_clamp_advanced_passes_valid_values():
    v = {"poll_interval": 0.4, "fade_seconds": 0, "max_messages": 200,
         "type_delay": 0.02, "overlay_alpha": 0.84}
    assert clamp_advanced(dict(v)) == v


def test_overlay_set_limits_trims_messages(root):
    ov = OverlayWindow(root, x=0, y=0, max_messages=5, fade_seconds=0)
    for i in range(5):
        ov.add_message(f"o{i}", f"t{i}", now=100.0)
    ov.set_limits(max_messages=3, fade_seconds=0)
    assert len(ov.visible_messages()) == 3
    assert ov.visible_messages()[0] == ("o2", "t2")  # 移除最舊


def test_overlay_set_alpha_applies_to_window_and_bubble(root):
    ov = OverlayWindow(root, x=0, y=0, max_messages=5, fade_seconds=0, alpha=0.84)
    ov.minimize()
    ov.set_alpha(0.5)
    assert float(ov._win.attributes("-alpha")) == 0.5
    assert float(ov._bubble.attributes("-alpha")) == 0.5
    ov.expand()


def _vars(root, poll=0.4, fade=0, max_msgs=200, type_delay=0.02, alpha=0.84):
    return (tk.DoubleVar(root, value=poll), tk.IntVar(root, value=fade),
            tk.IntVar(root, value=max_msgs), tk.DoubleVar(root, value=type_delay),
            tk.DoubleVar(root, value=alpha))


def test_parse_advanced_values_returns_error_on_non_numeric_input(root):
    poll, fade, max_msgs, type_delay, alpha = _vars(root)
    poll.set("abc")  # 模擬使用者在 Spinbox 手動鍵入非數字

    values, error = parse_advanced_values(poll, fade, max_msgs, type_delay, alpha)

    assert values is None
    assert error == "進階數值格式錯誤，請輸入數字"


def test_parse_advanced_values_clamps_valid_numeric_input(root):
    poll, fade, max_msgs, type_delay, alpha = _vars(
        root, poll=0.01, fade=-5, max_msgs=99999, type_delay=9.0, alpha=2.0)

    values, error = parse_advanced_values(poll, fade, max_msgs, type_delay, alpha)

    assert error is None
    assert values == {"poll_interval": 0.1, "fade_seconds": 0,
                      "max_messages": 1000, "type_delay": 0.5,
                      "overlay_alpha": 1.0}
