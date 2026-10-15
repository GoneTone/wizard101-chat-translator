"""給 tkinter 用的圖片：應用程式 icon（標題列小圖、泡泡）與服務商標誌。"""
import ctypes
import tkinter as tk

import win32api
import win32con
import win32gui

from src.logs import log
from src.runtime.resources import icon_path, png_icon_path, provider_icon_path
from src.ui.winstyle import root_hwnd


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


def apply_window_icon(root: tk.Tk) -> int | None:
    """把應用程式 icon 裝到視窗類別上，回傳 HICON；失敗回 None（icon 只是裝飾，不擋啟動）。

    不用 tkinter 的 `iconbitmap`：它在 Windows 上挑錯 ICO frame，實測 32x32 是放大裁切
    過的糊圖。改用 LoadImage 指定尺寸（會挑最接近的原生 frame）寫進視窗類別，之後每個
    Toplevel 都自動沿用。類別得透過 TkTopLevel 視窗才設得到 —— withdraw 的 root 是
    TkChild —— 故開一個隱藏 Toplevel 當跳板。小圖示（GCL_HICONSM）寫入回報成功卻讀不
    回來，但 Windows 11 工作列看的是 exe 資源的圖示，不受影響（尺寸要齊全，見
    src/assets/icon.ico）。"""
    path = icon_path()
    probe = None
    try:
        probe = tk.Toplevel(root)
        probe.withdraw()
        probe.update_idletasks()
        hwnd = root_hwnd(probe)
        size = win32api.GetSystemMetrics(win32con.SM_CXICON)
        hicon = win32gui.LoadImage(0, str(path), win32con.IMAGE_ICON, size, size,
                                   win32con.LR_LOADFROMFILE)
        user32 = ctypes.windll.user32
        user32.SetClassLongPtrW.argtypes = [ctypes.c_void_p, ctypes.c_int,
                                            ctypes.c_void_p]
        user32.SetClassLongPtrW.restype = ctypes.c_void_p
        user32.SetClassLongPtrW(hwnd, win32con.GCL_HICON, hicon)
        log(f"[ui] window icon applied: path={path} size={size} hicon={hicon:#x}")
        return hicon
    except Exception as exc:
        log(f"[ui] window icon failed: path={path} error={exc}")
        return None
    finally:
        if probe is not None:
            probe.destroy()
