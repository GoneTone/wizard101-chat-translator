"""把文字送進遊戲：「逐字鍵入」（模擬打字）。遊戲不支援剪貼簿貼上，故改用自動輸入。
兩條路徑共用同一個鍵入迴圈但規矩不同：譯文（type_into_window）絕不模擬 Enter，由使用者
自己確認後送出；攔截到的 Ctrl+V（paste_clipboard）行為等同原生貼上，剪貼簿內容原樣送。"""
import ctypes
import re
import threading
import time

import keyboard
import win32clipboard
import win32con
import win32gui
import win32process

from src.hotkeys import HotkeyInterceptor
from src.log import log
from src.reader.process import process_exe_path

FOCUS_DELAY = 0.15  # 切回遊戲視窗後、開始打字前的緩衝（秒）
PASTE_KEY = "v"     # 與 Ctrl 合按時被攔截的鍵
MODIFIER_RELEASE_TIMEOUT = 3.0  # 貼上前等使用者放開 Ctrl／V 的上限（秒）
KEY_RELEASE_POLL = 0.01


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
    except Exception as exc:
        log(f"[composer] force_foreground failed (hwnd={hwnd:#x}): "
            f"{type(exc).__name__}: {exc}")


_typing = threading.Lock()  # 譯文與貼上兩條路共用：鍵入進行中就持有，也讓兩者不交錯


def typing_in_progress() -> bool:
    """目前是否正在逐字鍵入（譯文或貼上）。"""
    return _typing.locked()


def _type_chars(hwnd: int | None, text: str, delay: float) -> None:
    """逐字鍵入 text，delay 為每字間隔（秒）。hwnd 存在時每個字送出前都確認它仍是前景：
    打到一半切窗（或搶焦點失敗）就停，剩下的字不會打進別的視窗；hwnd 為 None 則打到底。"""
    guarded = bool(hwnd and win32gui.IsWindow(hwnd))
    with _typing:
        for index, char in enumerate(text):
            if guarded and win32gui.GetForegroundWindow() != hwnd:
                log(f"[composer] typing aborted: target lost foreground after "
                    f"{index}/{len(text)} chars (hwnd={hwnd:#x})")
                return
            # 不還原修飾鍵：keyboard.write 打完會把使用者按著的 Ctrl 以「重播」注入按回去，
            # 套件自己的狀態表記不到，之後每個字都帶著 Ctrl 送出（Enter 變 Ctrl+Enter）
            keyboard.write(char, delay=delay, restore_state_after=False)


def type_into_window(hwnd: int | None, text: str, delay: float = 0.02) -> None:
    """把譯文送進遊戲：還原前景視窗（hwnd）後逐字鍵入 text；遊戲漏字就把 delay 調大。"""
    # 絕不送 Enter 的最後防線：keyboard.write 會把換行打成 Enter，模型偶發的多行
    # 輸出一律壓成空格
    text = re.sub(r"[\r\n]+", " ", text).strip()
    target_alive = bool(hwnd and win32gui.IsWindow(hwnd))
    log(f"[composer] typing chars={len(text)} delay={delay} "
        f"hwnd={hwnd or 0:#x} target_alive={target_alive}")
    if target_alive:
        force_foreground(hwnd)
        time.sleep(FOCUS_DELAY)
    _type_chars(hwnd, text, delay)


class PasteInterceptor(HotkeyInterceptor):
    """Ctrl+V 的攔截器：`should_intercept()` 成立（遊戲在前景、設定開啟）才吞掉並呼叫
    `on_paste`，否則原樣放行、不留 log —— 其他視窗的 Ctrl+V 很頻繁。`on_paste` 跑在
    低階 hook 裡，只能把工作丟給別的執行緒。"""

    def __init__(self, should_intercept, on_paste) -> None:
        def trigger() -> bool:
            if not should_intercept():
                return False
            on_paste()
            return True
        super().__init__(f"ctrl+{PASTE_KEY}", trigger)

    def _should_swallow(self, event) -> bool:
        # 鍵入進行中每個字都會先把使用者按著的 Ctrl 放開，此時再按 Ctrl+V 查不到 Ctrl；
        # 一律吞掉，否則 v 會插進打到一半的訊息
        return typing_in_progress() or super()._should_swallow(event)


def clipboard_text() -> str | None:
    """剪貼簿目前的文字；不是文字（圖片、空的）或開不了剪貼簿回 None。"""
    try:
        win32clipboard.OpenClipboard()
    except Exception as exc:
        log(f"[composer] clipboard open failed: {type(exc).__name__}: {exc}")
        return None
    try:
        return win32clipboard.GetClipboardData(win32con.CF_UNICODETEXT)
    except Exception as exc:
        log(f"[composer] clipboard has no text: {type(exc).__name__}: {exc}")
        return None
    finally:
        win32clipboard.CloseClipboard()


_paste_in_progress = threading.Lock()


def _wait_for_paste_keys_released() -> None:
    """等使用者把 Ctrl 與 V 都放開再開始打字：Ctrl 還按著時送出的 Enter 是 Ctrl+Enter，
    聊天框不會送出。一直按著不放就等到上限照打，不讓貼上永遠卡住。"""
    deadline = time.monotonic() + MODIFIER_RELEASE_TIMEOUT
    while keyboard.is_pressed("ctrl") or keyboard.is_pressed(PASTE_KEY):
        if time.monotonic() >= deadline:
            log(f"[composer] ctrl/{PASTE_KEY} still held after {MODIFIER_RELEASE_TIMEOUT}s; "
                f"typing anyway")
            return
        time.sleep(KEY_RELEASE_POLL)


def paste_clipboard(hwnd: int | None, delay: float, single_line: bool) -> bool:
    """攔到 Ctrl+V：把剪貼簿文字鍵入 hwnd（遊戲已在前景，不搶焦點），回傳有沒有真的開始打。
    行為要跟原生貼上一樣 —— 不只用在聊天框，空白照送、內容不 strip；Windows 的 CRLF
    折成一個換行。single_line（遊戲聊天輸入框開著）時每個換行改成一個空格：那裡的 Enter
    會把打到一半的內容送出；其他地方換行照打成 Enter。空剪貼簿不打；上一次還在打的
    時候再呼叫一律忽略，免得兩串字交錯。"""
    if not _paste_in_progress.acquire(blocking=False):
        log("[composer] paste ignored: previous paste still typing")
        return False
    try:
        text = clipboard_text()
        if not text:
            log("[composer] paste skipped: clipboard is empty or not text")
            return False
        text = re.sub(r"\r\n?", "\n", text)
        if single_line:
            text = text.replace("\n", " ")
        log(f"[composer] pasting clipboard chars={len(text)} delay={delay} "
            f"single_line={single_line} hwnd={hwnd or 0:#x}")
        _wait_for_paste_keys_released()
        _type_chars(hwnd, text, delay)
        return True
    finally:
        _paste_in_progress.release()
