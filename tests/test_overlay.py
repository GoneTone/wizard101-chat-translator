import pytest

from src.i18n import t
from src.ui.bubble import should_auto_expand
from src.ui.geometry import edge_at, is_click, moved_to, point_in_rect, resized_edge
from src.ui.message_list import should_stick_to_bottom
from src.ui.overlay import (
    _GRIP_SIZE,
    BG,
    BG_UPDATE,
    FG_ERROR,
    MIN_HEIGHT,
    MIN_WIDTH,
    STATUS_COLORS,
    OverlayWindow,
    autoscroll_pixels,
)
from src.ui.selection import TEXT_ORIGIN, line_font, visual_lines
from src.ui.thin_scrollbar import scroll_fraction, thumb_span


def test_is_click_within_threshold():
    assert is_click(0, 0) is True
    assert is_click(4, -3) is True   # 位移小於門檻＝點擊


def test_is_click_beyond_threshold_is_drag():
    assert is_click(6, 0) is False
    assert is_click(0, -8) is False


def test_resize_updates_existing_message_wraplength(root):
    # 視窗縮小後，既有訊息的換行寬度要跟著更新，否則文字右緣被切掉
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    ov.set_error("notice.offline")

    class FakeEvent:
        width = 240

    ov._list._on_canvas_configure(FakeEvent())
    expected = max(80, 240 - 12)
    for entry in ov._list._messages:
        for child in entry.row.winfo_children():
            assert int(float(child.itemcget("txt", "width"))) == expected


def test_minimize_counts_unread_and_expand_resets(root):
    ov = OverlayWindow(root, x=0, y=0, max_messages=10, fade_seconds=0)
    ov.add_message("m0", "t0")
    ov.minimize()
    assert ov.minimized
    ov.add_message("m1", "t1")
    ov.add_message("m2", "t2")
    assert ov.unread_count() == 2      # 縮小期間累積未讀
    ov.expand()
    assert not ov.minimized
    assert ov.unread_count() == 0      # 展開歸零
    assert len(ov.visible_messages()) == 3  # 縮小期間的訊息沒有遺失


def test_moved_to_adds_delta():
    assert moved_to(100, 200, 15, -30) == (115, 170)


def test_stick_to_bottom_when_view_at_bottom():
    assert should_stick_to_bottom(1.0) is True
    assert should_stick_to_bottom(0.9995) is True  # 幾乎到底也算


def test_do_not_stick_when_scrolled_up():
    assert should_stick_to_bottom(0.4) is False


def test_resized_edge_south_east_grows_without_moving():
    assert resized_edge("se", 100, 200, 400, 300, 50, 20,
                        MIN_WIDTH, MIN_HEIGHT) == (100, 200, 450, 320)


def test_resized_edge_south_east_clamps_to_minimum():
    assert resized_edge("se", 100, 200, 400, 300, -1000, -1000,
                        MIN_WIDTH, MIN_HEIGHT) == (100, 200, MIN_WIDTH, MIN_HEIGHT)


def test_resized_edge_west_moves_origin_and_keeps_right_edge():
    # 往左拖 40：左緣左移 40、寬度加 40，右緣（x+w）不動
    assert resized_edge("w", 100, 200, 400, 300, -40, 0,
                        MIN_WIDTH, MIN_HEIGHT) == (60, 200, 440, 300)


def test_resized_edge_north_moves_origin_and_keeps_bottom_edge():
    assert resized_edge("n", 100, 200, 400, 300, 0, -40,
                        MIN_WIDTH, MIN_HEIGHT) == (100, 160, 400, 340)


def test_resized_edge_west_freezes_origin_at_minimum_width():
    # 撞到最小寬度後左緣要停住；否則游標繼續右移會把整個視窗一起往右拉走
    x, y, w, h = resized_edge("w", 100, 200, 400, 300, 1000, 0,
                              MIN_WIDTH, MIN_HEIGHT)
    assert (w, h) == (MIN_WIDTH, 300)
    assert x == 100 + 400 - MIN_WIDTH   # 右緣仍固定在 500
    assert y == 200


def test_resized_edge_north_west_resizes_both_axes():
    assert resized_edge("nw", 100, 200, 400, 300, -30, -20,
                        MIN_WIDTH, MIN_HEIGHT) == (70, 180, 430, 320)


def test_edge_at_outside_window_is_no_edge():
    assert edge_at(99, 300, 100, 200, 400, 300) == ""


def test_edge_at_interior_is_no_edge():
    assert edge_at(300, 350, 100, 200, 400, 300) == ""


def test_edge_at_each_side():
    assert edge_at(102, 350, 100, 200, 400, 300) == "w"
    assert edge_at(497, 350, 100, 200, 400, 300) == "e"
    assert edge_at(300, 202, 100, 200, 400, 300) == "n"
    assert edge_at(300, 497, 100, 200, 400, 300) == "s"


def test_edge_at_corners_take_priority_over_sides():
    assert edge_at(108, 208, 100, 200, 400, 300) == "nw"
    assert edge_at(492, 208, 100, 200, 400, 300) == "ne"
    assert edge_at(108, 492, 100, 200, 400, 300) == "sw"
    assert edge_at(492, 492, 100, 200, 400, 300) == "se"


def test_edge_at_ignores_corner_band_away_from_the_other_axis():
    # 距離左緣 10px（大於邊界寬、小於角落寬）但在視窗中段：不是角落也不算邊
    assert edge_at(110, 350, 100, 200, 400, 300) == ""


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


def test_error_banner_toggle(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
    assert ov.error_text() is None
    ov.set_error("notice.offline")
    assert ov.error_text() == t("notice.offline")
    ov.clear_error()
    assert ov.error_text() is None


@pytest.mark.real_position
def test_explicit_position_and_size_applied(root):
    ov = OverlayWindow(root, x=0, y=0, width=400, height=250)
    ov._win.update_idletasks()
    geo = ov._win.geometry()  # 形如 "400x250+0+0"
    assert geo.startswith("400x250+0+0")


def test_geometry_change_callback_fires_on_manual_apply(root):
    saved = []
    ov = OverlayWindow(root, x=10, y=20, width=400, height=250,
                       on_geometry_change=lambda x, y, w, h: saved.append((x, y, w, h)))
    ov._apply_geometry(50, 60, 300, 200)
    ov._win.update_idletasks()
    ov._emit_geometry()
    assert saved and saved[-1][2:] == (300, 200)


def test_set_status_updates_bar_label(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
    assert ov.status_text() == ""
    ov.set_status("listening")
    assert ov.status_text() == t("status.listening")
    ov.set_status("translating")
    assert ov.status_text() == t("status.translating")


def test_thumb_hidden_when_content_fits():
    assert thumb_span(0.0, 1.0, 200) is None       # 全部可見＝不需捲軸
    assert thumb_span(0.0, 0.5, 0) is None         # 軌道還沒有高度


def test_thumb_span_follows_view_fraction():
    assert thumb_span(0.0, 0.5, 200) == (0, 100)
    assert thumb_span(0.5, 1.0, 200) == (100, 200)


def test_thumb_span_keeps_minimum_length_inside_track():
    assert thumb_span(0.0, 0.02, 200, min_thumb=20) == (0, 20)
    # 捲到最底且比例極小：撐到最短長度，但不可超出軌道下緣
    assert thumb_span(0.98, 1.0, 200, min_thumb=20) == (180, 200)


def test_scrollbar_stops_above_resize_grip(root):
    # 把手 place 在視窗右下角：捲軸鋪到底會被壓住，滑塊捲到底時尤其明顯
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
    ov._win.update_idletasks()
    sb = ov._list._scrollbar
    bottom = sb.winfo_rooty() - ov._win.winfo_rooty() + sb.winfo_height()
    assert bottom <= ov._win.winfo_height() - _GRIP_SIZE


def test_scroll_fraction_subtracts_grab_offset_and_clamps():
    assert scroll_fraction(100, 0.0, 200) == 0.5
    assert scroll_fraction(100, 0.25, 200) == 0.25  # 抓在滑塊中段：扣掉偏移
    assert scroll_fraction(-50, 0.0, 200) == 0.0    # 拖出軌道上緣
    assert scroll_fraction(400, 0.0, 200) == 1.0    # 拖出軌道下緣


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
    line = ov._list._messages[index].row.winfo_children()[1]
    return line.itemcget(line.find_withtag("fg")[0], "fill")


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
    """取某則訊息原文行的本色（同 _translation_fill，取 row 第 0 個子件）。"""
    line = ov._list._messages[index].row.winfo_children()[0]
    return line.itemcget(line.find_withtag("fg")[0], "fill")


def test_dimmed_scales_each_channel_toward_dark():
    from src.ui.message_list import dimmed
    assert dimmed("#ffffff") == "#b5b5b5"   # 各通道乘 DIM_FACTOR，原文明顯暗於譯文
    assert dimmed("#80ff00") == "#5bb500"
    assert dimmed("#000000") == "#000000"


def test_message_uses_game_color_translated_bright_original_dim(root):
    # 譯文用遊戲聊天的顯示色，原文用同色調暗版 —— 與遊戲內配色一眼對得上
    from src.ui.message_list import dimmed
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "甲", color="#80ff00")
    assert _translation_fill(ov) == "#80ff00"
    assert _original_fill(ov) == dimmed("#80ff00")


def test_pending_message_restores_game_color_on_update(root):
    # 佔位期間仍用暗灰（語意＝還沒翻好），真譯文落地才換成遊戲色
    from src.ui.message_list import dimmed
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


def test_point_in_rect_edges():
    assert point_in_rect(10, 10, 10, 10, 64, 64) is True
    assert point_in_rect(73, 73, 10, 10, 64, 64) is True
    assert point_in_rect(74, 40, 10, 10, 64, 64) is False   # 右緣外
    assert point_in_rect(9, 40, 10, 10, 64, 64) is False    # 左緣外


def test_should_auto_expand_when_bubble_becomes_foreground():
    # 點工作列按鈕／Alt+Tab 切回本工具：泡泡這一輪才變前景，游標不在泡泡上
    assert should_auto_expand(foreground=101, previous=202, bubble_hwnd=101,
                              cursor_on_bubble=False) is True


def test_no_auto_expand_while_cursor_on_bubble():
    # 直接按泡泡也會讓它變前景；展開與否交給既有的點擊／拖曳邏輯，否則拖不動
    assert should_auto_expand(foreground=101, previous=202, bubble_hwnd=101,
                              cursor_on_bubble=True) is False


def test_no_auto_expand_when_bubble_already_foreground():
    # 已經是前景（例如拖曳結束游標剛離開泡泡）不算切換，不重複觸發
    assert should_auto_expand(foreground=101, previous=101, bubble_hwnd=101,
                              cursor_on_bubble=False) is False


def test_no_auto_expand_for_other_windows():
    assert should_auto_expand(foreground=303, previous=202, bubble_hwnd=101,
                              cursor_on_bubble=False) is False


def test_minimize_starts_foreground_watch_and_expand_stops_it(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    assert ov._watch_job is None
    ov.minimize()
    assert ov._watch_job is not None
    ov.expand()
    assert ov._watch_job is None


def test_failed_translation_line_is_shown_in_error_colour(root):
    # 翻不出來的那則要跟一般對話一眼分得開：譯文行改用錯誤色，不用該則的遊戲色
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文", "翻譯中…", msg_id=7, pending=True, color="#66ccff")
    ov.update_message(7, "⚠  這則訊息翻譯不出來", failed=True)

    line = ov._list._messages[0].row.winfo_children()[1]
    assert line.itemcget("fg", "fill") == FG_ERROR


def test_successful_translation_keeps_the_game_colour(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文", "翻譯中…", msg_id=8, pending=True, color="#66ccff")
    ov.update_message(8, "譯文")

    line = ov._list._messages[0].row.winfo_children()[1]
    assert line.itemcget("fg", "fill") == "#66ccff"


def test_bubble_release_without_press_is_ignored(root):
    # 點 ─ 縮小時 minimize() withdraw 掉正被按住的視窗、隱式 grab 斷掉，放開滑鼠的事件落到
    # 剛出現的泡泡上 —— 沒有對應的 press，不得丟例外（實機 AttributeError），也不得展開
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.minimize()

    class FakeEvent:
        x_root = 10
        y_root = 10

    ov._bubble._release(FakeEvent())
    assert ov.minimized is True


def _filled_overlay(root, width=739, height=350, count=40,
                    max_messages=200, fade_seconds=0):
    """裝滿到需要捲動、且視圖停在最底的 overlay，供自動跟隨的測試當起點。"""
    ov = OverlayWindow(root, x=100, y=100, width=width, height=height,
                       max_messages=max_messages, fade_seconds=fade_seconds)
    root.update()
    for i in range(count):
        ov.add_message(f"message number {i}: a chat line long enough that a narrower "
                       f"window forces it onto a second line",
                       f"第 {i} 則譯文，內容夠長，窄視窗下一定會換行成兩行以上，"
                       f"這樣才測得到重新排版導致的高度變化", msg_id=i)
    root.update()
    assert ov._list._canvas.yview()[1] == 1.0, "前置條件：視圖應停在最底"
    return ov


def _at_bottom(ov) -> bool:
    return should_stick_to_bottom(ov._list._canvas.yview()[1])


def _row_offset(ov, msg_id: int) -> int:
    """某則訊息目前落在視口的哪個 y（畫面座標）。"""
    row = next(m.row for m in ov._list._messages if m.msg_id == msg_id)
    return row.winfo_y() - int(ov._list._canvas.canvasy(0))


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


def test_expand_reanchors_view_to_bottom(root):
    # 泡泡期間的訊息是在 unmap 狀態下排版的，展開後尺寸才確定。expand() 必須自己重算並貼底 ——
    # deiconify 不保證帶來 <Configure>，捲動範圍停在舊值的話最新訊息怎麼捲都捲不到
    ov = _filled_overlay(root)
    ov.minimize()
    root.update()
    for i in range(100, 106):
        ov.add_message(f"bubbled line {i} arriving while the window is a bubble",
                       f"第 {i} 則泡泡期間的譯文，長度足以換行", msg_id=i)
    ov._list._canvas.yview_moveto(0.0)   # 模擬 unmap 期間的排版落差把視圖推離底部
    root.update()
    assert ov._list._follow, "非使用者操作，跟隨狀態不該改變"
    ov.expand()
    root.update()
    assert _at_bottom(ov)


def test_expand_realigns_the_message_container(root):
    # 視窗隱藏期間畫布不重繪，內嵌容器停在舊捲動位置、與 canvas 的捲動帳目脫節：yview 回報
    # 已在底部，畫面卻少了最後幾則（要等下一則新訊息觸發重新佈局才會對齊）
    ov = _filled_overlay(root)
    ov.minimize()
    root.update()
    for i in range(200, 204):
        ov.add_message(f"bubbled line {i} arriving while the window is a bubble",
                       f"第 {i} 則泡泡期間的譯文，長度足以換行", msg_id=i)
        root.update()
    ov.expand()
    root.update()
    assert ov._list._inner.winfo_y() == -int(ov._list._canvas.canvasy(0))


def test_refresh_labels_retranslates_status_and_banner(root):
    from src import i18n

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
        ov.set_status("listening")
        ov.set_error("notice.offline")
        i18n.set_language("en-US")
        ov.refresh_labels()
        assert ov.status_text() == "●  Listening"
        assert ov.error_text() == "⚠  Translation server is offline — retrying…"
    finally:
        i18n.set_language(before)


def test_message_font_follows_language(root):
    # 新訊息的原文／譯文行要用建立當下的介面語言取字型，不能沿用啟動時鎖住的常數
    from src import i18n

    before = i18n.current_language()
    try:
        i18n.set_language("zh-CN")
        ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
        ov.add_message("[A] hi", "嗨", msg_id=1)
        row = ov._list._messages[0].row
        lines = row.winfo_children()  # [原文 canvas, 譯文 canvas]
        fonts = {str(line.itemcget(item, "font"))
                 for line in lines for item in line.find_withtag("txt")}
        assert all("YaHei" in f for f in fonts)
    finally:
        i18n.set_language(before)


def test_window_close_button_runs_the_clean_shutdown(root):
    # Alt+F4 與工作列「關閉視窗」都送 WM_DELETE_WINDOW；沒有 handler 時 Tk 只 destroy 這一個
    # Toplevel，留下 backdrop 孤兒、主迴圈照跑。兩個有工作列按鈕的視窗都必須接回乾淨關閉
    closed = []
    ov = OverlayWindow(root, x=0, y=0, max_messages=10, fade_seconds=0,
                       on_close=lambda: closed.append("win"))

    # tkinter 預設把這個 protocol 設成「destroy 這個 Toplevel」，所以不能只斷言它非空
    handler = ov._win.protocol("WM_DELETE_WINDOW")
    assert not handler.endswith("destroy"), "文字層仍停在 Tk 預設的 destroy 行為"
    root.call(handler)
    assert closed == ["win"]
    assert ov._win.winfo_exists(), "關閉要交給乾淨關閉流程，不是就地拆掉文字層"


def test_bubble_close_button_runs_the_clean_shutdown(root):
    # 泡泡同樣有工作列按鈕：被 Alt+F4 關掉而只 destroy 泡泡的話，主視窗仍是隱藏狀態，
    # 使用者會完全找不到這支程式。
    closed = []
    ov = OverlayWindow(root, x=0, y=0, max_messages=10, fade_seconds=0,
                       on_close=lambda: closed.append("bubble"))
    ov.minimize()

    handler = ov._bubble.protocol("WM_DELETE_WINDOW")
    assert not handler.endswith("destroy"), "泡泡仍停在 Tk 預設的 destroy 行為"
    root.call(handler)
    assert closed == ["bubble"]


def test_set_update_shows_a_clickable_banner(root):
    from src.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    assert ov.update_text() is None
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    assert ov.update_text() == t("update.available", version="0.2.0")
    assert ov._update_label.cget("cursor") == "hand2"
    ov.clear_update()
    assert ov.update_text() is None


def test_update_banner_uses_an_opaque_background(root):
    """回歸測試：橫幅底色不可等於 BG —— 本體視窗把 BG 設成 `-transparentcolor`，
    符合該色的像素在 Windows 下連 hit-test 都跳過，整列與右側 ✕ 就都點不到。"""
    from src.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    row = ov._update_row
    close, label = row.winfo_children()

    assert str(row.cget("bg")) == BG_UPDATE
    assert str(label.cget("bg")) == BG_UPDATE
    assert str(close.cget("bg")) == BG_UPDATE
    assert BG_UPDATE != BG


def test_update_banner_leaves_room_for_the_resize_grip(root):
    """回歸測試：橫幅右側要讓出縮放把手的寬度。

    把手 place 在右下角、底色是透明色鍵，疊在不透明橫幅上會挖出缺口並蓋掉半個 ✕。"""
    from src.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))

    # pack_info 的 padx 在不同 Tk 版本可能是 (0, 16) 序對或 "0 16" 字串
    padx = ov._update_row.pack_info()["padx"]
    right = padx[-1] if isinstance(padx, (tuple, list)) else str(padx).split()[-1]
    assert int(str(right)) == _GRIP_SIZE, f"右側未讓出把手寬度：padx={padx}"


def test_error_banner_formats_the_api_message_and_survives_language_switch(root):
    from src import i18n

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
        ov.set_error("notice.config_error_detail", status=401, message="Incorrect API key")
        assert ov.error_text() == t("notice.config_error_detail", status=401,
                                    message="Incorrect API key")
        assert "Incorrect API key" in ov.error_text()
        i18n.set_language("en-US")
        ov.refresh_labels()   # 換語言重繪時 format 變數不可丟失
        assert ov.error_text() == t("notice.config_error_detail", status=401,
                                    message="Incorrect API key")
        assert "Incorrect API key" in ov.error_text()
    finally:
        i18n.set_language(before)


def test_error_banner_makes_urls_in_the_api_message_clickable(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
    ov.set_error("notice.config_error_detail", status=401,
                 message="see https://a.example/keys for a key")
    assert "https://a.example/keys" in ov.error_text()
    assert ov._error_label.links() == [("https://a.example/keys", "https://a.example/keys")]
    ov.clear_error()
    assert ov.error_text() is None


def test_update_banner_and_error_banner_coexist(root):
    # 兩者生命週期完全不同（錯誤隨狀態來去、更新是一次性），不該互相覆蓋
    from src.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_error("notice.offline")
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    assert ov.error_text() == t("notice.offline")
    assert ov.update_text() == t("update.available", version="0.2.0")
    ov.clear_update()
    assert ov.error_text() == t("notice.offline")   # 關掉更新橫幅不影響錯誤橫幅


def test_update_banner_follows_language_and_width(root):
    from src import i18n
    from src.updater import Release

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
        ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
        i18n.set_language("en-US")
        ov.refresh_labels()
        assert ov.update_text() == "⬆  Version 0.2.0 is available — click to download"

        class FakeEvent:
            width = 240

        ov._list._on_canvas_configure(FakeEvent())
        assert ov._update_label.cget("wraplength") == max(80, 240 - 12)
    finally:
        i18n.set_language(before)


def test_every_status_has_both_a_label_and_a_colour():
    """set_status 直接查 STATUS_COLORS[state]，漏一個顏色＝執行期 KeyError，
    而狀態列出現的時機（掛不進遊戲）正是最不該再炸一次的時候。"""
    import json
    from pathlib import Path
    catalog = json.loads(
        (Path(__file__).resolve().parent.parent / "src" / "i18n" / "zh-TW.json")
        .read_text(encoding="utf-8"))
    labelled = {k.removeprefix("status.") for k in catalog if k.startswith("status.")}
    assert labelled == set(STATUS_COLORS)


def test_wrapped_banners_stay_left_aligned(root):
    """tk.Label 多行預設置中：版本不相容這類長橫幅換行後會歪成階梯狀。"""
    from types import SimpleNamespace
    ov = OverlayWindow(root, x=0, y=0, width=320, height=200,
                       max_messages=10, fade_seconds=0)
    ov.set_update(SimpleNamespace(version="1.2.3", url="https://example.invalid"))
    assert str(ov._update_label.cget("justify")) == "left"


def test_refresh_labels_keeps_the_title_free_of_the_old_menu_glyph(root):
    from src import i18n
    from src.config import app_name

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
        initial = ov._title_label.cget("text")
        i18n.set_language("en-US")
        ov.refresh_labels()
        # 標題列早已用 icon 取代 ≡，換語言重繪後也不該把它加回來
        assert ov._title_label.cget("text") == app_name()
        assert "≡" not in initial and "≡" not in ov._title_label.cget("text")
    finally:
        i18n.set_language(before)


def _select_whole_message(ov, index=0):
    """把第 index 則訊息整則選起來，回傳它的兩個行 canvas。"""
    ov._win.update_idletasks()
    first, second = ov._list._messages[index].row.winfo_children()
    ov._selection.begin(first.winfo_rootx() + TEXT_ORIGIN,
                        first.winfo_rooty() + first.winfo_height() // 2)
    ov._selection.extend(second.winfo_rootx() + 1000,
                         second.winfo_rooty() + second.winfo_height() // 2)
    return first, second


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


def test_minimize_clears_the_selection(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)

    ov.minimize()

    assert ov._selection.active is False
    ov.expand()


def test_resize_redraws_the_highlight_to_the_new_wrapping(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    # *10（非 *20）：縮放前要留在同一視覺行，_select_whole_message 的垂直置中點
    # 才會落在該行內、真的從頭選起 —— 量測顯示 *10 在縮放前的換行寬度下恰好一行、
    # *20 已經先換成兩行，命中點會落在行界上、選不到開頭（見 fix-round 報告）。
    ov.add_message("原文一" * 10, "譯文一" * 10)
    first, _ = _select_whole_message(ov)
    selected = ov._selection.text()

    class FakeEvent:
        width = 240

    ov._list._on_canvas_configure(FakeEvent())
    ov._win.update_idletasks()

    rects = first.find_withtag("sel")
    assert ov._selection.text() == selected
    # 每個視覺行一個反白矩形：換行變了、矩形數就要跟著變
    assert len(rects) == len(visual_lines(first, line_font(first)))
    # 沒有重畫的話，矩形仍是舊換行寬度下的幾何，右緣會超出新的換行寬度
    assert max(first.coords(i)[2] for i in rects) <= ov._list.wrap + TEXT_ORIGIN


class _Press:
    """假的滑鼠事件（只用到螢幕座標）。"""

    def __init__(self, x_root, y_root):
        self.x_root = x_root
        self.y_root = y_root
        self.widget = None


def test_press_on_a_message_starts_a_selection(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    # update()（非 update_idletasks）：_in_message_area 量的是 ov._list._canvas 的實際尺寸，
    # 這個 expand=True 的捲動畫布在全新 overrideredirect 視窗裡，只有真的跑過一輪
    # 事件迴圈（Windows 送 WM_SIZE）才會拿到非 1x1 的量測值，idle 佇列處理不到這段
    ov._win.update()
    first, second = ov._list._messages[0].row.winfo_children()

    ov._selection_press(_Press(first.winfo_rootx() + TEXT_ORIGIN,
                               first.winfo_rooty() + first.winfo_height() // 2))
    ov._selection_drag(_Press(second.winfo_rootx() + 1000,
                              second.winfo_rooty() + second.winfo_height() // 2))
    ov._selection_release(_Press(0, 0))

    assert ov._selection.text() == "原文一\n譯文一"


def test_press_on_a_row_scrolled_out_of_view_is_ignored(root):
    # 捲出視口的訊息列仍留著幾何位置：少了 _in_message_area 這道關卡，點在標題列
    # 附近就會選到看不見的訊息
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=50, fade_seconds=0)
    for i in range(20):
        ov.add_message(f"原文{i}", f"譯文{i}")
    ov._win.update()
    hidden_top, hidden_bottom = ov._list._messages[0].row.winfo_children()
    assert hidden_top.winfo_rooty() < ov._list._canvas.winfo_rooty(), \
        "第一則應該已經捲出視口上方，否則這個測試沒有守到東西"

    ov._selection_press(_Press(hidden_top.winfo_rootx() + 40,
                               hidden_top.winfo_rooty() + hidden_top.winfo_height() // 2))
    ov._selection_drag(_Press(hidden_bottom.winfo_rootx() + 1000,
                              hidden_bottom.winfo_rooty() + hidden_bottom.winfo_height() // 2))

    assert ov._selection.dragging is False
    assert ov._selection.text() == ""


def test_backdrop_press_away_from_any_edge_starts_a_selection(root):
    # 訊息列的底色是透明色鍵，字間空隙的點擊會落到 backdrop —— 那條路徑也要能起手
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    ov._win.update()   # 同上：_in_message_area 要量到真實的 canvas 尺寸
    first, second = ov._list._messages[0].row.winfo_children()

    # +40（非 +TEXT_ORIGIN）：捲動區沒有留邊，行 canvas 左緣與視窗左緣重合，
    # +TEXT_ORIGIN 會落在 EDGE 縮放感應帶內、被 _edge_press 誤判成縮放
    ov._edge_press(_Press(first.winfo_rootx() + 40,
                          first.winfo_rooty() + first.winfo_height() // 2))
    ov._edge_drag(_Press(second.winfo_rootx() + 1000,
                         second.winfo_rooty() + second.winfo_height() // 2))
    ov._edge_release(_Press(0, 0))

    assert ov._selection.text().endswith("\n譯文一")
    assert ov._selection.active is True
    assert ov._resize is None


@pytest.mark.real_position
def test_backdrop_press_on_an_edge_still_resizes(root):
    ov = OverlayWindow(root, x=200, y=200, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov._win.update_idletasks()

    ov._edge_press(_Press(200, 200 + 150))   # 左緣

    assert ov._resize is not None
    assert ov._selection.active is False
    ov._edge_release(_Press(200, 350))


def test_view_does_not_jump_to_the_bottom_while_selecting(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=50, fade_seconds=0)
    for i in range(20):
        ov.add_message(f"原文{i}", f"譯文{i}")
    ov._win.update()   # 同上：_in_message_area 要量到真實的 canvas 尺寸
    # 用最後一則（非第一則）：視圖貼底時第一則已捲出視口，_in_message_area
    # 會擋下這次按下、根本起不了選取
    first, second = ov._list._messages[-1].row.winfo_children()
    ov._selection_press(_Press(first.winfo_rootx() + TEXT_ORIGIN,
                               first.winfo_rooty() + first.winfo_height() // 2))
    ov._list._canvas.yview_moveto(0.0)
    ov._win.update_idletasks()
    before = ov._list._canvas.yview()[0]

    ov._list.refresh_scroll()

    assert ov._list._canvas.yview()[0] == before
    ov._selection_release(_Press(0, 0))


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
    first, second = ov._list._messages[-1].row.winfo_children()
    ov._selection_press(_Press(first.winfo_rootx() + TEXT_ORIGIN,
                               first.winfo_rooty() + first.winfo_height() // 2))
    ov._selection_drag(_Press(second.winfo_rootx() + 1000,
                              second.winfo_rooty() + second.winfo_height() // 2))
    selected = ov._selection.text()

    for i in range(3):
        ov.add_message(f"拖曳中{i}", f"拖曳中譯文{i}", now=2000.0)
    ov._win.update()
    before_y = second.winfo_rooty()

    ov.prune(now=2000.0)   # 最舊的 6 則過期（0.0 + 60 < 2000），其餘保留
    ov._win.update()

    assert ov._selection.text() == selected
    assert abs(second.winfo_rooty() - before_y) <= 1, \
        "拖曳中的那一列不該因為上方訊息被 prune 而在畫面上移動"


def test_copy_writes_only_when_something_is_selected(root):
    # 兩個斷言刻意合成一個測試：剪貼簿是全機器共用的資源，拆成兩個測試在
    # pytest-xdist 的 4 個 worker 下會互相覆蓋（addopts 的 -n 4）
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)

    ov.copy_selection()
    assert root.clipboard_get() == "原文一\n譯文一"

    ov._selection.clear("test")
    ov.copy_selection()
    assert root.clipboard_get() == "原文一\n譯文一", "沒有選取時不該動剪貼簿"


def test_right_click_without_a_selection_pops_no_menu(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")

    ov._selection_menu(_Press(300, 300))
    ov._win.update()

    assert ov._popup.visible is False


def test_right_click_with_a_selection_pops_the_themed_menu(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)

    ov._selection_menu(_Press(300, 300))
    ov._win.update()

    assert ov._popup.visible is True
    assert ov._popup.label_text() == t("menu.copy")


def test_choosing_copy_from_the_menu_copies_and_closes(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)
    ov._selection_menu(_Press(300, 300))
    ov._win.update()

    ov._popup._clicked(None)
    ov._win.update()

    assert root.clipboard_get() == "原文一\n譯文一"
    assert ov._popup.visible is False


def test_clicking_a_message_closes_the_menu(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    first, _ = _select_whole_message(ov)
    ov._selection_menu(_Press(300, 300))
    ov._win.update()

    first.event_generate("<ButtonPress-1>", x=2, y=2)
    ov._win.update()

    assert ov._popup.visible is False


def test_clicking_the_title_bar_closes_the_menu(root):
    # 「點別的地方就關掉」不能只涵蓋訊息區：標題列、捲軸、右下把手都是使用者會直覺
    # 點的地方，事件沿 bindtags 傳到 toplevel，綁在那裡才全部涵蓋得到
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)
    ov._selection_menu(_Press(300, 300))
    ov._win.update()

    ov._title_label.event_generate("<ButtonPress-1>", x=2, y=2)
    ov._win.update()

    assert ov._popup.visible is False


def test_autoscroll_is_still_inside_the_viewport():
    assert autoscroll_pixels(500, 400, 600) == 0
    assert autoscroll_pixels(400, 400, 600) == 0
    assert autoscroll_pixels(600, 400, 600) == 0


def test_autoscroll_goes_up_above_the_viewport_and_down_below_it():
    assert autoscroll_pixels(392, 400, 600) < 0
    assert autoscroll_pixels(608, 400, 600) > 0


def test_autoscroll_speeds_up_with_distance_but_is_capped():
    near = autoscroll_pixels(610, 400, 600, lo=2, hi=24)
    far = autoscroll_pixels(650, 400, 600, lo=2, hi=24)
    assert 0 < near < far < 24
    assert autoscroll_pixels(9000, 400, 600, lo=2, hi=24) == 24


def _tick_autoscroll(ov, times=20):
    """手動跑幾輪自動捲動。

    每輪都要讓 Tk 重新排版：`winfo_rooty()` 回報的是上次排版的位置，連續同步呼叫
    而不 update 的話，caret 會一直算在捲動前的那一列上。正式路徑每輪是獨立的
    `after` 回呼，中間本來就有事件迴圈。每輪自己排的下一輪也要取消，免得留
    after 排程給別的測試。"""
    for _ in range(times):
        ov._autoscroll()
        ov._stop_autoscroll()
        ov._win.update()


def test_dragging_above_the_viewport_scrolls_and_grows_the_selection(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=50, fade_seconds=0)
    for i in range(20):
        ov.add_message(f"原文{i}", f"譯文{i}")
    ov._win.update()
    first, second = ov._list._messages[-1].row.winfo_children()
    ov._selection_press(_Press(first.winfo_rootx() + TEXT_ORIGIN,
                               first.winfo_rooty() + first.winfo_height() // 2))
    ov._selection_drag(_Press(second.winfo_rootx() + 10,
                              ov._list._canvas.winfo_rooty() - 60))
    ov._stop_autoscroll()
    ov._win.update()
    view_before, text_before = ov._list._canvas.yview()[0], ov._selection.text()

    _tick_autoscroll(ov)
    ov._win.update()

    assert ov._list._canvas.yview()[0] < view_before, "拖到視口上方應該要往上捲"
    assert len(ov._selection.text()) > len(text_before), "捲動後選取要跟著長出來"


def test_autoscroll_stops_once_the_drag_ends(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=50, fade_seconds=0)
    for i in range(20):
        ov.add_message(f"原文{i}", f"譯文{i}")
    ov._win.update()
    first, _ = ov._list._messages[-1].row.winfo_children()
    ov._selection_press(_Press(first.winfo_rootx() + TEXT_ORIGIN,
                               first.winfo_rooty() + first.winfo_height() // 2))
    ov._selection_drag(_Press(first.winfo_rootx() + 10,
                              ov._list._canvas.winfo_rooty() - 60))
    ov._selection_release(_Press(0, 0))
    ov._win.update()
    view_after_release = ov._list._canvas.yview()[0]

    _tick_autoscroll(ov)
    ov._win.update()

    assert ov._list._canvas.yview()[0] == view_after_release
    assert ov._autoscroll_job is None


def test_minimize_closes_the_menu(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)
    ov._selection_menu(_Press(300, 300))
    ov._win.update()

    ov.minimize()
    ov._win.update()

    assert ov._popup.visible is False
    ov.expand()


def test_selection_entry_points_are_bound(root):
    # 所有既有測試都直接呼叫 handler，綁定整組刪掉也不會轉紅 —— 這條守住實際入口
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")

    assert ov._win.bind("<Control-c>")
    assert ov._win.bind("<Control-C>")
    assert ov._backdrop.bind("<Button-3>")
    for line in ov._list._messages[0].row.winfo_children():
        for sequence in ("<ButtonPress-1>", "<B1-Motion>",
                         "<ButtonRelease-1>", "<Button-3>"):
            assert line.bind(sequence), f"{sequence} 未綁定"


def test_offer_update_shows_the_banner_when_nothing_was_dismissed(root):
    from src.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.offer_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    assert ov.update_text() == t("update.available", version="0.2.0")
    ov.clear_update()


def test_offer_update_skips_a_version_the_user_dismissed(root):
    """每小時自動檢查會反覆查到同一版：使用者按 ✕ 關掉後，同版（或更舊）不再跳出，
    只有更新的版本才再顯示。"""
    from src.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    ov._dismiss_update()
    assert ov.update_text() is None

    ov.offer_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    assert ov.update_text() is None
    ov.offer_update(Release(version="0.3.0", url="https://example.invalid/rel"))
    assert ov.update_text() == t("update.available", version="0.3.0")
    ov.clear_update()


def test_set_update_ignores_the_dismissed_version(root):
    """手動檢查走 set_update：使用者自己按了「檢查更新」，關掉過的版本也要再顯示。"""
    from src.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    ov._dismiss_update()
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    assert ov.update_text() == t("update.available", version="0.2.0")
    ov.clear_update()


def test_region_button_invokes_the_callback_when_provided(root):
    calls = []
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       on_region=lambda: calls.append("region"))
    ov._win.update()
    ov._region_btn.event_generate("<Button-1>")
    ov._win.update()
    assert calls == ["region"]


def test_region_button_absent_without_a_callback(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
    assert ov._region_btn is None


def test_clear_update_does_not_count_as_dismissing(root):
    """程式自己收橫幅（clear_update）不是使用者的決定，之後同版仍該再提醒。"""
    from src.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    ov.clear_update()
    ov.offer_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    assert ov.update_text() == t("update.available", version="0.2.0")
    ov.clear_update()
