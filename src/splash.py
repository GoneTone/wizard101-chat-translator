"""啟動畫面：把 PyInstaller bootloader 的 splash 包成隨時可呼叫的三個函式。

`pyi_splash` 只在打包版且有帶 splash 時存在，本模組吞掉所有例外，呼叫端不必判斷
環境或包 try；文字與字族打包時就寫死成英文，執行期換不了。

`update()` 可能搶在 `redirect_output()` 之前執行，那時 `log()` 會憑空消失，因此
先存進 `_pending_logs`，等 `drain_logs()` 才補寫。`PHASE_LOADING`／`PHASE_STARTING`
定義在這裡，因為 build 時 `tools/splash_progress.py` 要 import 同一組常數烤進 Tcl。
"""
from src.log import log

# 兩個階段訊息：build 時 tools/splash_progress.py 會把這兩個字面值烤進 Tcl 樣板
# 跟 status_text 比對，改文字只需改這裡。
PHASE_LOADING = "Loading components..."
PHASE_STARTING = "Starting..."

try:
    import pyi_splash as _splash
except ImportError:     # 開發模式，或打包時沒帶 splash
    _splash = None

_closed = False
_failed = False
# None 代表輸出已導向，直接呼叫 log()；list 代表尚未導向，先把訊息存起來等 drain_logs()。
_pending_logs: list[str] | None = []


def is_available() -> bool:
    """bootloader 是否帶了啟動畫面（打包版且 spec 有加 Splash 才為真）。"""
    return _splash is not None


def had_failure() -> bool:
    """`update()` 或 `close()` 是否曾經失敗過（例如 IPC socket 斷線）。

    給 `main()` 的狀態 log 用：`is_available()` 只說明 bootloader 有沒有帶 splash，
    不代表這次執行期間的呼叫真的成功，兩者分開才不會把「其實失敗」報成「看起來正常」。
    """
    return _failed


def _record(message: str) -> None:
    """尚未導向輸出前先存進 buffer，導向之後直接記 log（見模組頂端說明）。"""
    if _pending_logs is None:
        log(message)
    else:
        _pending_logs.append(message)


def drain_logs() -> None:
    """把導向輸出之前累積的診斷一次寫出，並切換成之後直接記 log。

    `main()` 要在 `redirect_output()` 之後、印出 `[splash] startup screen ...` 之前
    呼叫一次；呼叫之後 `_record()` 一律直接送進 `log()`。重複呼叫安全（第二次起是無操作）。
    """
    global _pending_logs
    if _pending_logs is None:
        return
    pending, _pending_logs = _pending_logs, None
    for message in pending:
        log(message)


def update(text: str) -> None:
    """更新啟動畫面的狀態文字。沒有啟動畫面、或已經關掉時什麼都不做。"""
    global _failed
    if _splash is None or _closed:
        return
    try:
        _splash.update_text(text)
    except Exception as exc:
        _failed = True
        _record(f"[splash] update failed: {type(exc).__name__}: {exc}")


def close() -> None:
    """關閉啟動畫面。沒有啟動畫面時什麼都不做；重複呼叫安全。"""
    global _closed, _failed
    if _splash is None or _closed:
        return
    _closed = True
    try:
        _splash.close()
    except Exception as exc:
        _failed = True
        _record(f"[splash] close failed: {type(exc).__name__}: {exc}")
