"""給 tkinter 用的應用程式 icon 圖片（標題列小圖、泡泡）。"""
import sys
import tkinter as tk

from src.resources import png_icon_path


def load_icon(master: tk.Misc, size: int) -> tk.PhotoImage | None:
    """載入指定尺寸的 icon 圖片；失敗回 None，由呼叫端略過那個 icon。

    呼叫端必須自己留住回傳值（存成實例屬性）：Tk 只保存指標，PhotoImage 一被
    回收，畫面上的圖就跟著消失。"""
    path = png_icon_path(size)
    try:
        return tk.PhotoImage(file=str(path), master=master)
    except tk.TclError as exc:
        print(f"[ui] icon image failed: path={path} error={exc}", file=sys.stderr)
        return None
