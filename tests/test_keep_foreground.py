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
