from src.i18n import t
from src.reader.overlay import (
    FG_ERROR,
    MIN_HEIGHT,
    MIN_WIDTH,
    OverlayWindow,
    _GRIP_SIZE,
    edge_at,
    is_click,
    moved_to,
    point_in_rect,
    resized_edge,
    scroll_fraction,
    should_auto_expand,
    should_stick_to_bottom,
    thumb_span,
)


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

    ov._on_canvas_configure(FakeEvent())
    expected = max(80, 240 - 12)
    for entry in ov._messages:
        for child in entry.row.winfo_children():
            assert int(float(child.itemcget("txt", "width"))) == expected
    assert ov._error_label.cget("wraplength") == expected


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
    sb = ov._scrollbar
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
    line = ov._messages[index].row.winfo_children()[1]
    return line.itemcget(line.find_withtag("fg")[0], "fill")


def test_pending_placeholder_uses_dimmer_colour_until_filled(root):
    # 佔位期間譯文欄位要能一眼與已翻好的訊息區分，填入真正的譯文後恢復正常顏色
    from src.reader.overlay import FG_PENDING, FG_TRANSLATED
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "翻譯中…", msg_id=1, pending=True)
    assert _translation_fill(ov) == FG_PENDING
    ov.update_message(1, "甲")
    assert _translation_fill(ov) == FG_TRANSLATED


def test_completed_message_is_not_dimmed(root):
    from src.reader.overlay import FG_TRANSLATED
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "甲")          # 非佔位：直接就是完成品
    assert _translation_fill(ov) == FG_TRANSLATED


def _original_fill(ov, index=0):
    """取某則訊息原文行的本色（同 _translation_fill，取 row 第 0 個子件）。"""
    line = ov._messages[index].row.winfo_children()[0]
    return line.itemcget(line.find_withtag("fg")[0], "fill")


def test_dimmed_scales_each_channel_toward_dark():
    from src.reader.overlay import dimmed
    assert dimmed("#ffffff") == "#b5b5b5"   # 各通道乘 DIM_FACTOR，原文明顯暗於譯文
    assert dimmed("#80ff00") == "#5bb500"
    assert dimmed("#000000") == "#000000"


def test_message_uses_game_color_translated_bright_original_dim(root):
    # 譯文用遊戲聊天的顯示色，原文用同色調暗版——與遊戲內配色一眼對得上
    from src.reader.overlay import dimmed
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "甲", color="#80ff00")
    assert _translation_fill(ov) == "#80ff00"
    assert _original_fill(ov) == dimmed("#80ff00")


def test_pending_message_restores_game_color_on_update(root):
    # 佔位期間仍用暗灰（語意＝還沒翻好），真譯文落地才換成遊戲色
    from src.reader.overlay import FG_PENDING, dimmed
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "翻譯中…", msg_id=1, pending=True, color="#80ff00")
    assert _translation_fill(ov) == FG_PENDING
    assert _original_fill(ov) == dimmed("#80ff00")
    ov.update_message(1, "甲")
    assert _translation_fill(ov) == "#80ff00"


def test_message_without_color_falls_back_to_default_palette(root):
    # 讀不到遊戲色（理論上不會發生，防衛用）：維持現行預設配色
    from src.reader.overlay import FG_ORIGINAL, FG_TRANSLATED
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

    line = ov._messages[0].row.winfo_children()[1]
    assert line.itemcget("fg", "fill") == FG_ERROR


def test_successful_translation_keeps_the_game_colour(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文", "翻譯中…", msg_id=8, pending=True, color="#66ccff")
    ov.update_message(8, "譯文")

    line = ov._messages[0].row.winfo_children()[1]
    assert line.itemcget("fg", "fill") == "#66ccff"


def test_bubble_release_without_press_is_ignored(root):
    # 點標題列的 ─ 縮小時，minimize() withdraw 掉正被按住的視窗、隱式 grab 斷掉，
    # 放開滑鼠的事件會落到剛出現在游標下的泡泡上——沒有對應的 press，
    # 既不得丟例外（實機 log 有 AttributeError），也不得把剛收起的視窗展開
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.minimize()

    class FakeEvent:
        x_root = 10
        y_root = 10

    ov._bubble_release(FakeEvent())
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
    assert ov._canvas.yview()[1] == 1.0, "前置條件：視圖應停在最底"
    return ov


def _at_bottom(ov) -> bool:
    return should_stick_to_bottom(ov._canvas.yview()[1])


def _row_offset(ov, msg_id: int) -> int:
    """某則訊息目前落在視口的哪個 y（畫面座標）。"""
    row = next(m.row for m in ov._messages if m.msg_id == msg_id)
    return row.winfo_y() - int(ov._canvas.canvasy(0))


def _scroll_up(ov, fraction: float = 0.3) -> None:
    ov._canvas.yview_moveto(fraction)
    ov._note_scroll()
    assert not ov._follow, "前置條件：往上捲後應停止跟隨底部"


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
    ov._canvas.yview_moveto(0.0)
    ov._note_scroll()
    ov.add_message("newest", "最新", msg_id=998)
    root.update()
    assert not _at_bottom(ov)
    # 捲回底部後恢復自動跟隨
    ov._canvas.yview_moveto(1.0)
    ov._note_scroll()
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
                           "一行撐成三行以上——高度一變，底下的所有訊息都會跟著往下移，"
                           "使用者正在讀的那幾行也就跟著跑掉了，所以這裡要一起補位。"
                           "再多墊一句，確保在寬視窗下也一定會換行成好幾行。")
    root.update()
    assert _row_offset(ov, 605) == before


def test_expand_reanchors_view_to_bottom(root):
    # 泡泡期間的訊息是在 unmap 狀態下排版的，展開後尺寸才真正確定。
    # expand() 必須自己重算並貼底——deiconify 不保證帶來 <Configure>，
    # 沒有這一步的話捲動範圍停在舊值，最新訊息怎麼捲都捲不到。
    ov = _filled_overlay(root)
    ov.minimize()
    root.update()
    for i in range(100, 106):
        ov.add_message(f"bubbled line {i} arriving while the window is a bubble",
                       f"第 {i} 則泡泡期間的譯文，長度足以換行", msg_id=i)
    ov._canvas.yview_moveto(0.0)   # 模擬 unmap 期間的排版落差把視圖推離底部
    root.update()
    assert ov._follow, "非使用者操作，跟隨狀態不該改變"
    ov.expand()
    root.update()
    assert _at_bottom(ov)


def test_expand_realigns_the_message_container(root):
    # 視窗隱藏期間畫布不重繪，內嵌的訊息容器會停在舊的捲動位置，與 canvas 自己的
    # 捲動帳目脫節：yview 回報已在底部，畫面卻少了最後幾則，往下也捲不動
    # （要等下一則新訊息改變容器尺寸、觸發重新佈局才會對齊）。
    ov = _filled_overlay(root)
    ov.minimize()
    root.update()
    for i in range(200, 204):
        ov.add_message(f"bubbled line {i} arriving while the window is a bubble",
                       f"第 {i} 則泡泡期間的譯文，長度足以換行", msg_id=i)
        root.update()
    ov.expand()
    root.update()
    assert ov._inner.winfo_y() == -int(ov._canvas.canvasy(0))


def test_refresh_labels_retranslates_status_and_banner(root):
    from src import i18n

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
        ov.set_status("listening")
        ov.set_error("notice.offline")
        i18n.set_language("en")
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
        row = ov._messages[0].row
        lines = row.winfo_children()  # [原文 canvas, 譯文 canvas]
        fonts = {str(line.itemcget(item, "font"))
                 for line in lines for item in line.find_withtag("txt")}
        assert all("YaHei" in f for f in fonts)
    finally:
        i18n.set_language(before)


def test_window_close_button_runs_the_clean_shutdown(root):
    # Alt+F4 與工作列右鍵「關閉視窗」都送 WM_DELETE_WINDOW；沒有 handler 時 Tk 只會
    # destroy 這一個 Toplevel，留下 backdrop 那層半透明底板孤兒在畫面上、主迴圈照跑。
    # 兩個有工作列按鈕的視窗都必須把它接回和 ✕ 一樣的乾淨關閉。
    closed = []
    ov = OverlayWindow(root, x=0, y=0, max_messages=10, fade_seconds=0,
                       on_close=lambda: closed.append("win"))

    # tkinter predefines this protocol as "destroy this Toplevel"，所以不能只斷言它非空
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
