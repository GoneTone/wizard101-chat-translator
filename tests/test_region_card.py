"""結果卡片：三態文字、點擊關閉、貼在框選矩形正下方。"""
import pytest

from src.i18n import t
from src.ui import region_card as card_module
from src.ui.palette import FG_ERROR, FG_PENDING, FG_TRANSLATED
from src.ui.region_card import ANCHOR_GAP, RegionCard

_RECT = (300, 200, 400, 120)
_AREA = (0, 0, 1920, 1040)


@pytest.fixture
def card(root, monkeypatch):
    monkeypatch.setattr(card_module, "work_area_at", lambda x, y: _AREA)
    c = RegionCard(root, alpha=0.8)
    yield c
    c.hide()


def test_pending_then_text(card, root):
    assert not card.is_open
    card.show_pending(_RECT)
    root.update()
    assert card.is_open and card.text() == t("notice.pending")
    assert card._label.cget("fg") == FG_PENDING
    card.show_text("譯文第一行\n第二行")
    root.update()
    assert card.text() == "譯文第一行\n第二行"
    assert card._label.cget("fg") == FG_TRANSLATED


def test_empty_text_shows_the_no_text_notice(card, root):
    card.show_pending(_RECT)
    card.show_text("")
    root.update()
    assert card.text() == t("region.no_text")
    assert card._label.cget("fg") == FG_PENDING


def test_error_is_red(card, root):
    card.show_pending(_RECT)
    card.show_error("HTTP 401: bad key")
    root.update()
    assert card.text() == "HTTP 401: bad key"
    assert card._label.cget("fg") == FG_ERROR


def test_click_anywhere_hides(card, root):
    card.show_pending(_RECT)
    root.update()
    card._label.event_generate("<Button-1>", x=5, y=5)
    root.update()
    assert not card.is_open


def test_close_affordance_is_shown(card, root):
    card.show_pending(_RECT)
    root.update()
    assert card._close.cget("text") == "✕"
    assert card._hint.cget("text") == t("region.close_hint")
    assert card.text() == t("notice.pending")   # 提示文字不能混進主要內容


def test_close_button_hides(card, root):
    card.show_pending(_RECT)
    root.update()
    card._close.event_generate("<Button-1>")
    root.update()
    assert not card.is_open


def test_hide_is_idempotent(card):
    card.hide()
    card.show_pending(_RECT)
    card.hide()
    card.hide()
    assert not card.is_open


def test_card_is_anchored_below_the_rect_with_the_rect_width(card, root, monkeypatch):
    calls = []

    def spy(anchor, w, h, area, gap):
        calls.append((anchor, w, area, gap))
        return 10000, 10000   # 停在螢幕外：測試期間不在畫面上畫任何東西
    monkeypatch.setattr(card_module, "anchored_position", spy)
    card.show_pending(_RECT)
    root.update()
    assert calls and calls[-1] == (_RECT, _RECT[2], _AREA, ANCHOR_GAP)
    assert card._win.winfo_width() == _RECT[2]


def test_narrow_rect_gets_the_minimum_width(card, root):
    card.show_pending((300, 200, 40, 20))
    root.update()
    assert card._win.winfo_width() == card_module.MIN_WIDTH
