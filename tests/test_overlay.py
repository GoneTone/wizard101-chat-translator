from src.reader.overlay import (
    MIN_HEIGHT,
    MIN_WIDTH,
    OverlayWindow,
    _GRIP_SIZE,
    is_click,
    moved_to,
    resized_to,
    scroll_fraction,
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
    ov.set_error("錯誤橫幅")

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


def test_resized_to_adds_delta():
    assert resized_to(400, 300, 50, 20, MIN_WIDTH, MIN_HEIGHT) == (450, 320)


def test_resized_to_clamps_to_minimum():
    assert resized_to(400, 300, -1000, -1000, MIN_WIDTH, MIN_HEIGHT) == (MIN_WIDTH, MIN_HEIGHT)


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
    ov.set_error("⚠ 翻譯伺服器離線")
    assert ov.error_text() == "⚠ 翻譯伺服器離線"
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
    ov.set_status("●  監聽中", "#7dc87d")
    assert ov.status_text() == "●  監聽中"
    ov.set_status("●  翻譯中…", "#6fa8dc")
    assert ov.status_text() == "●  翻譯中…"


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
    ov.set_status("●  連線遊戲中…", "#e0b050")
    assert ov.placeholder_visible() is True          # 沒訊息 → 置中顯示狀態
    assert ov.status_text() == "●  連線遊戲中…"
    ov.add_message("[A] hi", "譯文")
    assert ov.placeholder_visible() is False         # 有訊息 → 收掉
    ov.set_status("●  監聽中", "#7dc87d")
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
    assert dimmed("#ffffff") == "#949494"   # 各通道乘 DIM_FACTOR，原文明顯暗於譯文
    assert dimmed("#80ff00") == "#4a9400"
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
