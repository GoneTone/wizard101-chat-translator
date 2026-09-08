"""把翻譯後的文字送進遊戲：還原前景視窗後「逐字鍵入」（模擬打字）。
遊戲不支援剪貼簿貼上，故改用自動輸入。絕不模擬 Enter，由使用者自己確認後送出。"""
import ctypes
import re
import time

import keyboard
import win32gui
import win32process

from src.reader.mem_reader import process_exe_path

FOCUS_DELAY = 0.15  # 切回遊戲視窗後、開始打字前的緩衝（秒）


def foreground_exe() -> str | None:
    """目前前景視窗所屬程序的 exe 路徑；沒有前景視窗或查不到回 None。
    搭配 is_game_process_path 判斷使用者是否正在遊戲畫面。"""
    try:
        hwnd = win32gui.GetForegroundWindow()
        if not hwnd:
            return None
        _, pid = win32process.GetWindowThreadProcessId(hwnd)
    except Exception:
        return None
    return process_exe_path(pid)


def force_foreground(hwnd: int | None) -> None:
    """把前景切到 hwnd。單純 SetForegroundWindow 會被前景鎖擋下，先 AttachThreadInput
    掛上目前前景視窗的執行緒再切才可靠；失敗放棄，由使用者自行點回。"""
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
    """還原前景視窗（hwnd），逐字鍵入 text；delay 為每字間隔（秒），遊戲漏字就調大。"""
    # 絕不送 Enter 的最後防線：keyboard.write 會把換行打成 Enter，模型偶發的多行
    # 輸出一律壓成空格
    text = re.sub(r"[\r\n]+", " ", text).strip()
    if hwnd and win32gui.IsWindow(hwnd):
        force_foreground(hwnd)
        time.sleep(FOCUS_DELAY)
    keyboard.write(text, delay=delay)
