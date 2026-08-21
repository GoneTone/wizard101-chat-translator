"""把翻譯後的文字送進遊戲:還原前景視窗後「逐字鍵入」(模擬打字)。
遊戲不支援剪貼簿貼上,故改用自動輸入。絕不模擬 Enter,由使用者自己確認後送出。"""
import time

import keyboard
import win32gui

FOCUS_DELAY = 0.15  # 切回遊戲視窗後、開始打字前的緩衝(秒)


def type_into_window(hwnd: int | None, text: str, delay: float = 0.02) -> None:
    """還原前景視窗(hwnd),逐字鍵入 text。delay 為每個字元間隔(秒),
    太快遊戲可能漏字,可調大。絕不送 Enter。"""
    if hwnd and win32gui.IsWindow(hwnd):
        try:
            win32gui.SetForegroundWindow(hwnd)
        except Exception:
            pass  # Windows 前景鎖:失敗就讓使用者自己點回遊戲後再重試
        time.sleep(FOCUS_DELAY)
    keyboard.write(text, delay=delay)
