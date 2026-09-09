"""conftest 的 `_park_windows`：跑測試時視窗不該彈到使用者畫面上搶焦點。"""
import tkinter as tk

import pytest


def test_windows_are_parked_offscreen(root):
    win = tk.Toplevel(root)
    try:
        win.geometry("200x100+50+60")
        win.update_idletasks()
        assert win.winfo_x() >= 9000, "視窗應被停到螢幕外"
        assert win.winfo_y() >= 9000, "視窗應被停到螢幕外"
    finally:
        win.destroy()


def test_parking_keeps_the_requested_size(root):
    """只改寫位置、不動尺寸 —— 量排版的測試靠的就是真實尺寸。"""
    win = tk.Toplevel(root)
    try:
        win.geometry("240x120+10+10")
        win.update_idletasks()
        assert (win.winfo_width(), win.winfo_height()) == (240, 120)
    finally:
        win.destroy()


@pytest.mark.real_position
def test_marked_tests_keep_their_real_position(root):
    """斷言視窗位置的測試要拿得到真實座標，否則等於改壞被測目標。"""
    win = tk.Toplevel(root)
    try:
        win.geometry("200x100+50+60")
        win.update_idletasks()
        assert (win.winfo_x(), win.winfo_y()) == (50, 60)
    finally:
        win.destroy()
