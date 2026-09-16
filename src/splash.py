"""啟動畫面：把 PyInstaller bootloader 的 splash 包成隨時可呼叫的三個函式。

`pyi_splash` 只存在於打包版、且只在 bootloader 帶了 splash 時可用 —— 開發模式
（`uv run run.py`）會 ImportError，沒帶 splash 會 RuntimeError，IPC socket 斷了會
ConnectionError。本模組把這些全部吞掉，呼叫端不必判斷環境、也不必包 try。

文字一律英文：解壓期間顯示的那句是打包時寫死的（那時 Python 還沒啟動，讀不到使用者
的介面語言），字族同樣是打包時定死、執行期換不了，中途換語言只會有跳躍感。

本模組刻意不在 import 時直接記 log：`run.py` 會在 `redirect_output()` 之前就呼叫
`update()`，那時 windowed exe 的 stderr 還是 None，`log()` 會直接 return、寫出去的行
憑空消失 —— 正好是打包版最需要診斷的那一刻。失敗訊息因此先進 `_pending_logs` 這個
buffer，等 `main()` 在 `redirect_output()` 之後呼叫 `drain_logs()` 才一次補寫、並切換成
之後直接呼叫 `log()`。`_failed` 讓呼叫端（`main()` 的 `[splash] startup screen ...` 那行）
能判斷「看起來可用」與「其實已經失敗過」的差別，不必自己重複解析訊息內容。

`PHASE_LOADING`／`PHASE_STARTING` 定義在這裡（而不是直接寫死在 `run.py`／
`src/main.py` 呼叫處），是因為 `tools/splash_progress.py` 在 build 時要把同一組字串
烤進 Tcl 腳本，用來判斷 bootloader 回報的 `status_text` 是不是這兩個階段訊息、藉此把
進度條從解壓推進到「Python 正在跑」的後段。三處（這裡、`run.py`、`src/main.py`）都
從這裡 import 常數，改字面文字只要改這一處，Tcl 那邊的比對不會跟著兜不起來。
build.spec 在 import `tools.splash_progress`之前已經 `sys.path.insert(0, SPECPATH)`，
build 時 `import src.splash` 讀得到這個模組；`pyi_splash` 那段 import 包在
try/except ImportError 裡，build 環境沒有 `pyi_splash`，會安靜地走 except 分支，不會
讓 build 掛掉。
"""
from src.log import log

# 兩個階段訊息：building 時 tools/splash_progress.py 會把這兩個字面值原封不動烤進
# Tcl，跟 bootloader 回報的 status_text 做完全比對，藉此判斷「解壓已經結束，Python
# 正在跑到哪個階段」——改這裡的文字，Tcl 那邊自動跟著換，不必去 build 腳本裡改一份
# 複製的字串。
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
