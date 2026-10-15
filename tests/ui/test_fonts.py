"""介面字型：字族依介面語言的文字系統推導。"""
import pytest

from src import i18n
from src.ui.fonts import font_family, ui_font


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


@pytest.mark.parametrize("code, family", [
    ("zh-TW", "Microsoft JhengHei"),
    ("zh-CN", "Microsoft YaHei"),
    ("ja-JP", "Yu Gothic UI"),
    ("ko-KR", "Malgun Gothic"),
    ("de-DE", "Segoe UI"),      # 拉丁、斯拉夫、希臘等由 Segoe UI 保底
    ("ru-RU", "Segoe UI"),
    ("xx-XX", "Segoe UI"),      # 認不得的語言標籤
])
def test_font_family_comes_from_the_script(code, family):
    # 字族不必等語言檔宣告：還沒有語言檔的語言（日文、韓文）也推導得出來
    assert font_family(code) == family


def test_font_accepts_weight():
    i18n.set_language("zh-TW")
    assert ui_font(10, "bold") == ("Microsoft JhengHei", 10, "bold")
