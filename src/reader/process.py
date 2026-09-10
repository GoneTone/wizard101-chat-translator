"""遊戲程序辨識與安裝路徑偵測（wizwalker 需要安裝路徑讀 Data/GameData 的 WAD）。
只用 pywin32 列舉程序，不掃描記憶體。
"""
import os

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


def detect_install_path() -> str | None:
    """從執行中的 WizardGraphicalClient.exe 推導遊戲根目錄（...\\Bin\\ 的上一層）。
    找不到回傳 None。用 pywin32 列舉程序，不掃描記憶體。"""
    try:
        import win32process
    except ImportError:
        return None
    for pid in win32process.EnumProcesses():
        path = process_exe_path(pid)
        if is_game_process_path(path):
            return os.path.dirname(os.path.dirname(path))
    return None


def pid_alive(pid: int) -> bool:
    """PID 是否仍在執行（供清掉殘留狀態檔）；判斷不了就當活著，不誤刪。"""
    try:
        import win32process
        return pid in win32process.EnumProcesses()
    except Exception:
        return True
