"""全域熱鍵與遊戲聊天輸入框事件的分派：決定要不要接手，再交給輸入框、框選或貼上。
按鍵攔截本身在 `src.input.hotkeys`；這裡的回呼跑在鍵盤 hook 裡，UI 動作一律排進 ui_queue。"""
import queue
import threading

import win32gui

from src.composer.paste import foreground_exe, paste_clipboard
from src.config import DEFAULT_CONFIG
from src.input.hotkeys import register_hotkey
from src.logs import log
from src.reader.process import is_game_process_path
from src.ui.input_box import InputBox
from src.ui.region.flow import RegionFlow


def region_hotkey_to_register(cfg: dict) -> str | None:
    """框選熱鍵要不要註冊：與輸入框熱鍵相同就不註冊（回 None），否則回熱鍵字串。
    舊設定檔補上預設的 region_hotkey 後可能與使用者自訂的 hotkey 撞名，兩把一起觸發
    會同時開輸入框與選取層；設定視窗儲存時會擋，啟動時要自己擋。"""
    if cfg["region_hotkey"] == cfg["hotkey"]:
        log(f"[app] region_hotkey {cfg['region_hotkey']!r} collides with hotkey; "
            f"region translation disabled until a different hotkey is saved in settings")
        return None
    return cfg["region_hotkey"]


def register_region_hotkey(cfg: dict, callback) -> object | None:
    """註冊框選熱鍵並把實際生效的鍵寫回 cfg，回傳解除用的函式；與輸入框熱鍵撞名就不註冊、
    回 None（見 region_hotkey_to_register）。退回的預設值要用 region_hotkey 自己的，否則手改壞掉
    的 config.json 會讓兩把熱鍵撞在一起。"""
    hotkey = region_hotkey_to_register(cfg)
    if hotkey is None:
        return None
    unregister, cfg["region_hotkey"] = register_hotkey(
        hotkey, callback, fallback=DEFAULT_CONFIG["region_hotkey"])
    return unregister


def foreground_game_hwnd(prefix: str) -> int | None:
    """兩把熱鍵共用的前景守衛（鍵盤 hook 裡）：前景是遊戲就回它的 HWND，
    否則記一行 log 回 None —— 在瀏覽器、聊天軟體裡按到熱鍵要當作沒按。"""
    exe = foreground_exe()
    if is_game_process_path(exe):
        return win32gui.GetForegroundWindow()
    log(f"[{prefix}] hotkey ignored: foreground is not the game window (exe={exe!r})")
    return None


def _retarget_and_show(input_box: InputBox, hwnd: int) -> None:
    """雙開切客戶端：先把翻譯輸入框改綁到新前景客戶端，再呼出。"""
    input_box.retarget(hwnd)
    input_box.show()


def on_hotkey(input_box: InputBox, ui_queue: queue.Queue) -> bool:
    """全域熱鍵的回呼（跑在鍵盤 hook 裡，回傳有沒有接手）：遊戲在前景才呼出翻譯輸入框，並改綁
    目標到這個前景客戶端（雙開時框可能還開著、綁在上一個客戶端）。輸入框已開著時（它自己
    就是前景，讀不到遊戲 hwnd）只重新對焦、不改動 target_hwnd —— 這不算「忽略」，不記
    ignored log；真的兩者皆非才記一行。不走 foreground_game_hwnd：那顆的 log 是為框選
    熱鍵寫的，套在這裡會在框自己是前景時也誤報 ignored。"""
    exe = foreground_exe()
    if is_game_process_path(exe):
        hwnd = win32gui.GetForegroundWindow()
        ui_queue.put(lambda: _retarget_and_show(input_box, hwnd))
        return True
    if input_box.is_open:
        ui_queue.put(input_box.show)
        return True
    log(f"[app] hotkey ignored: foreground is not the game window (exe={exe!r})")
    return False


def on_region_hotkey(flow: RegionFlow, ui_queue: queue.Queue) -> bool:
    """框選熱鍵的回呼（跑在鍵盤 hook 裡，回傳有沒有接手）：選取層開著就取消（此時前景是
    選取層本身）；遊戲在前景才開始框選。"""
    if flow.is_selecting:
        ui_queue.put(lambda: flow.toggle(0))
        return True
    hwnd = foreground_game_hwnd("region")
    if hwnd is None:
        return False
    ui_queue.put(lambda: flow.toggle(hwnd))
    return True


def should_intercept_paste(cfg: dict) -> bool:
    """Ctrl+V 要不要由本程式接手：設定開啟且遊戲在前景。其他視窗一律不碰，
    瀏覽器、聊天軟體裡的貼上照常。跑在鍵盤 hook 裡，每次按鍵都會問，要快。"""
    return bool(cfg["paste_hotkey"]) and is_game_process_path(foreground_exe())


class ChatInputTracker:
    """哪些遊戲視窗的聊天輸入框正開著。reader 執行緒（經 ui_queue）寫、貼上的鍵盤 hook 讀，
    雙開時每個客戶端各自開關，不能用一個布林。"""

    def __init__(self) -> None:
        self._open: set[int] = set()
        self._lock = threading.Lock()

    def opened(self, hwnd: int) -> None:
        with self._lock:
            self._open.add(hwnd)

    def closed(self, hwnd: int) -> None:
        with self._lock:
            self._open.discard(hwnd)

    def is_open(self, hwnd: int) -> bool:
        with self._lock:
            return hwnd in self._open


class GameInputEvents:
    """遊戲聊天輸入框開／關事件的分派（UI 執行緒）。只認前景客戶端：使用者正在玩 B 時，
    A 的聊天框開關不該彈出或收掉他的翻譯輸入框。"""

    def __init__(self, cfg: dict, input_box: InputBox, tracker: ChatInputTracker,
                 foreground=win32gui.GetForegroundWindow) -> None:
        self._cfg = cfg
        self._box = input_box
        self._tracker = tracker
        self._foreground = foreground
        self._shown_for: int | None = None   # 目前的錨點／自動呼出是為哪個客戶端

    def opened(self, hwnd: int, anchor) -> None:
        """聊天框開了：記下來；是前景客戶端才設錨點（熱鍵呼出也要貼齊）、改綁翻譯目標、
        依設定自動呼出。"""
        self._tracker.opened(hwnd)
        if self._foreground() != hwnd:
            log(f"[app] game chat opened in a background client (hwnd={hwnd:#x}); ignoring")
            return
        self._shown_for = hwnd
        if anchor is not None:
            self._box.set_anchor(anchor)
        self._box.retarget(hwnd)   # 框可能已開著、使用者換到這個客戶端：譯文改打回這裡
        if self._cfg["auto_show_input"]:
            self._box.show()

    def closed(self, hwnd: int) -> None:
        """聊天框關了：只有當初為它設錨點／呼出的那個客戶端才收起，打到一半的文字留到下次。"""
        self._tracker.closed(hwnd)
        if hwnd != self._shown_for:
            return
        self._shown_for = None
        self._box.clear_anchor()
        if self._cfg["auto_show_input"]:
            self._box.hide()


def on_paste_hotkey(cfg: dict, tracker: ChatInputTracker) -> threading.Thread:
    """攔到遊戲內的 Ctrl+V：記下當下的前景視窗，交給背景執行緒鍵入剪貼簿（回傳該執行緒）。
    不能在 hook 回呼裡直接打字（Windows 會判定 hook 逾時而整個拔掉）。**前景那個客戶端**
    的聊天輸入框開著時走單行模式（換行改空格），其他地方換行照打。"""
    hwnd = win32gui.GetForegroundWindow()
    worker = threading.Thread(target=paste_clipboard,
                              args=(hwnd, cfg["type_delay"], tracker.is_open(hwnd)),
                              daemon=True)
    worker.start()
    return worker
