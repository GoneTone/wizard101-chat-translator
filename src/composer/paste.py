"""把翻譯後的文字送進遊戲：還原前景視窗後「逐字鍵入」（模擬打字）。
遊戲不支援剪貼簿貼上，故改用自動輸入。絕不模擬 Enter，由使用者自己確認後送出。"""
import ctypes
import time

import keyboard
import win32gui

FOCUS_DELAY = 0.15  # 切回遊戲視窗後、開始打字前的緩衝（秒）


def force_foreground(hwnd: int | None) -> None:
    """把前景切到 hwnd。單純 SetForegroundWindow 會被 Windows 前景鎖擋下，
    掛上目前前景視窗的執行緒（AttachThreadInput）再切才可靠；
    失敗放棄，由使用者自行點回。"""
    if not hwnd or not win32gui.IsWindow(hwnd):
        return
    try:
        user32 = ctypes.windll.user32
        fg_thread = user32.GetWindowThreadProcessId(win32gui.GetForegroundWindow(), None)
        this_thread = ctypes.windll.kernel32.GetCurrentThreadId()
        user32.AttachThreadInput(this_thread, fg_thread, True)
        try:
            user32.SetForegroundWindow(hwnd)
        finally:
            user32.AttachThreadInput(this_thread, fg_thread, False)
    except Exception:
        pass


def type_into_window(hwnd: int | None, text: str, delay: float = 0.02) -> None:
    """還原前景視窗（hwnd），逐字鍵入 text。delay 為每個字元間隔（秒），
    太快遊戲可能漏字，可調大。絕不送 Enter。"""
    if hwnd and win32gui.IsWindow(hwnd):
        force_foreground(hwnd)
        time.sleep(FOCUS_DELAY)
    keyboard.write(text, delay=delay)
