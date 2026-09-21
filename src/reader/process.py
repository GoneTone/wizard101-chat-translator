"""遊戲程序辨識與安裝路徑偵測（路徑只用於診斷 log，wizwalker 用到的 API 不讀它）。
只用 pywin32 列舉程序，不掃描記憶體。
"""
import os

from src.log import log

PROCESS_NAME = "WizardGraphicalClient.exe"


def is_game_process_path(path: str | None) -> bool:
    """exe 路徑是否為遊戲主程式（依檔名比對，不分大小寫）。"""
    return bool(path) and path.lower().endswith(PROCESS_NAME.lower())


def process_exe_path(pid: int) -> str | None:
    """取 pid 的主模組路徑；開不了程序（權限、已結束）回 None。"""
    try:
        import win32api
        import win32process
        h = win32api.OpenProcess(0x0410, False, pid)  # QUERY_INFORMATION | VM_READ
        try:
            return win32process.GetModuleFileNameEx(h, 0)
        finally:
            win32api.CloseHandle(h)
    except Exception:
        return None


def find_game_window() -> int | None:
    """列舉可見的頂層視窗，找出屬於遊戲程序的那一個；找不到回 None。

    供標題列按鈕（而非熱鍵）呼出框選：按鈕點下去時遊戲多半不是前景視窗，
    不能像熱鍵路徑那樣直接讀 `GetForegroundWindow`，只能自己列舉找。單一視窗
    查詢失敗（權限、視窗剛消失）不該中斷整輪列舉，比照 `main.focus_running_instance`
    吞例外繼續。"""
    import win32gui
    import win32process

    found: list[int] = []

    def visit(hwnd, _):
        try:
            if win32gui.IsWindowVisible(hwnd):
                _, pid = win32process.GetWindowThreadProcessId(hwnd)
                if is_game_process_path(process_exe_path(pid)):
                    found.append(hwnd)
        except Exception:
            pass   # 單一視窗查詢失敗不該中斷整輪列舉
        return True

    try:
        win32gui.EnumWindows(visit, None)
    except Exception as exc:
        # 整輪列舉失敗要記下來：使用者從 app.log 才分得出「遊戲沒開」與「列舉本身炸了」
        log(f"[reader] window scan failed: {type(exc).__name__}: {exc}")
        return None
    return found[0] if found else None


def window_exists(hwnd: int) -> bool:
    """視窗 handle 是否仍指向一個存在的視窗（客戶端關掉後為 False）；查不到當作不存在。"""
    try:
        import win32gui
        return bool(win32gui.IsWindow(hwnd))
    except Exception:
        return False


def install_path_of(pid: int) -> str | None:
    """該遊戲程序的安裝根目錄（`...\\Bin\\` 的上一層）；開不了程序或不是遊戲回 None。"""
    path = process_exe_path(pid)
    if not is_game_process_path(path):
        return None
    return os.path.dirname(os.path.dirname(path))


def pid_alive(pid: int) -> bool:
    """PID 是否仍在執行（供清掉殘留狀態檔）；判斷不了就當活著，不誤刪。"""
    try:
        import win32process
        return pid in win32process.EnumProcesses()
    except Exception:
        return True
