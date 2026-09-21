"""收訊端的 supervisor：定期列舉遊戲視窗，每個視窗配一個編號並起一條 reader_loop 執行緒，
視窗消失就釋出編號；第一次同時有兩個客戶端時通知 overlay 進入多客戶端模式。

reader_loop 本身怎麼跑由 `spawn(hwnd, slot)` 決定（main 提供、閉包帶著共用物件），
這裡只管生命週期。
"""
import threading
import time
import traceback
from collections.abc import Callable, Iterable

from src.log import log
from src.reader.status import StatusBoard

SCAN_INTERVAL = 5.0   # 列舉遊戲視窗的間隔（秒）；與舊版找不到遊戲的重試間隔相同
JOIN_TIMEOUT = 8.0    # 關閉時等所有 reader unhook 的總預算（秒）；main 以 +2 秒 join supervisor 本身


def allocate_slot(used: Iterable[int]) -> int:
    """最小的未使用正整數：客戶端關掉後編號釋出，下一個新視窗補進來。"""
    taken = set(used)
    slot = 1
    while slot in taken:
        slot += 1
    return slot


def game_windows() -> list[int]:
    """目前所有遊戲客戶端的視窗 handle（wizwalker 以視窗類別名辨識）。"""
    from wizwalker.utils import get_all_wizard_handles
    return list(get_all_wizard_handles())


def supervise(stop: threading.Event, spawn: Callable[[int, int], threading.Thread],
              enumerate_windows: Callable[[], list[int]], on_multi_client: Callable[[], None],
              board: StatusBoard, interval: float = SCAN_INTERVAL) -> None:
    """supervisor 執行緒的進入點；stop 被設定後等所有 reader 執行緒結束再返回。"""
    active: dict[int, tuple[int, threading.Thread]] = {}   # hwnd → (slot, thread)
    multi_client = False
    while not stop.is_set():
        try:
            for hwnd, (slot, thread) in list(active.items()):
                if not thread.is_alive():
                    del active[hwnd]
                    log(f"[reader] reader thread exited hwnd={hwnd:#x} slot={slot} freed")
            windows = enumerate_windows()
            for hwnd in windows:
                if hwnd in active:
                    continue
                slot = allocate_slot(s for s, _ in active.values())
                log(f"[reader] client window appeared hwnd={hwnd:#x} slot={slot}")
                active[hwnd] = (slot, spawn(hwnd, slot))
            # 看本輪列舉到的視窗數，不是 active 的執行緒數：殭屍執行緒（視窗已消失、
            # 執行緒還沒退出）不算，否則「關遊戲、馬上重開」會誤觸發多客戶端模式且不會關
            if not multi_client and len(windows) >= 2:
                log(f"[reader] multi-client mode on (slots={sorted(s for s, _ in active.values())})")
                on_multi_client()   # 先呼叫再鎖存：raise 時下一輪還會重試，不會永久跳過通知
                multi_client = True   # 一次性：之後客戶端減回一個也不關（見 spec）
            board.refresh()
        except Exception as exc:   # 列舉或起執行緒失敗不可讓整個收訊端死掉
            log(f"[reader] supervisor scan failed: {type(exc).__name__}: {exc}\n"
                f"{traceback.format_exc()}")
        stop.wait(interval)
    deadline = time.monotonic() + JOIN_TIMEOUT   # 全體 reader 共用一個預算，不是每條各 JOIN_TIMEOUT
    for hwnd, (slot, thread) in active.items():
        thread.join(timeout=max(0.0, deadline - time.monotonic()))
        if thread.is_alive():
            log(f"[reader] reader thread still alive after shutdown budget "
                f"(slot={slot}, hwnd={hwnd:#x}); hooks may be left for next-launch repair")
