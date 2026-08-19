"""剪貼簿寫入與貼上模擬。只貼上,絕不模擬 Enter 送出。"""
import time

import keyboard
import win32clipboard
import win32con
import win32gui


def set_clipboard(text: str) -> None:
    max_retries = 5
    for attempt in range(max_retries):
        try:
            win32clipboard.OpenClipboard()
            break
        except Exception:
            if attempt < max_retries - 1:
                time.sleep(0.05)
            else:
                raise
    try:
        win32clipboard.EmptyClipboard()
        win32clipboard.SetClipboardData(win32con.CF_UNICODETEXT, text)
    finally:
        win32clipboard.CloseClipboard()


def get_clipboard() -> str:
    max_retries = 5
    for attempt in range(max_retries):
        try:
            win32clipboard.OpenClipboard()
            break
        except Exception:
            if attempt < max_retries - 1:
                time.sleep(0.05)
            else:
                raise
    try:
        return win32clipboard.GetClipboardData(win32con.CF_UNICODETEXT)
    finally:
        win32clipboard.CloseClipboard()


def paste_into_window(hwnd: int | None) -> None:
    """還原前景視窗並送 Ctrl+V。剪貼簿須已由呼叫端填好。"""
    if hwnd and win32gui.IsWindow(hwnd):
        try:
            win32gui.SetForegroundWindow(hwnd)
        except Exception:
            pass  # Windows 前景鎖:失敗就讓使用者自己點回遊戲手動 Ctrl+V
        time.sleep(0.15)
    keyboard.send("ctrl+v")
