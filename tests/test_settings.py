"""設定視窗純邏輯與 overlay set_limits 測試。"""
from src.reader.overlay import OverlayWindow
from src.ui.settings import clamp_advanced


def test_clamp_advanced_limits_ranges():
    v = clamp_advanced({"poll_interval": 0.01, "fade_seconds": -5,
                        "max_messages": 99999, "type_delay": 9.0})
    assert v == {"poll_interval": 0.1, "fade_seconds": 0,
                 "max_messages": 1000, "type_delay": 0.5}


def test_clamp_advanced_passes_valid_values():
    v = {"poll_interval": 0.4, "fade_seconds": 0, "max_messages": 200, "type_delay": 0.02}
    assert clamp_advanced(dict(v)) == v


def test_overlay_set_limits_trims_messages(root):
    ov = OverlayWindow(root, x=0, y=0, max_messages=5, fade_seconds=0)
    for i in range(5):
        ov.add_message(f"o{i}", f"t{i}", now=100.0)
    ov.set_limits(max_messages=3, fade_seconds=0)
    assert len(ov.visible_messages()) == 3
    assert ov.visible_messages()[0] == ("o2", "t2")  # 移除最舊
