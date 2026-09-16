"""啟動畫面：把 PyInstaller bootloader 的 splash 包成隨時可呼叫的三個函式。

`pyi_splash` 只存在於打包版、且只在 bootloader 帶了 splash 時可用 —— 開發模式
（`uv run run.py`）會 ImportError，沒帶 splash 會 RuntimeError，IPC socket 斷了會
ConnectionError。本模組把這些全部吞掉，呼叫端不必判斷環境、也不必包 try。

文字一律英文：解壓期間顯示的那句是打包時寫死的（那時 Python 還沒啟動，讀不到使用者
的介面語言），字族同樣是打包時定死、執行期換不了，中途換語言只會有跳躍感。

本模組刻意不在 import 時記 log：`run.py` 會在 `redirect_output()` 之前就呼叫 `update()`，
那時 windowed exe 的 stderr 還是 None，寫出去的行會直接消失。狀態改由 `main()` 在輸出
導向之後記一次（見 `is_available`）。
"""
from src.log import log

try:
    import pyi_splash as _splash
except ImportError:     # 開發模式，或打包時沒帶 splash
    _splash = None

_closed = False


def is_available() -> bool:
    """bootloader 是否帶了啟動畫面（打包版且 spec 有加 Splash 才為真）。"""
    return _splash is not None


def update(text: str) -> None:
    """更新啟動畫面的狀態文字。沒有啟動畫面、或已經關掉時什麼都不做。"""
    if _splash is None or _closed:
        return
    try:
        _splash.update_text(text)
    except Exception as exc:
        log(f"[splash] update failed: {type(exc).__name__}: {exc}")


def close() -> None:
    """關閉啟動畫面。沒有啟動畫面時什麼都不做；重複呼叫安全。"""
    global _closed
    if _splash is None or _closed:
        return
    _closed = True
    try:
        _splash.close()
    except Exception as exc:
        log(f"[splash] close failed: {type(exc).__name__}: {exc}")
