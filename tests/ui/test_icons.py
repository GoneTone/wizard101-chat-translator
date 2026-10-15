"""視窗圖示：`apply_window_icon` 的容錯與掛到視窗類別。"""
from src.ui.icons import apply_window_icon


def test_apply_window_icon_survives_missing_file(root, monkeypatch, tmp_path):
    monkeypatch.setattr("src.ui.icons.icon_path", lambda: tmp_path / "nope.ico")
    apply_window_icon(root)   # 只該留 log，不該把啟動流程帶掉


def test_apply_window_icon_installs_it_on_the_window_class(root):
    # tkinter 的 iconbitmap 挑不對 ICO 的 frame（實測掛出來是放大裁切過的糊圖），
    # 所以改成自己載入正確尺寸再寫進視窗類別 —— 之後建立的每個 Toplevel 都該沿用它。
    import tkinter as tk

    import win32con
    import win32gui

    hicon = apply_window_icon(root)
    assert hicon, "沒有載入到 icon"

    probe = tk.Toplevel(root)
    probe.withdraw()
    probe.update_idletasks()
    hwnd = win32gui.GetAncestor(probe.winfo_id(), 2)   # GA_ROOT
    try:
        assert win32gui.GetClassLong(hwnd, win32con.GCL_HICON) == hicon
    finally:
        probe.destroy()
