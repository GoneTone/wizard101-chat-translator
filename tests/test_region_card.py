"""結果卡片：三態文字、點擊關閉、貼在框選矩形正下方。"""
import tkinter as tk

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


def test_show_stage_replaces_the_pending_text(card, root):
    card.show_pending(_RECT)
    root.update()
    card.show_stage(t("region.recognizing"))
    root.update()
    assert card.text() == t("region.recognizing")
    assert card._label.cget("fg") == FG_PENDING


def test_show_text_with_source_puts_both_in_one_selectable_label(card, root):
    # 原文與譯文現在放同一顆 RichLabel（set_blocks），中間空一行分隔 —— 拖曳選取
    # 才能一路跨過去，不會卡在兩段文字的交界（見 richtext.RichLabel.set_blocks）
    card.show_pending(_RECT)
    root.update()
    card.show_text("譯文", source="Talk to Merle")
    root.update()
    assert card._label.get("1.0", "end-1c") == "Talk to Merle\n\n譯文"
    assert card.source_text() == "Talk to Merle"
    assert card.text() == "譯文"


def test_show_text_without_source_shows_only_the_translation(card, root):
    card.show_pending(_RECT)
    root.update()
    card.show_text("譯文", source="Talk to Merle")
    root.update()
    card.show_text("譯文")
    root.update()
    assert card.source_text() == ""
    assert card._label.get("1.0", "end-1c") == "譯文"


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
    card._label.event_generate("<ButtonPress-1>", x=5, y=5)
    card._label.event_generate("<ButtonRelease-1>", x=5, y=5)
    root.update()
    assert not card.is_open


def test_close_affordance_is_shown(card, root):
    card.show_pending(_RECT)
    root.update()
    assert card._close.cget("text") == "✕"
    assert card._hint.cget("text") == t("region.close_hint")
    assert card.text() == t("notice.pending")   # 提示文字不能混進主要內容


def test_close_button_tooltip_shows_close_text(card, root):
    card.show_pending(_RECT)
    root.update()

    card._close_tooltip._show()  # 直接觸發顯示，不必真的等懸停

    assert card._close_tooltip.text() == t("tooltip.close")


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


def test_right_click_on_translation_shows_the_copy_menu(card, root):
    card.show_pending(_RECT)
    root.update()
    card.show_text("譯文", source="Talk to Merle")
    root.update()

    card._label.event_generate("<Button-3>", x=5, y=5)
    root.update()

    assert card._popup.visible is True
    assert card._popup.label_text() == t("menu.copy")


def test_right_click_does_not_close_the_card(card, root):
    card.show_pending(_RECT)
    root.update()
    card.show_text("譯文", source="Talk to Merle")
    root.update()

    card._label.event_generate("<Button-3>", x=5, y=5)
    root.update()
    assert card.is_open

    card._label.event_generate("<ButtonPress-1>", x=5, y=5)
    card._label.event_generate("<ButtonRelease-1>", x=5, y=5)
    root.update()
    assert not card.is_open


def test_click_without_drag_closes_the_card(card, root):
    card.show_pending(_RECT)
    root.update()
    card.show_text("很長很長的譯文內容", source="Talk to Merle")
    root.update()

    card._label.event_generate("<ButtonPress-1>", x=2, y=2)
    card._label.event_generate("<ButtonRelease-1>", x=3, y=2)
    root.update()

    assert not card.is_open


def test_drag_on_the_translation_label_selects_instead_of_closing(card, root, monkeypatch):
    monkeypatch.setattr(card, "_focus_for_copy", lambda: None)
    card.show_pending(_RECT)
    root.update()
    card.show_text("很長很長的譯文內容，足夠拖出一段選取範圍出來測試", source="Talk to Merle")
    root.update()

    card._label.event_generate("<ButtonPress-1>", x=2, y=2)
    root.update()
    card._label.event_generate("<B1-Motion>", x=80, y=2)
    root.update()
    card._label.event_generate("<ButtonRelease-1>", x=80, y=2)
    root.update()

    # Tk 是否真的透過合成事件跑完原生框選因平台而異，這裡只驗證「拖曳不關卡片」；
    # 選取內容另外用 copy_selection 的測試（顯式 tag_add）驗證。
    assert card.is_open


def test_copy_selection_and_menu_copy_share_the_clipboard(card, root):
    # 所有會動到剪貼簿的斷言合成同一個測試：剪貼簿是全機器共用的資源，拆成多個
    # 測試在 pytest-xdist 的 4 個 worker 下會彼此覆蓋（見 addopts 的 -n 4）
    card.show_pending(_RECT)
    root.update()
    card.show_text("譯文第一行", source="Talk to Merle")
    root.update()
    # widget 內容此刻是 "Talk to Merle\n\n譯文第一行"：第 1 行原文、第 3 行譯文

    card._label.tag_add("sel", "3.0", "3.2")
    card.copy_selection()
    try:
        assert root.clipboard_get() == "譯文"
    except tk.TclError:
        pytest.skip("clipboard unavailable in this environment")

    card._label.tag_remove("sel", "1.0", "end")
    root.clipboard_clear()
    root.clipboard_append("哨兵內容")
    root.update()
    card.copy_selection()
    assert root.clipboard_get() == "哨兵內容", "沒有選取時不該動剪貼簿"

    card._label.tag_add("sel", "1.0", "end-1c")
    card.copy_selection()
    assert root.clipboard_get() == "Talk to Merle\n\n譯文第一行", "選取要跨得過原文與譯文"

    card._label.event_generate("<Button-3>", x=5, y=5)
    root.update()
    card._copy()
    root.update()
    assert root.clipboard_get() == "Talk to Merle\n\n譯文第一行", "右鍵複製要優先用選取範圍"
    assert card._popup.visible is False

    card._label.tag_remove("sel", "1.0", "end")
    card._label.event_generate("<Button-3>", x=5, y=5)
    root.update()
    card._copy()
    root.update()
    assert root.clipboard_get() == "Talk to Merle\n\n譯文第一行", "沒有選取時複製整顆內容"


def test_focus_for_copy_swallows_a_tcl_error_when_the_window_is_gone(card, root):
    ghost = tk.Toplevel(root)
    ghost.withdraw()
    ghost.destroy()
    card._win = ghost

    card._focus_for_copy()   # 不該丟例外


def test_control_c_entry_points_are_bound(card, root):
    # Control-c 靠 event_generate 模擬鍵盤不可靠（合成事件不一定真的送進 Text
    # 的 bindtags，實測在這個 Tk 版本上就是不會觸發），改比照 overlay 的
    # test_selection_entry_points_are_bound：只驗證綁定確實掛著。
    card.show_pending(_RECT)
    root.update()

    assert card._win.bind("<Control-c>")
    assert card._win.bind("<Control-C>")
    assert card._label.bind("<Control-c>")
    assert card._label.bind("<Control-C>")


def test_selection_stays_visible_without_keyboard_focus(card, root):
    card.show_pending(_RECT)
    card.show_text("譯文", source="Talk to Merle")
    root.update()
    label = card._label
    assert label.cget("inactiveselectbackground") == label.cget("selectbackground")
