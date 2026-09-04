"""無邊框 Tk 視窗的 Win32 樣式調整：取根 HWND、點擊不奪焦點、工作列按鈕。
overlay 本體與底板、泡泡、翻譯輸入框共用；每一項都是錦上添花，失敗只留 log。"""
import sys
import tkinter as tk

import win32con
import win32gui


def root_hwnd(win: tk.Misc) -> int:
    """Tk 視窗對應的頂層 HWND。`winfo_id` 給的是 Tk 內層的子視窗，
    extended style、owner、前景切換都得作用在根視窗上才有效。"""
    return win32gui.GetAncestor(win.winfo_id(), 2)  # GA_ROOT


def make_non_activating(win: tk.Toplevel) -> None:
    """讓視窗攔截滑鼠事件但點擊不奪焦點、不改變疊序（WS_EX_NOACTIVATE）。
    用於 overlay 底板：點到透明背景區不會穿到遊戲，也不會把底板抬到文字層之上。
    失敗的後果是點擊底板可能改變疊序，另有 lower() 保險擋著。"""
    try:
        win.update_idletasks()
        hwnd = root_hwnd(win)
        style = win32gui.GetWindowLong(hwnd, win32con.GWL_EXSTYLE)
        style |= win32con.WS_EX_NOACTIVATE
        win32gui.SetWindowLong(hwnd, win32con.GWL_EXSTYLE, style)
    except Exception as exc:
        print(f"[ui] non-activating setup failed: {exc}", file=sys.stderr)


def enable_taskbar_button(win: tk.Toplevel, alpha: float | None = None) -> None:
    """讓無邊框視窗出現在工作列與 Alt+Tab。
    overrideredirect 視窗預設拿不到工作列按鈕，把 WS_EX_APPWINDOW 加進
    extended style 即可；需 withdraw→deiconify 一次讓樣式生效，
    之後重設 topmost（與 alpha，若有）。失敗只是少個按鈕，不影響功能。"""
    try:
        win.update_idletasks()
        hwnd = root_hwnd(win)
        style = win32gui.GetWindowLong(hwnd, win32con.GWL_EXSTYLE)
        style = (style & ~win32con.WS_EX_TOOLWINDOW) | win32con.WS_EX_APPWINDOW
        win32gui.SetWindowLong(hwnd, win32con.GWL_EXSTYLE, style)
        win.withdraw()
        win.deiconify()
        win.attributes("-topmost", True)
        if alpha is not None:
            win.attributes("-alpha", alpha)
    except Exception as exc:
        print(f"[ui] taskbar button setup failed: {exc}", file=sys.stderr)
