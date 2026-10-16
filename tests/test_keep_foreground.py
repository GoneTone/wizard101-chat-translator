"""conftest 的 `_keep_foreground`：測試期間每個綁了 force_foreground 的模組都拿到無操作版本。"""
import importlib

from tests.conftest import FOREGROUND_MODULES


def test_the_scan_finds_the_modules_that_raise_windows():
    assert {"src.composer.paste", "src.ui.input_box", "src.ui.settings.form"} <= set(
        FOREGROUND_MODULES)


def test_every_module_binding_force_foreground_is_stubbed_during_tests():
    for name in FOREGROUND_MODULES:
        module = importlib.import_module(name)
        assert module.force_foreground.__name__ == "_noop", name


def test_tk_focus_force_cannot_take_the_foreground_during_tests():
    """Tk 的 focus_force 在使用者閒置時拿得到 Windows 前景：測試期間要換成只在程式內移動焦點的 focus_set。"""
    import tkinter as tk
    assert tk.Misc.focus_force is tk.Misc.focus_set


def test_every_module_binding_cursor_position_gets_the_fixed_cursor():
    """輸入框與框選流程會讀游標：測試期間一律拿到固定點，不取決於開發者當下的滑鼠。"""
    from tests.conftest import CURSOR_MODULES, FIXED_CURSOR
    assert {"src.ui.input_box", "src.ui.region.flow"} <= set(CURSOR_MODULES)
    for name in CURSOR_MODULES:
        assert importlib.import_module(name).cursor_position() == FIXED_CURSOR, name
