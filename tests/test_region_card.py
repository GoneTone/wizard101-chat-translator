"""結果卡片：三態文字、點擊關閉、貼在框選矩形正下方。"""
import tkinter as tk
from types import SimpleNamespace

import pytest

from src.i18n import t
from src.ui import region_card as card_module
from src.ui.geometry import Placement, anchored_geometry, beside_geometry
from src.ui.palette import FG_ERROR, FG_PENDING, FG_TRANSLATED
from src.ui.region_card import ANCHOR_GAP, MIN_HEIGHT, SIDE_MIN_WIDTH, RegionCard

_RECT = (300, 200, 400, 120)
_AREA = (0, 0, 1920, 1040)
_SHORT_AREA = (0, 0, 1920, 300)   # 上下都塞不下長譯文的矮工作區


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
    assert card.is_open and card.text() == t("region.working")
    assert card._label.cget("fg") == FG_PENDING
    card.show_text("譯文第一行\n第二行")
    root.update()
    assert card.text() == "譯文第一行\n第二行"
    assert card._label.cget("fg") == FG_TRANSLATED


def test_show_text_with_source_puts_both_in_one_selectable_label(card, root):
    # 原文與譯文放同一顆 RichLabel（set_blocks），中間空一行分隔 —— 拖曳選取
    # 才能一路跨過去，不會卡在兩段文字的交界
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
    assert card.text() == t("region.working")   # 提示文字不能混進主要內容


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

    def spy(anchor, w, h, area, gap, side_w, min_w, min_h):
        calls.append((anchor, w, area, gap, min_w, min_h))
        return Placement(10000, 10000, w, h, "below")   # 停在螢幕外：測試期間不在畫面上畫任何東西
    monkeypatch.setattr(card_module, "anchored_geometry", spy)
    card.show_pending(_RECT)
    root.update()
    assert calls and calls[-1] == (_RECT, _RECT[2], _AREA, ANCHOR_GAP, SIDE_MIN_WIDTH, MIN_HEIGHT)
    assert card._win.winfo_width() == _RECT[2]


def test_card_below_the_rect_is_at_least_the_minimum_height(card, root):
    card.show_pending(_RECT)   # 「翻譯中…」只有一行，自然高度比下限矮
    root.update()
    assert card._win.winfo_height() == MIN_HEIGHT


def test_side_card_takes_the_text_width_instead_of_the_rect_width(card, root, monkeypatch):
    monkeypatch.setattr(card_module, "work_area_at", lambda x, y: _SHORT_AREA)
    card.show_pending((300, 100, 400, 100))
    root.update()
    card.show_text("這一行譯文很長，長到用矩形的寬度排會換成好幾行，" * 2)
    for _ in range(4):
        root.update()
    assert card._win.winfo_width() == card._natural_width() > 400


def test_side_card_stays_beside_the_rect_after_rewrapping_wider(card, root, monkeypatch):
    # 側邊用文字寬度排版後行數變少、高度變矮，若拿這個高度重判「下方放得下」就會
    # 跳回下方、再換行變高、再跳回側邊……落點要黏住直到內容換掉
    monkeypatch.setattr(card_module, "work_area_at", lambda x, y: (0, 0, 1920, 380))
    layouts = []
    real_layout = card._layout

    def budgeted_layout():
        layouts.append(card._side)
        if len(layouts) <= 20:   # 來回跳會無限重排、把 root.update() 卡死：超過預算就停
            real_layout()
    monkeypatch.setattr(card, "_layout", budgeted_layout)
    card.show_pending((300, 100, 400, 100))   # 下方 167px：矩形寬排六行放不下，側邊排兩行放得下
    root.update()
    card.show_text("這一行譯文很長，長到用矩形的寬度排會換成好幾行，" * 6)
    for _ in range(6):
        root.update()

    assert card._side == "right"
    assert len(layouts) < 20


def test_card_is_capped_to_the_work_area_and_scrolls_when_neither_side_fits(
        card, root, monkeypatch):
    monkeypatch.setattr(card_module, "work_area_at", lambda x, y: _SHORT_AREA)
    card.show_pending((300, 100, 400, 100))
    root.update()
    card.show_text("\n".join(f"第 {i} 行" for i in range(60)))
    root.update()
    assert card.is_open
    assert card._win.winfo_height() == _SHORT_AREA[3]
    assert card._label.yview()[1] < 1.0   # 內容超出可視高度，剩下的要捲


def test_wheel_scrolls_the_capped_card(card, root, monkeypatch):
    monkeypatch.setattr(card_module, "work_area_at", lambda x, y: _SHORT_AREA)
    card.show_pending((300, 100, 400, 100))
    root.update()
    card.show_text("\n".join(f"第 {i} 行" for i in range(60)))
    root.update()

    card._on_wheel(SimpleNamespace(delta=-120))
    root.update()

    assert card._label.yview()[0] > 0.0


def test_clicking_the_scrollbar_does_not_close_the_card(card, root):
    card.show_pending(_RECT)
    root.update()
    card._scrollbar.event_generate("<ButtonPress-1>", x=2, y=2)
    card._scrollbar.event_generate("<ButtonRelease-1>", x=2, y=2)
    root.update()
    assert card.is_open


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


def test_closing_by_click_or_the_close_button_notifies_on_close(card, root):
    closed = []
    card.on_close = lambda: closed.append(1)
    card.show_pending(_RECT)
    root.update()
    card._label.event_generate("<ButtonPress-1>", x=2, y=2)
    card._label.event_generate("<ButtonRelease-1>", x=3, y=2)
    root.update()
    assert closed == [1] and not card.is_open

    card.show_pending(_RECT)
    root.update()
    card._close.event_generate("<Button-1>")
    root.update()
    assert closed == [1, 1] and not card.is_open


def test_replacing_the_card_from_the_flow_does_not_count_as_closing(card, root):
    closed = []
    card.on_close = lambda: closed.append(1)
    card.show_pending(_RECT)
    card.show_pending((10, 10, 300, 100))   # 流程換位置重開
    card.hide()                             # 流程自己收掉
    assert closed == []


def test_the_card_leaves_room_for_the_box_handles_below_the_rect():
    from src.ui.region_box import HANDLE, MARGIN
    assert ANCHOR_GAP >= MARGIN + HANDLE // 2


# --- anchored_geometry：下方 → 上方 → 左右較寬的一側，最後夾高度 ---
_GAP, _MIN_W, _MIN_H = 4, 240, 120


def _place(anchor, w, h, area=_AREA, side_w=None):
    return anchored_geometry(anchor, w, h, area, gap=_GAP, side_w=side_w or w,
                             min_w=_MIN_W, min_h=_MIN_H)


def test_geometry_sits_below_when_it_fits():
    assert _place((300, 200, 400, 120), 400, 200) == (300, 324, 400, 200, "below")


def test_geometry_below_and_above_floor_the_height_at_the_minimum():
    assert _place((300, 200, 400, 120), 400, 50) == (300, 324, 400, 120, "below")
    assert _place((300, 900, 400, 120), 400, 50) == (300, 776, 400, 120, "above")


def test_geometry_flips_above_when_no_room_below():
    assert _place((300, 900, 400, 120), 400, 200) == (300, 696, 400, 200, "above")


def test_geometry_moves_to_the_wider_side_when_neither_above_nor_below_fits():
    assert _place((300, 100, 400, 900), 400, 200) == (704, 100, 400, 200, "right")
    assert _place((1300, 100, 400, 900), 400, 200) == (896, 100, 400, 200, "left")


def test_geometry_beside_the_rect_uses_the_text_width():
    assert _place((300, 100, 400, 900), 400, 200, side_w=900) == (704, 100, 900, 200, "right")


def test_geometry_beside_the_rect_caps_the_width_to_the_side_room():
    assert _place((300, 100, 400, 900), 400, 200, side_w=2000) == (704, 100, 1216, 200, "right")


def test_geometry_shrinks_to_the_side_room_but_not_below_the_minimum_width():
    # 兩側都只剩兩百多 px：縮到最小寬度、再夾回工作區內，蓋到矩形一角是最終狀態
    assert _place((200, 100, 1500, 900), 1500, 200) == (1680, 100, 240, 200, "right")
    assert _place((300, 100, 400, 900), 400, 200, side_w=100) == (704, 100, 240, 200, "right")


def test_side_minimum_width_is_wider_than_the_rect_minimum():
    # 側邊卡片與矩形不同寬，太窄會變成一長條；蓋到矩形一部分無妨
    assert SIDE_MIN_WIDTH > card_module.MIN_WIDTH


def test_geometry_beside_the_rect_slides_up_to_stay_inside_the_work_area():
    assert _place((300, 600, 400, 400), 400, 700) == (704, 340, 400, 700, "right")


def test_geometry_caps_the_height_to_the_work_area():
    assert _place((300, 100, 400, 900), 400, 3000) == (704, 0, 400, 1040, "right")


def test_geometry_keeps_the_minimum_height_on_a_tiny_work_area():
    assert _place((10, 10, 100, 80), 400, 500, area=(0, 0, 800, 100)) == (114, 0, 400, 120, "right")


def test_beside_geometry_matches_the_side_branch_of_anchored_geometry():
    # 側邊落點黏住後卡片直接呼叫 beside_geometry 重排，兩者必須算出同一個結果
    beside = beside_geometry((300, 100, 400, 900), 900, 200, _AREA, _GAP, _MIN_W, _MIN_H)
    assert beside == _place((300, 100, 400, 900), 400, 200, side_w=900)
