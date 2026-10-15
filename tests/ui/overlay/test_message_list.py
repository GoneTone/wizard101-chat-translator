"""訊息清單 `MessageList` 的測試：分頁畫布、換行重排、捲動錨點、補畫、訊息上限與淡出。"""
import time

import pytest

import src.ui.overlay.message_list as message_list_module
from src.i18n import t
from src.reader.chat_tabs import TabState
from src.ui.overlay.message_list import _ROW_PAD, should_stick_to_bottom
from src.ui.overlay.outlined_text import line_height, line_origin
from src.ui.overlay.selection import TEXT_ORIGIN, line_font, visual_lines
from src.ui.overlay.window import (
    FG_ERROR,
    OverlayWindow,
)
from tests.ui.overlay.overlay_helpers import (
    _at_bottom,
    _bar_state,
    _configure_width,
    _covered,
    _filled_overlay,
    _line_box,
    _long_overlay,
    _Press,
    _select_whole_message,
    _view,
)
from tests.ui.overlay.tk_helpers import settle_events


def test_resize_updates_existing_message_wraplength(root):
    # 視窗縮小後，既有訊息的換行寬度要跟著更新，否則文字右緣被切掉
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    ov.set_error("notice.offline")

    class FakeEvent:
        width = 240

    ov._list._on_canvas_configure(FakeEvent())
    ov._list.complete_layout()
    expected = max(80, 240 - 12)
    for entry in ov._list._messages:
        for line in entry.lines:
            assert int(float(line.canvas.itemcget(line.fg, "width"))) == expected


def test_stick_to_bottom_when_view_at_bottom():
    assert should_stick_to_bottom(1.0) is True
    assert should_stick_to_bottom(0.9995) is True  # 幾乎到底也算


def test_do_not_stick_when_scrolled_up():
    assert should_stick_to_bottom(0.4) is False


def test_add_message_appends_and_caps_at_max(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, max_messages=3, fade_seconds=180)
    for i in range(5):
        ov.add_message(f"msg {i}", f"訊息 {i}")
    texts = ov.visible_messages()
    assert len(texts) == 3
    # visible_messages 依時間順序（舊→新）回傳，保留最新 3 則
    assert texts[-1] == ("msg 4", "訊息 4")
    assert texts[0] == ("msg 2", "訊息 2")


def test_prune_removes_expired(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=10)
    ov.add_message("old", "舊", now=100.0)
    ov.add_message("new", "新", now=105.0)
    ov.prune(now=111.0)  # 100+10 < 111 過期；105+10 >= 111 保留
    assert ov.visible_messages() == [("new", "新")]


def test_no_fade_when_fade_seconds_zero(root):
    # fade_seconds=0：永不依時間清除
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("a", "甲", now=100.0)
    ov.add_message("b", "乙", now=105.0)
    ov.prune(now=1_000_000.0)  # 很久之後
    assert ov.visible_messages() == [("a", "甲"), ("b", "乙")]


def test_placeholder_centered_when_empty_hidden_after_message(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
    ov.set_status("locating")
    assert ov.placeholder_visible() is True          # 沒訊息 → 置中顯示狀態
    assert ov.status_text() == t("status.locating")
    ov.add_message("[A] hi", "譯文")
    assert ov.placeholder_visible() is False         # 有訊息 → 收掉
    ov.set_status("listening")
    assert ov.placeholder_visible() is False         # 有訊息時更新狀態也不重現


def test_update_message_fills_translation_in_place(root):
    # 佔位：訊息一讀到就先顯示原文，譯文稍後填入同一個位置（順序不因翻譯先後而變）
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "翻譯中…", msg_id=1)
    ov.add_message("[B] two", "翻譯中…", msg_id=2)
    ov.update_message(2, "乙")          # 後到的先翻完
    ov.update_message(1, "甲")
    assert ov.visible_messages() == [("[A] one", "甲"), ("[B] two", "乙")]


def _translation_fill(ov, index=0):
    """取某則訊息譯文行的本色（"fg" tag 只掛在本色上，描邊不算）。"""
    line = ov._list._messages[index].lines[1]
    return line.canvas.itemcget(line.fg, "fill")


def test_pending_placeholder_uses_dimmer_colour_until_filled(root):
    # 佔位期間譯文欄位要能一眼與已翻好的訊息區分，填入真正的譯文後恢復正常顏色
    from src.ui.palette import FG_PENDING, FG_TRANSLATED
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "翻譯中…", msg_id=1, pending=True)
    assert _translation_fill(ov) == FG_PENDING
    ov.update_message(1, "甲")
    assert _translation_fill(ov) == FG_TRANSLATED


def test_completed_message_is_not_dimmed(root):
    from src.ui.palette import FG_TRANSLATED
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "甲")          # 非佔位：直接就是完成品
    assert _translation_fill(ov) == FG_TRANSLATED


def _original_fill(ov, index=0):
    """取某則訊息原文行的本色（同 _translation_fill，取第 0 行）。"""
    line = ov._list._messages[index].lines[0]
    return line.canvas.itemcget(line.fg, "fill")


def test_dimmed_scales_each_channel_toward_dark():
    from src.ui.overlay.message_list import dimmed
    assert dimmed("#ffffff") == "#b5b5b5"   # 各通道乘 DIM_FACTOR，原文明顯暗於譯文
    assert dimmed("#80ff00") == "#5bb500"
    assert dimmed("#000000") == "#000000"


def test_message_uses_game_color_translated_bright_original_dim(root):
    # 譯文用遊戲聊天的顯示色，原文用同色調暗版 —— 與遊戲內配色一眼對得上
    from src.ui.overlay.message_list import dimmed
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "甲", color="#80ff00")
    assert _translation_fill(ov) == "#80ff00"
    assert _original_fill(ov) == dimmed("#80ff00")


def test_pending_message_restores_game_color_on_update(root):
    # 佔位期間仍用暗灰（語意＝還沒翻好），真譯文落地才換成遊戲色
    from src.ui.overlay.message_list import dimmed
    from src.ui.palette import FG_PENDING
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "翻譯中…", msg_id=1, pending=True, color="#80ff00")
    assert _translation_fill(ov) == FG_PENDING
    assert _original_fill(ov) == dimmed("#80ff00")
    ov.update_message(1, "甲")
    assert _translation_fill(ov) == "#80ff00"


def test_message_without_color_falls_back_to_default_palette(root):
    # 讀不到遊戲色（理論上不會發生，防衛用）：維持現行預設配色
    from src.ui.palette import FG_ORIGINAL, FG_TRANSLATED
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "甲")
    assert _translation_fill(ov) == FG_TRANSLATED
    assert _original_fill(ov) == FG_ORIGINAL


def test_update_message_ignores_unknown_id(root):
    # 佔位訊息可能已被 max_messages 擠掉或被 prune 清除：晚到的譯文安靜忽略，不得拋錯
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, max_messages=1, fade_seconds=0)
    ov.add_message("[A] one", "翻譯中…", msg_id=1)
    ov.add_message("[B] two", "翻譯中…", msg_id=2)   # 擠掉 msg_id=1
    ov.update_message(1, "甲")
    assert ov.visible_messages() == [("[B] two", "翻譯中…")]


def test_failed_translation_line_is_shown_in_error_colour(root):
    # 翻不出來的那則要跟一般對話一眼分得開：譯文行改用錯誤色，不用該則的遊戲色
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文", "翻譯中…", msg_id=7, pending=True, color="#66ccff")
    ov.update_message(7, "⚠  這則訊息翻譯不出來", failed=True)

    line = ov._list._messages[0].lines[1]
    assert line.canvas.itemcget(line.fg, "fill") == FG_ERROR


def test_successful_translation_keeps_the_game_colour(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文", "翻譯中…", msg_id=8, pending=True, color="#66ccff")
    ov.update_message(8, "譯文")

    line = ov._list._messages[0].lines[1]
    assert line.canvas.itemcget(line.fg, "fill") == "#66ccff"


def _row_offset(ov, msg_id: int) -> int:
    """某則訊息目前落在視口的哪個 y（畫面座標）。"""
    m = next(m for m in ov._list._messages if m.msg_id == msg_id)
    return ov._list._canvas.bbox(m.lines[0].tag, m.lines[1].tag)[1] - int(ov._list._canvas.canvasy(0))


def _scroll_up(ov, fraction: float = 0.3) -> None:
    ov._list._canvas.yview_moveto(fraction)
    ov._list.note_scroll()
    assert not ov._list._follow, "前置條件：往上捲後應停止跟隨底部"


def test_narrowing_window_keeps_following_new_messages(root):
    # 視窗變窄 → 文字重新換行變高 → 視圖被內容推離底部；自動跟隨必須自己貼回去，
    # 否則之後每一則新訊息都落在畫面外，看起來就像「訊息漏掉了」
    ov = _filled_overlay(root)
    ov._win.geometry("420x350+100+100")
    root.update()
    assert _at_bottom(ov)
    ov.add_message("brand new line", "全新的一行", msg_id=999)
    root.update()
    assert _at_bottom(ov)


def test_shortening_window_keeps_following_new_messages(root):
    # 視窗變矮 → 視口縮小、內容不動，同樣會讓視圖不再貼底
    ov = _filled_overlay(root)
    ov._win.geometry("739x220+100+100")
    root.update()
    assert _at_bottom(ov)


def test_error_banner_keeps_following_new_messages(root):
    # 錯誤橫幅從畫布底部吃走高度，效果等同視窗變矮
    ov = _filled_overlay(root)
    ov.set_error("notice.offline")
    root.update()
    assert _at_bottom(ov)
    ov.clear_error()
    root.update()
    assert _at_bottom(ov)


def test_scrolling_up_stops_following_until_back_at_bottom(root):
    # 使用者往上捲＝正在讀歷史，新訊息不該把畫面搶走
    ov = _filled_overlay(root)
    ov._list._canvas.yview_moveto(0.0)
    ov._list.note_scroll()
    ov.add_message("newest", "最新", msg_id=998)
    root.update()
    assert not _at_bottom(ov)
    # 捲回底部後恢復自動跟隨
    ov._list._canvas.yview_moveto(1.0)
    ov._list.note_scroll()
    ov.add_message("newer still", "更新的", msg_id=997)
    root.update()
    assert _at_bottom(ov)


def test_message_cap_keeps_scrolled_view_in_place(root):
    # 往上讀歷史時撞到訊息上限：最舊的幾則從上方被移除，底下的內容整段往上滑。
    # 畫布記的是像素原點、不是「看到哪一則」，不補位的話正在讀的那幾行就會跳掉。
    ov = _filled_overlay(root, count=40, max_messages=40)
    _scroll_up(ov)
    root.update()
    before = _row_offset(ov, 20)
    for i in range(100, 105):
        ov.add_message(f"newcomer {i} pushing the oldest lines out of the list",
                       f"第 {i} 則擠掉最舊訊息的新訊息，長度足以換行", msg_id=i)
    root.update()
    assert _row_offset(ov, 20) == before


def test_prune_keeps_scrolled_view_in_place(root):
    # 淡出清除同樣是從上方移除內容，效果與撞上限一致
    ov = OverlayWindow(root, x=100, y=100, width=739, height=350,
                       max_messages=200, fade_seconds=60)
    root.update()
    for i in range(40):
        ov.add_message(f"message number {i}: a chat line long enough to wrap",
                       f"第 {i} 則譯文，內容夠長，窄視窗下一定會換行成兩行以上",
                       now=1000.0 + i, msg_id=i)
    root.update()
    _scroll_up(ov)
    root.update()
    before = _row_offset(ov, 20)
    ov.prune(now=1000.0 + 10 + 60)   # 清掉最舊的十則
    root.update()
    assert len(ov.visible_messages()) == 29
    assert _row_offset(ov, 20) == before


def test_filling_a_pending_translation_keeps_scrolled_view_in_place(root):
    # 佔位訊息填入譯文會改變該列高度；若那列在視口上方，底下的內容會整段下移
    ov = _filled_overlay(root, count=40)
    ov.add_message("pending line", "翻譯中…", msg_id=500, pending=True)
    for i in range(600, 610):
        ov.add_message(f"later line {i} keeping the pending row above the viewport",
                       f"第 {i} 則後續訊息，長度足以換行", msg_id=i)
    root.update()
    _scroll_up(ov, 0.8)
    root.update()
    before = _row_offset(ov, 605)
    ov.update_message(500, "終於補上的譯文。這段刻意寫得很長，長到足以讓那一列從佔位時的"
                           "一行撐成三行以上 —— 高度一變，底下的所有訊息都會跟著往下移，"
                           "使用者正在讀的那幾行也就跟著跑掉了，所以這裡要一起補位。"
                           "再多墊一句，確保在寬視窗下也一定會換行成好幾行。")
    root.update()
    assert _row_offset(ov, 605) == before


def test_prune_clears_a_selection_in_the_removed_row(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=60)
    ov.add_message("原文一", "譯文一", now=0.0)
    _select_whole_message(ov)
    assert ov._selection.active is True

    ov.prune(now=1000.0)

    assert ov._selection.active is False
    assert ov.visible_messages() == []


def test_max_messages_overflow_clears_a_selection_in_the_dropped_row(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=1, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)

    ov.add_message("原文二", "譯文二")

    assert ov._selection.active is False


def test_update_message_clears_a_selection_in_that_row(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "翻譯中…", msg_id=7, pending=True)
    _select_whole_message(ov)

    ov.update_message(7, "譯文一")

    assert ov._selection.active is False


def test_resize_redraws_the_highlight_to_the_new_wrapping(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    # *10（非 *20）：縮放前要留在同一視覺行，_select_whole_message 的垂直置中點
    # 才會落在該行內、真的從頭選起 —— 量測顯示 *10 在縮放前的換行寬度下恰好一行、
    # *20 已經先換成兩行，命中點會落在行界上、選不到開頭（見 fix-round 報告）。
    ov.add_message("原文一" * 10, "譯文一" * 10)
    _select_whole_message(ov)
    selected = ov._selection.text()

    class FakeEvent:
        width = 240

    ov._list._on_canvas_configure(FakeEvent())
    ov._win.update_idletasks()

    first, second = ov._list._messages[0].lines   # 換行寬度一變就刪掉重畫，行要重新取
    canvas = first.canvas
    rects = [i for i in canvas.find_withtag("sel")
             if canvas.coords(i)[1] < line_origin(second)[1]]   # 只取原文行的反白
    assert ov._selection.text() == selected
    # 每個視覺行一個反白矩形：換行變了、矩形數就要跟著變
    assert len(rects) == len(visual_lines(first, line_font(first)))
    # 沒有重畫的話，矩形仍是舊換行寬度下的幾何，右緣會超出新的換行寬度
    assert max(canvas.coords(i)[2] for i in rects) <= ov._list.wrap + TEXT_ORIGIN


def test_prune_during_a_drag_keeps_the_selected_text(root):
    # 拖曳中上方訊息被 prune 掉，內容整段上移；沒有錨點補位的話，游標下的字會換掉。
    #
    # 6 則舊訊息會被 prune、20+ 則新訊息留下 —— 內容量要夠大，「貼底時的相對位置」
    # 才補得回去；只留一兩則的話，內容縮到比視口還矮，怎麼補位都會被頂到頂端。
    # 拖曳開始後還要再新增幾則訊息（模擬翻譯持續進來）：跟隨模式每次加訊息都會把
    # 視圖精準貼齊捲動範圍下緣，若拖曳一開始就呼叫 prune，Tk 自己重算 scrollregion
    # 時剛好會把畫面重新頂回底部、巧合掩蓋掉這個 bug；插入這幾則之後視圖才會真正
    # 脫離下緣，需要 `_view_anchor` 主動補位才守得住。
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=50, fade_seconds=60)
    for i in range(6):
        ov.add_message(f"原文{i}", f"譯文{i}", now=0.0)
    for i in range(20):
        ov.add_message(f"原文新{i}", f"譯文新{i}", now=2000.0)
    ov._win.update()
    first, second = ov._list._messages[-1].lines
    x, top, height = _line_box(first)
    ov._selection_press(_Press(x + TEXT_ORIGIN, top + height // 2))
    x, top, height = _line_box(second)
    ov._selection_drag(_Press(x + 1000, top + height // 2))
    selected = ov._selection.text()

    for i in range(3):
        ov.add_message(f"拖曳中{i}", f"拖曳中譯文{i}", now=2000.0)
    ov._win.update()
    before_y = _line_box(second)[1]

    ov.prune(now=2000.0)   # 最舊的 6 則過期（0.0 + 60 < 2000），其餘保留
    ov._win.update()

    assert ov._selection.text() == selected
    assert abs(_line_box(second)[1] - before_y) <= 1, \
        "拖曳中的那一列不該因為上方訊息被 prune 而在畫面上移動"


def test_translation_fills_a_message_in_a_hidden_view(root):
    from src.ui.palette import FG_TRANSLATED
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    ov.add_message("group", "翻譯中…", msg_id=5, pending=True, slot=1, tabs=(1,))
    ov.update_message(5, "隊")
    ov.set_tabs(1, _bar_state(1))
    assert ov.visible_messages() == [("group", "隊")]
    assert _translation_fill(ov) == FG_TRANSLATED


def test_hidden_pending_and_failed_rows_keep_their_colours(root):
    from src.ui.palette import FG_PENDING
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    ov.add_message("a", "翻譯中…", msg_id=1, pending=True, slot=1, tabs=(1,))
    ov.add_message("b", "翻譯中…", msg_id=2, pending=True, slot=1, tabs=(1,))
    ov.update_message(2, "⚠", failed=True)
    ov.set_tabs(1, _bar_state(1))
    assert _translation_fill(ov, 0) == FG_PENDING
    assert _translation_fill(ov, 1) == FG_ERROR


def test_max_messages_applies_per_tab(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, max_messages=2,
                       fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    ov.add_message("g", "隊", slot=1, tabs=(1,))
    for i in range(3):
        ov.add_message(f"m{i}", f"主{i}", slot=1, tabs=(0,))
    assert ov.visible_messages() == [("m1", "主1"), ("m2", "主2")]
    ov.set_tabs(1, _bar_state(1))
    assert ov.visible_messages() == [("g", "隊")]


def test_prune_drops_faded_messages_in_hidden_views_too(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=10)
    ov.set_tabs(1, _bar_state(0))
    ov.add_message("old", "舊", now=100.0, slot=1, tabs=(1,))
    ov.add_message("new", "新", now=105.0, slot=1, tabs=(1,))
    ov.prune(now=111.0)
    ov.set_tabs(1, _bar_state(1))
    assert ov.visible_messages() == [("new", "新")]


def test_switching_views_clears_the_selection(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    ov.add_message("main", "主", slot=1, tabs=(0,))
    _select_whole_message(ov)
    assert ov._selection.active is True
    ov.set_tabs(1, _bar_state(1))
    assert ov._selection.active is False


def test_switching_views_returns_to_following_the_bottom(root):
    ov = OverlayWindow(root, x=100, y=100, width=739, height=350, max_messages=200,
                       fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    root.update()
    for i in range(40):
        ov.add_message(f"message number {i}: a chat line long enough to wrap",
                       f"第 {i} 則譯文，內容夠長，窄視窗下一定會換行", slot=1, tabs=(0,))
    root.update()
    _scroll_up(ov)
    ov.set_tabs(1, _bar_state(1))
    ov.set_tabs(1, _bar_state(0))
    root.update()
    assert ov._list._follow
    assert _at_bottom(ov)


def _assert_rows_follow_the_layout(messages):
    """各則依舊版 frame＋pack(pady=2) 的幾何排列：兩行上下相接、列與列之間 4px。"""
    top = line_origin(messages[0].lines[0])[1]
    for m in messages:
        original, translated = m.lines
        assert line_origin(original) == (0, top)
        assert line_origin(translated) == (0, top + line_height(original))
        top += line_height(original) + line_height(translated) + 4


def _scrollregion(ov):
    return [float(v) for v in ov._list._canvas.cget("scrollregion").split()]


def test_layout_follows_the_old_frame_geometry(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, max_messages=10,
                       fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    ov.add_message("原文二", "譯文二")
    ov._win.update_idletasks()   # 捲動更新排在 idle
    first, second = ov._list._messages
    assert line_origin(first.lines[0]) == (0, 2)
    _assert_rows_follow_the_layout(ov._list._messages)
    last = second.lines[1]
    bottom = line_origin(last)[1] + line_height(last)
    assert _scrollregion(ov)[1] == 0 and _scrollregion(ov)[3] == bottom + 2

    ov.set_limits(1, 0)   # 刪掉最舊的一則：其餘不搬，捲動範圍從剩下那則的列頂算起
    assert _scrollregion(ov)[1] == line_origin(second.lines[0])[1] - 2
    assert _scrollregion(ov)[3] == bottom + 2


def test_translation_growing_a_hidden_line_pushes_later_messages_down(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    root.update()   # 先讓換行寬度定案，切回分頁 1 時才不會整頁重排、掩蓋位移
    for i in range(3):
        ov.add_message(f"group {i}", "翻譯中…", msg_id=i, pending=True, slot=1, tabs=(1,))
    ov.update_message(0, "這則譯文刻意寫得很長，長到一定會在分頁 1 的畫布上多換一行以上，"
                         "這樣後面兩則才需要往下移" * 2)
    ov.set_tabs(1, _bar_state(1))
    messages = ov._list._messages
    assert line_height(messages[0].lines[1]) > line_height(messages[1].lines[1]), \
        "前置條件：第一則的譯文應該要變高"
    _assert_rows_follow_the_layout(messages)
    canvas = ov._list._canvas
    for upper, lower in zip(messages, messages[1:], strict=False):
        upper_box = canvas.bbox(*(ln.tag for ln in upper.lines))
        lower_box = canvas.bbox(*(ln.tag for ln in lower.lines))
        assert upper_box[3] < lower_box[1]


def test_resize_while_hidden_rewraps_on_show(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    root.update()
    for _ in range(2):
        ov.add_message("group chat line " * 12, "隊伍頻道的譯文 " * 12, slot=1, tabs=(1,))
    before = line_height(ov._list._views[(1, 1)].records[0].lines[0])
    old = ov._list.wrap
    ov._win.geometry("300x300+0+0")
    root.update()
    assert ov._list.wrap != old, "前置條件：縮放應該改變換行寬度"
    ov.set_tabs(1, _bar_state(1))
    ov._list.complete_layout()
    assert line_height(ov._list._messages[0].lines[0]) > before, \
        "前置條件：原文行應該多換一行，下方內容才需要移動"
    for m in ov._list._messages:
        for line in m.lines:
            assert int(float(line.canvas.itemcget(line.fg, "width"))) == ov._list.wrap
    _assert_rows_follow_the_layout(ov._list._messages)


def test_pruning_above_a_selection_keeps_text_and_highlight(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, max_messages=10,
                       fade_seconds=60)
    ov.add_message("原文一", "譯文一", now=0.0)
    ov.add_message("原文二", "第二則的譯文，選取其中一段", now=1000.0)
    ov._win.update()
    original, translated = ov._list._messages[1].lines
    x, top, height = _line_box(translated)
    ov._selection.begin(x + 40, top + height // 2)
    ov._selection.extend(x + 120, top + height // 2)
    selected = ov._selection.text()
    assert selected, "前置條件：應該選到一段文字"

    ov.prune(now=100.0)   # 第一則（0.0 + 60 < 100）淡出
    ov._win.update()

    assert ov.visible_messages() == [("原文二", "第二則的譯文，選取其中一段")]
    assert ov._selection.text() == selected
    canvas = translated.canvas
    rects = canvas.find_withtag("sel")
    assert rects
    low = line_origin(original)[1]
    high = line_origin(translated)[1] + line_height(translated)
    for rect in rects:
        _, y0, _, y1 = canvas.coords(rect)
        assert low <= y0 and y1 <= high


def test_wheel_scrolls_a_newly_created_view(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, max_messages=50,
                       fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    ov.set_tabs(1, _bar_state(1))
    for i in range(20):
        ov.add_message(f"原文{i}", f"譯文{i}", slot=1, tabs=(1,))
    ov._win.update()
    canvas = ov._list._canvas
    before = canvas.yview()[0]
    assert before > 0, "前置條件：內容要多到可以往上捲"

    canvas.event_generate("<Enter>")
    canvas.event_generate("<MouseWheel>", delta=120)
    canvas.event_generate("<Leave>")

    assert canvas.yview()[0] < before


def test_switching_views_keeps_each_views_canvas(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    ov.add_message("main", "主", slot=1, tabs=(0,))
    canvas = ov._list._canvas
    ov.set_tabs(1, _bar_state(1))
    assert ov._list._canvas is not canvas
    ov.set_tabs(1, _bar_state(0))
    assert ov._list._canvas is canvas
    assert canvas.winfo_manager() == "pack"


def test_rewrap_between_taking_and_restoring_the_anchor_keeps_the_view(root):
    # refresh_scroll 的 update_idletasks() 可能剛好送來改變寬度的 <Configure>，重排後錨點仍要對得回去
    ov = _filled_overlay(root)
    _scroll_up(ov)
    root.update()
    anchor = ov._list._view_anchor()
    tag, offset = anchor

    _configure_width(ov, 420)
    ov._list.refresh_scroll(anchor)

    assert _offset_of(ov, tag) == offset


def test_resizing_with_a_selection_does_not_grow_the_font_cache(root):
    # 重排會重畫行；字型快取若以行為鍵，每次縮放都會多留下一批 Tcl 字型
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, max_messages=10, fade_seconds=0)
    ov.add_message("原文一" * 10, "譯文一" * 10)
    _select_whole_message(ov)

    class FakeEvent:
        width = 240

    for width in (240, 300, 240, 300):
        FakeEvent.width = width
        ov._list._on_canvas_configure(FakeEvent())

    assert ov._selection.active
    assert len(ov._selection._fonts) <= 2


def test_add_after_a_tab_switch_in_the_same_turn_still_sticks_to_bottom(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    for i in range(30):
        ov.add_message(f"原文{i}", f"譯文{i}", slot=1, tabs=(0, 1))
    for tab in (1, 0):   # 兩個畫布都先排版過，之後切換不再有 Configure 幫忙補刷新
        ov.set_tabs(1, _bar_state(tab))
        ov._win.update_idletasks()
    ov.add_message("甲", "甲譯", slot=1, tabs=(0,))
    ov.set_tabs(1, _bar_state(1))
    ov.add_message("乙", "乙譯", slot=1, tabs=(1,))
    ov._win.update_idletasks()
    canvas = ov._list._canvas
    assert canvas.yview()[1] == 1.0
    assert float(canvas.cget("scrollregion").split()[3]) == ov._list._region()[1]


def test_a_burst_of_adds_keeps_a_scrolled_up_view_in_place(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    for i in range(40):
        ov.add_message(f"原文{i}", f"譯文{i}")
    ov._win.update_idletasks()
    canvas = ov._list._canvas
    canvas.yview_moveto(0.3)
    ov._list.note_scroll()
    before = canvas.canvasy(0)
    for i in range(5):
        ov.add_message(f"新{i}", f"新譯文{i}")
    ov._win.update_idletasks()
    assert canvas.canvasy(0) == before


def test_showing_a_stale_tab_rewraps_it_before_it_is_visible(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    for _ in range(3):
        ov.add_message("原文" * 20, "譯文" * 20, slot=1, tabs=(0, 1))
    ov.set_tabs(1, _bar_state(1))
    ov.set_tabs(1, _bar_state(0))
    ov._win.update_idletasks()

    class FakeEvent:
        width = 240

    ov._list._on_canvas_configure(FakeEvent())   # 分頁 1 隱藏中，實際畫布寬度不變
    ov.set_tabs(1, _bar_state(1))
    ov._list.complete_layout()
    for m in ov._list._messages:
        for line in m.lines:
            assert int(line.canvas.itemcget(line.fg, "width")) == ov._list.wrap


class _Wheel:
    """假的滾輪事件（只用到 delta）。"""

    def __init__(self, delta):
        self.delta = delta


def _layout(ov):
    """每則兩行相對第一則的 y 偏移、行高與換行寬度。"""
    messages = ov._list._messages
    base = line_origin(messages[0].lines[0])[1]
    return [(line_origin(line)[1] - base, line_height(line),
             line.canvas.itemcget(line.fg, "width"))
            for m in messages for line in m.lines]


def _fresh_overlay_with_same_messages(root, ov, wrap):
    """以 wrap 一次全部畫出 ov 現有訊息的新 overlay。"""
    messages = ov._list._messages
    fresh = OverlayWindow(root, x=0, y=0, width=460, height=300,
                          max_messages=len(messages) + 1, fade_seconds=0)
    fresh._win.update()
    _configure_width(fresh, wrap + 12)
    for m in messages:
        fresh.add_message(m.original, m.translated, msg_id=m.msg_id)
    assert fresh._list.wrap == wrap
    return fresh


def _offset_of(ov, tag):
    """tag 那則的列頂相對視口頂端的偏移。"""
    m = next(m for m in ov._list._messages if m.tag == tag)
    return round(line_origin(m.lines[0])[1]) - int(ov._list._canvas.canvasy(0))


def test_resize_draws_only_the_viewport_then_completes_identically(root):
    ov = _long_overlay(root, 120)
    _configure_width(ov, 300)
    view = ov._list._views[ov._list.shown_key]
    assert view.hi == 120 and view.hi - view.lo < 30
    assert _covered(ov)
    ov._list.complete_layout()
    assert _layout(ov) == _layout(_fresh_overlay_with_same_messages(root, ov, wrap=ov._list.wrap))


def test_wheeling_up_after_a_resize_never_shows_a_gap(root):
    ov = _long_overlay(root, 120)
    _configure_width(ov, 300)
    for _ in range(400):
        ov._list.on_wheel(_Wheel(delta=120))
        assert _covered(ov)


def test_dragging_the_scrollbar_to_the_top_lands_on_the_first_message(root):
    ov = _long_overlay(root, 120)
    _configure_width(ov, 300)
    ov._list._user_scroll("moveto", 0.0)
    ov._win.update_idletasks()
    first = ov._list._messages[0]
    assert first.lines and round(line_origin(first.lines[0])[1]) - ov._list._canvas.canvasy(0) == _ROW_PAD


def test_taller_window_during_partial_layout_shows_no_gap(root):
    ov = _long_overlay(root, 120)
    ov._apply_geometry(10000, 10000, 312, 300)
    settle_events(ov._win)
    assert _view(ov).lo > 0, "前置條件：跟隨底部的部分排版"
    seen = []
    # 記下 <Configure> 處理完當下（重繪之前）的覆蓋狀態，排在 idle 的補救來不及
    ov._list._canvas.bind("<Configure>", lambda e: seen.append((e.height, _covered(ov))), add="+")
    ov._apply_geometry(10000, 10000, 312, 900)
    settle_events(ov._win)
    assert seen and seen[-1] == (ov._list._canvas.winfo_height(), True)
    assert seen[-1][0] > 600


def test_resizing_an_empty_view_logs_nothing(root, monkeypatch):
    logged = []
    monkeypatch.setattr(message_list_module, "log", logged.append)
    monkeypatch.setattr(message_list_module, "_SLOW_RELAYOUT_MS", -1)   # 有訊息的話一定會記
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    _configure_width(ov, 240)
    assert not [line for line in logged if "relaid out" in line or "rewrapped" in line]


def test_a_slow_viewport_fill_is_logged_but_not_inside_a_relayout(root, monkeypatch):
    logged = []
    monkeypatch.setattr(message_list_module, "log", logged.append)
    monkeypatch.setattr(message_list_module, "_SLOW_RELAYOUT_MS", -1)   # 有補畫的話一定會記
    ov = _long_overlay(root, 120)
    _configure_width(ov, 300)
    assert not [line for line in logged if "viewport filled" in line]   # 重排另有自己的 log
    ov._list._user_scroll("moveto", 0.0)
    assert [line for line in logged if "viewport filled" in line]


def test_follow_mode_keeps_the_newest_message_at_the_bottom_after_a_resize(root):
    ov = _long_overlay(root, 120)
    _configure_width(ov, 300)
    ov._win.update_idletasks()
    assert ov._list._canvas.yview()[1] == 1.0


def test_reading_history_keeps_the_viewport_top_message_in_place_on_resize(root):
    ov = _long_overlay(root, 120)
    ov._list._user_scroll("moveto", 0.5)
    ov._win.update()
    tag, offset = ov._list._view_anchor()
    assert 0 <= offset < ov._list._canvas.winfo_height(), "前置條件：錨點那則要在視口內"
    _configure_width(ov, 300)
    ov._win.update_idletasks()
    assert abs(_offset_of(ov, tag) - offset) <= 1


def test_updates_trims_and_adds_during_partial_layout_match_a_full_redraw(root):
    ov = _long_overlay(root, 120)
    _configure_width(ov, 300)
    ov.update_message(ov._list._messages[5].msg_id, "補上的長譯文" * 20)
    ov.update_message(ov._list._messages[-1].msg_id, "補上的長譯文" * 20)
    ov.add_message("新來的", "擠掉最舊一則")
    ov._list.complete_layout()
    assert _layout(ov) == _layout(_fresh_overlay_with_same_messages(root, ov, wrap=ov._list.wrap))


def test_dragging_the_scrollbar_to_the_bottom_lands_on_the_newest_message(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, max_messages=300, fade_seconds=0)
    for i in range(300):
        ov.add_message(f"wizard chat line number {i}", f"第 {i} 則一行放得下的譯文", msg_id=i)
    ov._win.update()
    ov._list.complete_layout()
    ov._list._user_scroll("moveto", 0.3)
    _configure_width(ov, 140)   # 變窄：單行訊息變多行，下方待補的估計高度偏低
    assert ov._list._views[ov._list.shown_key].hi < 300, "前置條件：最新一則還沒畫"
    ov._list._user_scroll("moveto", 1.0)
    ov._win.update_idletasks()
    assert ov._list._views[ov._list.shown_key].hi == 300
    assert ov._list._canvas.yview()[1] == 1.0
    assert _covered(ov)


def test_dragging_the_scrollbar_to_the_top_lands_on_the_first_message_when_estimates_are_low(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, max_messages=120, fade_seconds=0)
    for i in range(120):
        ov.add_message(f"wizard chat line number {i}", f"第 {i} 則一行放得下的譯文", msg_id=i)
    ov._win.update()
    ov._list.complete_layout()
    ov._list._user_scroll("moveto", 0.7)
    _configure_width(ov, 140)   # 變窄：單行訊息變多行，上方待補的估計高度偏低
    assert _view(ov).lo > 0, "前置條件：第一則還沒畫"
    ov._list._user_scroll("moveto", 0.0)
    ov._win.update_idletasks()
    assert _view(ov).lo == 0
    assert ov._list._canvas.yview()[0] == 0.0
    assert _covered(ov)


def test_pruning_the_whole_drawn_range_during_partial_layout_matches_a_full_redraw(root):
    ov = _long_overlay(root, 120, fade_seconds=60)
    ov._list._user_scroll("moveto", 0.3)
    _configure_width(ov, 300)
    view = ov._list._views[ov._list.shown_key]
    assert view.lo > 0 and view.hi < 110, "前置條件：已畫範圍在中段"
    hi = view.hi
    ov.prune(now=hi + 5 + 60)   # 時刻 ≤ hi + 5 的全部淡出，已畫範圍整段移光
    assert len(view.records) == 120 - (hi + 6)
    assert view.lo == 0 and view.hi > view.lo and _covered(ov)
    ov._list.complete_layout()
    assert _layout(ov) == _layout(_fresh_overlay_with_same_messages(root, ov, wrap=ov._list.wrap))


def _pump(root, seconds):
    """跑事件迴圈 seconds 秒（背景補畫靠 after 排程推進）。"""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        root.update()
        time.sleep(0.01)


def _pump_until(root, condition, timeout):
    """跑事件迴圈直到 condition 成立，逾時就失敗。"""
    deadline = time.monotonic() + timeout
    while not condition():
        if time.monotonic() > deadline:
            pytest.fail(f"condition not met within {timeout}s")
        root.update()
        time.sleep(0.01)


def _long_overlay_with_two_clients(root, n):
    """slot 1、2 各一個分頁、各 n 則會換行的訊息，前景停在 slot 1（它的訊息全部畫完）。"""
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, max_messages=n, fade_seconds=0)
    for slot in (1, 2):
        ov.set_tabs(slot, TabState(0, ((0, "Main"),)))
    # 兩個畫布都先排版過：之後切換不會再收到改寬度的 <Configure>
    for slot in (2, 1):
        for i in range(n):
            ov.add_message(f"slot {slot} message {i}: " + "wizard chat line " * (1 + i % 6),
                           f"第 {i} 則譯文" + "會換行的長譯文內容" * (1 + i % 4),
                           slot=slot, tabs=(0,))
        ov.set_foreground_slot(slot)
        ov._win.update()
    ov._list.complete_layout()
    return ov


def test_background_backfill_completes_without_further_input(root):
    ov = _long_overlay(root, 120)
    _configure_width(ov, 300)
    _pump_until(root, lambda: _view(ov).lo == 0 and _view(ov).hi == 120, timeout=60)
    assert ov._list._canvas.yview()[1] == 1.0                # 往上補畫時仍貼底
    assert _layout(ov) == _layout(_fresh_overlay_with_same_messages(root, ov, wrap=ov._list.wrap))


def test_only_the_last_width_of_a_drag_is_backfilled(root, monkeypatch):
    logged = []
    monkeypatch.setattr(message_list_module, "log", logged.append)
    monkeypatch.setattr(message_list_module, "_SLOW_RELAYOUT_MS", -1)
    # 拖拉總長（1 秒）超過靜止判定，每段間隔（0.25 秒）遠短於它：沒取消的話第一輪會在拖拉中開跑
    monkeypatch.setattr(message_list_module, "_BACKFILL_DELAY_MS", 600)
    ov = _long_overlay(root, 120)
    for width in (420, 380, 340, 300):
        _configure_width(ov, width)
        lo = _view(ov).lo
        _pump(root, seconds=0.25)
        assert _view(ov).lo == lo                       # 寬度還在變（間隔短於靜止判定），不補畫
    _pump_until(root, lambda: _view(ov).lo == 0 and _view(ov).hi == 120, timeout=60)
    for m in ov._list._messages:
        for line in m.lines:
            assert int(line.canvas.itemcget(line.fg, "width")) == ov._list.wrap
    rewrapped = [line for line in logged if "view rewrapped" in line]
    assert len(rewrapped) == 1 and f"wrap={ov._list.wrap}," in rewrapped[0], rewrapped
    assert len([line for line in logged if "relaid out" in line]) == 1   # 拖拉中只記一次同步階段


def test_one_event_loop_pass_runs_one_backfill_batch(root):
    ov = _long_overlay(root, 120)
    _configure_width(ov, 300)
    start = _view(ov).lo
    _pump_until(root, lambda: _view(ov).lo < start, timeout=60)
    # 批與批之間經 idle 讓出事件迴圈（重繪）；直接 after(1) 的話一次 update() 就會全部補完
    assert _view(ov).lo > 0


def test_a_hidden_view_is_not_backfilled_and_resumes_when_shown(root):
    ov = _long_overlay_with_two_clients(root, 120)
    _configure_width(ov, 300)
    start = _view(ov).lo
    ov._list._backfill_once()                          # 先補一點，才分得出「沿用」與「重排」
    lo = _view(ov).lo
    assert 0 < lo < start
    ov.set_foreground_slot(2)
    _pump(root, seconds=0.5)
    ov.set_foreground_slot(1)
    assert _view(ov).lo == lo and _covered(ov)         # 隱藏期間沒補；切回沿用已畫範圍
    _pump_until(root, lambda: _view(ov).lo == 0, timeout=60)


def test_switching_back_to_a_view_whose_newest_message_is_undrawn_relays_it_out(root):
    ov = _long_overlay_with_two_clients(root, 120)
    _scroll_up(ov, 0.2)
    _configure_width(ov, 300)                            # 錨點在上方，最新一則還沒畫
    assert _view(ov).hi < 120
    ov.set_foreground_slot(2)
    ov.set_foreground_slot(1)
    ov._win.update_idletasks()
    assert _view(ov).hi == 120 and _covered(ov) and ov._list._canvas.yview()[1] == 1.0
    assert _view(ov).hi - _view(ov).lo < 30              # 從底部重排，而非把中間全部同步畫完


def test_destroying_the_overlay_cancels_its_pending_work(root):
    ov = _long_overlay(root, 120)
    _configure_width(ov, 300)
    root.tk.eval("set ::bg_errors {}; proc bgerror msg {lappend ::bg_errors $msg}")
    try:
        ov._win.destroy()
        _pump(root, seconds=0.3)
        errors = root.tk.splitlist(root.tk.eval("set ::bg_errors"))
    finally:
        root.tk.eval("rename bgerror {}")
    assert not errors   # 已排的工作呼叫到已刪除的 Tcl 命令
