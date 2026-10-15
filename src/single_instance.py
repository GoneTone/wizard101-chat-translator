"""單一實例：搶 mutex 確認只有一份在跑，第二份啟動時把既有那份的視窗帶到前景。"""
import win32api
import win32con
import win32event
import win32gui
import winerror

from src.composer.paste import force_foreground
from src.log import log

# 第二份實例會搶著對遊戲掛 wizwalker hook，也會同時寫同一份 config.json 與 log。
SINGLE_INSTANCE_MUTEX = "wizard101-chat-translator.single-instance"


def acquire_single_instance(name: str = SINGLE_INSTANCE_MUTEX) -> int | None:
    """搶下單一實例的 mutex；已經有一份在跑時回 None。
    呼叫端必須留住回傳的 handle 直到程序結束：mutex 隨 handle 關閉而釋放。
    name 可覆寫讓測試各用各的名稱，不會被正在執行的本尊卡住。"""
    handle = win32event.CreateMutex(None, False, name)
    if win32api.GetLastError() == winerror.ERROR_ALREADY_EXISTS:
        return None
    return handle


def focus_running_instance(title: str) -> bool:
    """把既有實例的視窗帶到前景；找不到視窗或還原失敗時回 False。
    切前景走 composer 的 force_foreground（AttachThreadInput），單純 SetForegroundWindow
    會被前景鎖擋下；就算仍被擋，系統也會改成閃工作列按鈕，使用者看得到回應。"""
    found = []

    def collect(hwnd, _):
        try:
            if win32gui.IsWindowVisible(hwnd) and win32gui.GetWindowText(hwnd) == title:
                found.append(hwnd)
        except Exception:
            pass   # 單一視窗查詢失敗不該中斷整輪列舉
        return True

    try:
        win32gui.EnumWindows(collect, None)
    except Exception as exc:
        log(f"[app] window scan failed: {exc}")
        return False
    if not found:
        log(f"[app] no window titled {title!r} to focus")
        return False
    hwnd = found[0]
    try:
        win32gui.ShowWindow(hwnd, win32con.SW_RESTORE)
    except Exception as exc:
        log(f"[app] focus existing instance failed: hwnd={hwnd:#x} error={exc}")
        return False
    force_foreground(hwnd)
    return True
