"""唯讀打包資源（語言檔、icon）的路徑解析。

這類資源會被 PyInstaller 收進 exe，執行期解壓到 `_MEIPASS`——和 config.json、
log 所在的「exe 旁目錄」（`config.app_dir`）不是同一處，兩者不可混用：前者唯讀且
每次啟動換位置，後者可寫且跟著 exe 走。
"""
import sys
from pathlib import Path


def bundle_dir(name: str) -> Path:
    """打包資源子目錄（開發時為 `src/<name>`，frozen 時為 `_MEIPASS/<name>`）。"""
    if getattr(sys, "frozen", False):
        return Path(sys._MEIPASS) / name
    return Path(__file__).resolve().parent / name


def icon_path() -> Path:
    """應用程式 icon：tkinter 視窗與打包後的 exe 共用同一份。"""
    return bundle_dir("assets") / "icon.ico"


def png_icon_path(size: int) -> Path:
    """給 tkinter `PhotoImage` 用的 icon（它讀不了 ICO）。

    只提供 overlay 實際會用到的原生尺寸——`PhotoImage` 只能整數倍縮放，畫質很差，
    要新尺寸請從 icon.ico 另外匯出一張，不要在執行期縮。"""
    return bundle_dir("assets") / f"icon_{size}.png"
