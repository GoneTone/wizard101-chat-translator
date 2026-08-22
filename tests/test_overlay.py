from src.reader.overlay import (
    MIN_HEIGHT,
    MIN_WIDTH,
    OverlayWindow,
    is_click,
    moved_to,
    resized_to,
    should_stick_to_bottom,
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
    for _, _, _, row in ov._messages:
        for child in row.winfo_children():
            assert child.cget("wraplength") == expected
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


def test_placeholder_centered_when_empty_hidden_after_message(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
    ov.set_status("●  連線遊戲中…", "#e0b050")
    assert ov.placeholder_visible() is True          # 沒訊息 → 置中顯示狀態
    assert ov.status_text() == "●  連線遊戲中…"
    ov.add_message("[A] hi", "譯文")
    assert ov.placeholder_visible() is False         # 有訊息 → 收掉
    ov.set_status("●  監聽中", "#7dc87d")
    assert ov.placeholder_visible() is False         # 有訊息時更新狀態也不重現
