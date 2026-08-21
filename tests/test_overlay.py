from src.reader.overlay import (
    MIN_HEIGHT,
    MIN_WIDTH,
    OverlayWindow,
    moved_to,
    resized_to,
    should_stick_to_bottom,
)


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
    # visible_messages 依時間順序(舊→新)回傳,保留最新 3 則
    assert texts[-1] == ("msg 4", "訊息 4")
    assert texts[0] == ("msg 2", "訊息 2")


def test_prune_removes_expired(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=10)
    ov.add_message("old", "舊", now=100.0)
    ov.add_message("new", "新", now=105.0)
    ov.prune(now=111.0)  # 100+10 < 111 過期;105+10 >= 111 保留
    assert ov.visible_messages() == [("new", "新")]


def test_no_fade_when_fade_seconds_zero(root):
    # fade_seconds=0:永不依時間清除
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
    ov.set_status("● 監聽中", "#7dc87d")
    assert ov.status_text() == "● 監聽中"
    ov.set_status("● 翻譯中…", "#6fa8dc")
    assert ov.status_text() == "● 翻譯中…"
