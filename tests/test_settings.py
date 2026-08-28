"""設定視窗純邏輯與 overlay set_limits／set_alpha 測試。"""
import copy
import tkinter as tk

from src.reader.overlay import OverlayWindow
from src.config import clamp_advanced
from src.ui.settings import parse_advanced_values


def test_clamp_advanced_limits_ranges():
    v = clamp_advanced({"poll_interval": 0.01, "fade_seconds": -5,
                        "max_messages": 99999, "type_delay": 9.0,
                        "overlay_alpha": 0.01, "max_parallel_translations": 99})
    assert v == {"poll_interval": 0.1, "fade_seconds": 0,
                 "max_messages": 1000, "type_delay": 0.5,
                 "overlay_alpha": 0.3, "max_parallel_translations": 8}


def test_clamp_advanced_passes_valid_values():
    v = {"poll_interval": 0.4, "fade_seconds": 0, "max_messages": 200,
         "type_delay": 0.02, "overlay_alpha": 0.84, "max_parallel_translations": 4}
    assert clamp_advanced(dict(v)) == v


def test_overlay_set_limits_trims_messages(root):
    ov = OverlayWindow(root, x=0, y=0, max_messages=5, fade_seconds=0)
    for i in range(5):
        ov.add_message(f"o{i}", f"t{i}", now=100.0)
    ov.set_limits(max_messages=3, fade_seconds=0)
    assert len(ov.visible_messages()) == 3
    assert ov.visible_messages()[0] == ("o2", "t2")  # 移除最舊


def test_overlay_set_alpha_applies_to_backdrop_and_bubble(root):
    # 雙層視窗：透明度只套在底板（backdrop）與泡泡，文字層（_win）維持不透明
    ov = OverlayWindow(root, x=0, y=0, max_messages=5, fade_seconds=0, alpha=0.84)
    ov.minimize()
    ov.set_alpha(0.5)
    assert float(ov._backdrop.attributes("-alpha")) == 0.5
    assert float(ov._win.attributes("-alpha")) == 1.0
    assert float(ov._bubble.attributes("-alpha")) == 0.5
    ov.expand()


def test_overlay_backdrop_follows_geometry(root):
    ov = OverlayWindow(root, x=30, y=40, width=460, height=300,
                       max_messages=5, fade_seconds=0)
    ov._apply_geometry(120, 90, 400, 250)
    root.update()
    assert (ov._backdrop.winfo_x(), ov._backdrop.winfo_y()) == (120, 90)
    assert (ov._backdrop.winfo_width(), ov._backdrop.winfo_height()) == (400, 250)


def _vars(root, poll=0.4, fade=0, max_msgs=200, type_delay=0.02, alpha=0.84, parallel=4):
    return (tk.DoubleVar(root, value=poll), tk.IntVar(root, value=fade),
            tk.IntVar(root, value=max_msgs), tk.DoubleVar(root, value=type_delay),
            tk.DoubleVar(root, value=alpha), tk.IntVar(root, value=parallel))


def test_parse_advanced_values_returns_error_on_non_numeric_input(root):
    poll, fade, max_msgs, type_delay, alpha, parallel = _vars(root)
    poll.set("abc")  # 模擬使用者在 Spinbox 手動鍵入非數字

    values, error = parse_advanced_values(poll, fade, max_msgs, type_delay, alpha, parallel)

    assert values is None
    assert error == "error.advanced_not_number"


def test_parse_advanced_values_clamps_valid_numeric_input(root):
    poll, fade, max_msgs, type_delay, alpha, parallel = _vars(
        root, poll=0.01, fade=-5, max_msgs=99999, type_delay=9.0, alpha=2.0)

    values, error = parse_advanced_values(poll, fade, max_msgs, type_delay, alpha, parallel)

    assert error is None
    assert values == {"poll_interval": 0.1, "fade_seconds": 0,
                      "max_messages": 1000, "type_delay": 0.5,
                      "overlay_alpha": 1.0, "max_parallel_translations": 4}


def test_parse_advanced_values_clamps_parallel(root):
    poll, fade, max_msgs, type_delay, alpha, parallel = _vars(root, parallel=99)
    values, error = parse_advanced_values(poll, fade, max_msgs, type_delay, alpha, parallel)
    assert error is None
    assert values["max_parallel_translations"] == 8


def test_parse_advanced_values_returns_error_key():
    import tkinter as tk

    from src.ui.settings import parse_advanced_values

    class BadVar:
        def get(self):
            raise ValueError("not a number")

    values, error = parse_advanced_values(BadVar(), BadVar(), BadVar(), BadVar(),
                                          BadVar(), BadVar())
    assert values is None
    assert error == "error.advanced_not_number"


def test_save_applies_ui_language(root, tmp_path):
    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.ui.settings import SettingsWindow

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        cfg["api"] = {"provider": "custom", "base_url": "http://x", "model": "m",
                      "api_key": "", "thinking": False}
        saved = []
        win = SettingsWindow(root, cfg,
                             on_save=lambda: saved.append(i18n.current_language()))
        win.open()
        win._ui_language.set_value("en")
        win._save()
        assert cfg["ui_language"] == "en"
        assert i18n.current_language() == "en"
        # on_save 看到的必須已經是新語言：這行是本測試的重點，
        # 若 _save() 把 on_save 提前到套用語言之前，這裡會是 "zh-TW"。
        assert saved == ["en"]
    finally:
        i18n.set_language(before)
