"""給 tkinter 用的圖片：應用程式 icon（標題列小圖、泡泡）與服務商標誌。"""
import tkinter as tk

from src.log import log
from src.resources import png_icon_path, provider_icon_path


def _photo(master: tk.Misc, path, detail: str) -> tk.PhotoImage | None:
    """讀一張 PNG 成 PhotoImage；失敗回 None，由呼叫端決定少畫那張圖。

    呼叫端必須自己留住回傳值（存成實例屬性）：Tk 只保存指標，PhotoImage 一被
    回收，畫面上的圖就跟著消失。"""
    try:
        return tk.PhotoImage(file=str(path), master=master)
    except tk.TclError as exc:
        log(f"[ui] icon image failed: {detail} path={path} error={exc}")
        return None


def load_icon(master: tk.Misc, size: int) -> tk.PhotoImage | None:
    """載入指定尺寸的應用程式 icon 圖片。"""
    return _photo(master, png_icon_path(size), f"size={size}")


def load_provider_icon(master: tk.Misc, key: str) -> tk.PhotoImage | None:
    """載入某家服務商的標誌。"""
    return _photo(master, provider_icon_path(key), f"provider={key}")


class ProviderIcons:
    """服務商標誌的快取，兼作 PhotoImage 的持有者 —— 沒人留住它們，圖會消失。
    每家只載一次，卡片重畫時不重載。"""

    def __init__(self, master: tk.Misc):
        self._master = master
        self._cache: dict[str, tk.PhotoImage | None] = {}

    def get(self, provider: str) -> tk.PhotoImage | None:
        if provider not in self._cache:
            self._cache[provider] = load_provider_icon(self._master, provider)
        return self._cache[provider]
