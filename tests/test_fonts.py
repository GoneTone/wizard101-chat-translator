"""介面字型：字族隨介面語言切換。"""
import pytest

from src import i18n
from src.ui.fonts import ui_font


@pytest.fixture(autouse=True)
def restore_language():
    before = i18n.current_language()
    yield
    i18n.set_language(before)


def test_font_family_follows_language():
    i18n.set_language("zh-TW")
    assert ui_font(9) == ("Microsoft JhengHei", 9)
    i18n.set_language("zh-CN")
    assert ui_font(9) == ("Microsoft YaHei", 9)
    i18n.set_language("en-US")
    assert ui_font(9) == ("Segoe UI", 9)


def test_font_accepts_weight():
    i18n.set_language("zh-TW")
    assert ui_font(10, "bold") == ("Microsoft JhengHei", 10, "bold")
