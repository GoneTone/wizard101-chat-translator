"""系統剪貼簿寫入（疊加視窗與框選卡片共用）。"""
import tkinter as tk


def copy_to_clipboard(win: tk.Misc, text: str) -> None:
    """把 text 寫進系統剪貼簿。Windows 下要 `update()` flush 過，內容才真的落進系統剪貼簿；
    這會連帶清空 after 佇列，呼叫端排程中的回呼可能在這裡重入執行，要用的值得先存成區域變數。"""
    win.clipboard_clear()
    win.clipboard_append(text)
    win.update()
