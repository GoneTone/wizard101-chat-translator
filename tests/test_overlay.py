import tkinter as tk

import pytest

from src.reader.overlay import OverlayWindow


@pytest.fixture(scope="module")
def root():
    r = tk.Tk()
    r.withdraw()
    yield r
    r.destroy()


def test_add_message_appends_and_caps_at_max(root):
    ov = OverlayWindow(root, x=0, y=0, max_messages=3, fade_seconds=180)
    for i in range(5):
        ov.add_message(f"msg {i}", f"訊息 {i}")
    texts = ov.visible_messages()
    assert len(texts) == 3
    assert texts[-1] == ("msg 4", "訊息 4")
    assert texts[0] == ("msg 2", "訊息 2")


def test_prune_removes_expired(root):
    ov = OverlayWindow(root, x=0, y=0, fade_seconds=10)
    ov.add_message("old", "舊", now=100.0)
    ov.add_message("new", "新", now=105.0)
    ov.prune(now=111.0)  # 100+10 < 111 過期;105+10 >= 111 保留
    assert ov.visible_messages() == [("new", "新")]


def test_error_banner_toggle(root):
    ov = OverlayWindow(root, x=0, y=0)
    assert ov.error_text() is None
    ov.set_error("⚠ 翻譯伺服器離線")
    assert ov.error_text() == "⚠ 翻譯伺服器離線"
    ov.clear_error()
    assert ov.error_text() is None


def test_explicit_zero_position_is_honored(root):
    ov = OverlayWindow(root, x=0, y=0)
    ov._win.update_idletasks()
    geom = ov._win.geometry()
    assert geom.endswith("+0+0"), f"Expected geometry to end with '+0+0', got {geom}"
