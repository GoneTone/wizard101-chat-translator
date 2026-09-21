"""唯讀打包資源（語言檔、icon）的路徑解析。

PyInstaller 把這類資源收進 exe，執行期解壓到 `_MEIPASS` —— 唯讀且每次啟動換位置，
與 config.json、log 所在的可寫「exe 旁目錄」（`config.app_dir`）不可混用。
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


def provider_icon_path(key: str) -> Path:
    """服務商標誌。只有原生 32px 一種 —— 卡片與服務商選單都用這個尺寸，不必縮。"""
    return bundle_dir("assets") / f"provider_{key}_32.png"


def png_icon_path(size: int) -> Path:
    """給 tkinter `PhotoImage` 用的 icon（它讀不了 ICO）。只提供原生尺寸：`PhotoImage`
    只能整數倍縮放且畫質很差，要新尺寸從 icon.ico 另外匯出，不在執行期縮。"""
    return bundle_dir("assets") / f"icon_{size}.png"
