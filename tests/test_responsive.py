import tkinter as tk
from tkinter import ttk

from src.ui.responsive import MIN_WRAP, apply_wrap, bind_wrap, wrap_width


def test_wrap_width_subtracts_reserved_space():
    assert wrap_width(640, reserved=40) == 600


def test_wrap_width_never_goes_below_minimum():
    # 視窗被拖到極窄時仍給文字一個可讀寬度，不會算出 0 或負數
    assert wrap_width(80, reserved=40) == MIN_WRAP


def test_apply_wrap_sets_wraplength_and_reports_change(root):
    label = tk.Label(root, text="說明文字")
    assert apply_wrap(label, 500, reserved=40) is True
    assert label.cget("wraplength") == 460


def test_apply_wrap_reports_no_change_when_width_is_same(root):
    # 值沒變就不寫入：Configure → wraplength → Configure 的回圈由此打斷
    label = tk.Label(root, text="說明文字")
    apply_wrap(label, 500, reserved=40)
    assert apply_wrap(label, 500, reserved=40) is False


def test_bind_wrap_follows_container_width(root):
    # 容器變寬 → 換行寬度跟著變寬（設定視窗放大後說明文字要鋪開，不能卡在原寬度）
    win = tk.Toplevel(root)
    win.geometry("400x120")
    frame = tk.Frame(win)
    frame.pack(fill="both", expand=True)
    label = tk.Label(frame, text="說明文字" * 20)
    label.pack(side="left")
    bind_wrap(label)
    win.update()
    narrow = int(label.cget("wraplength"))

    win.geometry("700x120")
    win.update()
    assert int(label.cget("wraplength")) > narrow
    win.destroy()


def test_bind_wrap_reserves_space_taken_by_earlier_widgets_in_the_row(root):
    # 說明文字擺在同列的標籤與輸入框右邊：換行寬度只能用它右邊剩下的空間，
    # 否則文字會溢出視窗右緣
    win = tk.Toplevel(root)
    win.geometry("500x80")
    row = tk.Frame(win)
    row.pack(fill="x")
    tk.Label(row, text="輪詢間隔（秒）", width=14).pack(side="left")
    tk.Spinbox(row, width=8).pack(side="left")
    hint = tk.Label(row, text="收訊掃描頻率" * 10)
    hint.pack(side="left")
    bind_wrap(hint)
    win.update()

    assert hint.winfo_x() > 0  # 前面的控件確實佔掉了寬度
    assert int(hint.cget("wraplength")) <= row.winfo_width() - hint.winfo_x()
    win.destroy()


def test_bind_wrap_ignores_unmapped_container(root):
    # 視窗尚未 map 時 winfo_width() 回 1：此時算出的換行寬度沒有意義，不能寫進去
    # （寫了會讓文字一開窗就折成一長條，並連帶把自適應高度撐大）
    win = tk.Toplevel(root)
    label = tk.Label(win, text="說明文字" * 20, wraplength=400)
    label.pack(fill="x")
    bind_wrap(label, container=win)
    win.update_idletasks()
    assert int(label.cget("wraplength")) == 400
    win.destroy()


def test_apply_wrap_handles_label_without_wraplength(root):
    # ttk 的 Label 沒設過 wraplength 時 cget 回空字串（tk 回 0）：不能拿去 int()
    label = ttk.Label(root, text="說明文字")
    assert apply_wrap(label, 500, reserved=40) is True
    assert int(label.cget("wraplength")) == 460


def test_bind_wrap_settles_on_the_same_width_regardless_of_history(root):
    # 同樣的容器寬度必須算出同樣的換行寬度：Configure 當下量到的是過渡值，
    # 先拉大再拉回來就會停在與直接設定不同的結果（輸入框高度因此忽高忽低）
    def wrap_after(widths):
        win = tk.Toplevel(root)
        frame = tk.Frame(win)
        frame.pack(fill="both", expand=True)
        tk.Label(frame, text="標籤", width=10).pack(side="left")
        label = tk.Label(frame, text="說明文字" * 20)
        label.pack(side="left", padx=8)
        bind_wrap(label)
        for width in widths:
            win.geometry(f"{width}x120")
            win.update()
        result = int(label.cget("wraplength"))
        win.destroy()
        return result

    assert wrap_after([700]) == wrap_after([700, 1300, 700])
