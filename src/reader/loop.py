"""單一遊戲客戶端的收訊執行緒主迴圈：讀聊天 → 推進上下文、在 overlay 佔位 → 交給翻譯池。
多個客戶端各跑一條，由 supervisor 起／收。

本迴圈不做翻譯（單則翻譯卡住不會延誤後續訊息的讀取與顯示），也不直接碰 Tk ——
所有 UI 更新都包成回呼排進 ui_queue，由主執行緒的 pump 依序執行。
"""
import itertools
import queue
import threading
import time
import traceback
from dataclasses import dataclass
from typing import TYPE_CHECKING

import win32gui

from src.i18n import t
from src.logs import log
from src.reader.chat_tabs import TabState
from src.reader.mem_reader import (
    GameAccessDenied,
    GameNotRunning,
    GameVersionMismatch,
    WizChatReader,
)
from src.reader.message_log import MessageLog
from src.reader.process import window_exists
from src.reader.status import StatusBoard, banner_for  # noqa: F401  既有測試自 loop 取用 banner_for
from src.translation.cache import TranslationCache
from src.translation.context import ClientContexts
from src.translation.pool import TranslationPool

if TYPE_CHECKING:   # 只當型別用：reader 不該在執行期依賴 ui
    from src.ui.overlay import OverlayWindow

GAME_MISSING_INTERVAL = 5.0  # 找不到遊戲時的重試間隔（秒）
POLL_INTERVAL = 0.1  # 收訊間隔（秒）：每輪只讀 hook 環形緩衝的計數器，很便宜
# 遊戲聊天輸入框的取樣間隔（秒）：只讀一個可見性旗標，可比 POLL_INTERVAL 密得多，
# 讓翻譯輸入框幾乎在聊天欄打開的當下就彈出
INPUT_POLL_INTERVAL = 0.05
# 完整分頁狀態與前景的取樣間隔（秒）：一次要讀約 45 處記憶體；切換分頁另由每輪讀目前分頁偵測
TAB_POLL_INTERVAL = 0.4


def foreground_window() -> int:
    """目前前景視窗的 hwnd，取不到回 0。"""
    try:
        return win32gui.GetForegroundWindow()
    except Exception:
        return 0


class MessageIds:
    """跨所有收訊執行緒共用的訊息序號：翻譯池以它回填 overlay 的佔位列，必須全域唯一。"""

    def __init__(self):
        self._count = itertools.count(1)
        self._lock = threading.Lock()

    def next(self) -> int:
        with self._lock:
            return next(self._count)


class _InputWatch:
    """遊戲聊天輸入框開／關的邊緣觸發：開 → 回報（hwnd，錨點）；關 → 回報 hwnd。
    on_input_open 為 None 時完全不取樣（呼叫端不關心輸入框）。"""

    def __init__(self, reader: WizChatReader, hwnd: int, slot: int, on_input_open, on_input_close):
        self._reader = reader
        self._hwnd = hwnd
        self._slot = slot
        self._on_open = on_input_open
        self._on_close = on_input_close
        self.open = False

    def poll(self) -> None:
        """取樣一次，狀態翻轉才回報。"""
        if self._on_open is None:
            return
        now_open = self._reader.input_open()
        if now_open == self.open:
            return
        self.open = now_open
        if now_open:
            anchor = self._reader.input_box_screen_rect()
            log(f"[reader] game chat input opened (slot={self._slot}, "
                f"hwnd={self._hwnd:#x}, anchor={anchor})")
            self._on_open(self._hwnd, anchor)
        else:
            self._report_closed()

    def force_closed(self) -> None:
        """遊戲斷線＝輸入框已不存在：開著就同步收回。"""
        if self.open:
            self.open = False
            self._report_closed()

    def _report_closed(self) -> None:
        log(f"[reader] game chat input closed (slot={self._slot}, hwnd={self._hwnd:#x})")
        if self._on_close is not None:
            self._on_close(self._hwnd)


@dataclass(frozen=True)
class ReaderShared:
    """所有客戶端的收訊執行緒共用的物件；狀態一律經 board 回報，由它彙整多個客戶端。"""
    cfg: dict
    overlay: "OverlayWindow"
    ui_queue: queue.Queue
    stop: threading.Event
    pool: TranslationPool
    board: StatusBoard
    msg_ids: MessageIds
    system_pool: TranslationPool | None = None
    cache: TranslationCache | None = None


def reader_loop(shared: ReaderShared, hwnd: int, slot: int,
                on_input_open=None, on_input_close=None,
                message_log: MessageLog | None = None,
                contexts: ClientContexts | None = None) -> None:
    """一個遊戲客戶端（hwnd）的收訊執行緒進入點；stop 被設定或該視窗消失後解除 hook 再返回。
    slot＝這個客戶端的顯示編號。
    on_input_open(hwnd, anchor)／on_input_close(hwnd)＝遊戲聊天輸入框開關的邊緣觸發，
    不受 auto_show_input 影響（要不要自動呼出由呼叫端決定，錨點則熱鍵呼出也用得到）；
    anchor＝遊戲輸入框的螢幕矩形 (x, y, w, h)，讀不到為 None。
    contexts 預設每條執行緒自建一份；呼叫端要拿去給發話用時可傳入。"""
    _ClientLoop(shared, hwnd, slot, on_input_open, on_input_close, message_log,
                contexts if contexts is not None else ClientContexts()).run()


class _ClientLoop:
    """reader_loop 的本體：一個客戶端的讀取、分頁同步與訊息分派。"""

    def __init__(self, shared: ReaderShared, hwnd: int, slot: int, on_input_open,
                 on_input_close, message_log: MessageLog | None, contexts: ClientContexts):
        self._s = shared
        self._hwnd = hwnd
        self._slot = slot
        self._contexts = contexts
        # 全用關鍵字：測試以 `lambda **kw` 替換 WizChatReader
        self._reader = WizChatReader(hwnd=hwnd, message_log=message_log, slot=slot)
        self._inputs = _InputWatch(self._reader, hwnd, slot, on_input_open, on_input_close)
        self._game_issue: str | None = None  # 遊戲端問題的橫幅文案 key（None＝遊戲正常）
        self._reported_tabs: TabState | None = None
        self._tabs_polled_at = float("-inf")
        self._sampled_tab: int | None = None  # 上次完整取樣時的目前分頁

    def run(self) -> None:
        stop = self._s.stop
        try:
            while not stop.is_set():
                if not window_exists(self._hwnd):
                    log(f"[reader] game window gone (slot={self._slot}, hwnd={self._hwnd:#x}); "
                        f"stopping this reader")
                    self._inputs.force_closed()
                    self._report_tabs(None)
                    break
                events = self._read()
                if events is None:
                    continue
                self._sync_tabs()
                for event in events:
                    self._dispatch(event)
                self._report_activity()
                self._inputs.poll()
                self._wait_watching_input(POLL_INTERVAL)
        finally:
            # 迴圈本體逃出例外也要收回編號、解除 hook，不留殭屍編號與未 unhook 的客戶端
            self._s.board.drop(self._slot)
            self._reader.close()

    def _listening_status(self) -> str:
        return "listening" if self._reader.anchored else "locating"

    def _read(self) -> list | None:
        """讀這輪的新聊天事件；遊戲不在或讀取出錯時已等過該等的間隔，回 None 讓迴圈重來。"""
        s, slot = self._s, self._slot
        # 每輪重讀設定：設定視窗可能在執行中切換系統訊息開關
        self._reader.emit_system = s.cfg.get("translate_system_messages", False)
        s.board.report(slot, self._listening_status(), self._game_issue)
        try:
            events = self._reader.read_new()
        except GameNotRunning as exc:
            if isinstance(exc, GameAccessDenied):
                status, issue = "access_denied", "notice.access_denied"
            elif isinstance(exc, GameVersionMismatch):
                status, issue = "version_mismatch", "notice.version_mismatch"
            else:
                status, issue = "waiting_game", "notice.game_missing"
            if issue != self._game_issue:  # 只在原因改變時記錄，否則每輪重試都灌一行
                log(f"[reader] game not ready (slot={slot}, hwnd={self._hwnd:#x}): {exc}")
            self._game_issue = issue
            s.board.report(slot, status, self._game_issue)
            self._inputs.force_closed()
            self._report_tabs(None)
            s.stop.wait(GAME_MISSING_INTERVAL)
            return None
        except Exception as exc:  # 收訊偶發錯誤：略過該輪，不讓執行緒死掉
            log(f"[reader] poll skipped (slot={slot}): {type(exc).__name__}: {exc}\n"
                f"{traceback.format_exc()}")
            s.stop.wait(POLL_INTERVAL)
            return None
        if self._game_issue:
            self._game_issue = None
            log(f"[reader] game back, resuming (slot={slot})")
        return events

    def _sync_tabs(self) -> None:
        # 目前分頁只要一次讀取、每輪都看：遊戲裡切分頁不必等 TAB_POLL_INTERVAL 才同步
        current = self._reader.current_tab()
        if (time.monotonic() - self._tabs_polled_at < TAB_POLL_INTERVAL
                and (current is None or current == self._sampled_tab)):
            return
        self._tabs_polled_at = time.monotonic()
        state = self._reader.tab_state()
        # 完整取樣失敗時記下這輪讀到的分頁，免得每輪都重試完整取樣
        self._sampled_tab = state.current if state else current
        self._report_tabs(state)
        # 每次取樣都送（overlay 對同一客戶端是 no-op）：只送邊緣時，兩條執行緒錯開會漏掉快速 A→B→A
        if foreground_window() == self._hwnd:
            overlay, slot = self._s.overlay, self._slot
            self._s.ui_queue.put(lambda: overlay.set_foreground_slot(slot))

    def _report_tabs(self, state: TabState | None) -> None:
        if state == self._reported_tabs:
            return
        self._reported_tabs = state
        # 遊戲不在時分頁不明：發話上下文退回主分頁
        self._contexts.current_tab = state.current if state is not None else 0
        overlay, slot = self._s.overlay, self._slot
        self._s.ui_queue.put(lambda: overlay.set_tabs(slot, state))

    def _dispatch(self, event) -> None:
        """一則新訊息：在 overlay 佔位並交給對應的翻譯池（系統訊息快取命中則直接顯示完成態）。"""
        s, slot, line = self._s, self._slot, event.line
        overlay = s.overlay
        msg_id = s.msg_ids.next()
        if line.system:
            # 系統訊息不進上下文；快取命中就直接以完成態顯示，不佔位也不進 pool
            cached = s.cache.get(line.text) if s.cache is not None else None
            if cached is not None:
                s.ui_queue.put(lambda o=line.text, tr=cached, c=line.color, tb=event.tabs:
                               overlay.add_message(o, tr, color=c, slot=slot, tabs=tb))
                return
            s.ui_queue.put(lambda o=line.text, c=line.color, m=msg_id, tb=event.tabs:
                           overlay.add_message(o, t("notice.pending"), msg_id=m,
                                               pending=True, color=c, slot=slot, tabs=tb))
            if s.system_pool is not None:
                s.system_pool.submit(line.text, [], msg_id)
            return
        context = self._contexts.for_tab(event.tabs[0])
        ctx = context.snapshot()   # 該行之前的行；提交後即固定，重試不漂移
        context.push(line.text)
        s.ui_queue.put(lambda o=line.text, c=line.color, m=msg_id, tb=event.tabs:
                       overlay.add_message(o, t("notice.pending"), msg_id=m,
                                           pending=True, color=c, slot=slot, tabs=tb))
        s.pool.submit(line.text, ctx, msg_id)

    def _report_activity(self) -> None:
        s = self._s
        if s.pool.in_flight or (s.system_pool is not None and s.system_pool.in_flight):
            s.board.report(self._slot, "translating", self._game_issue)
        else:
            s.board.report(self._slot, self._listening_status(), self._game_issue)

    def _wait_watching_input(self, seconds: float) -> None:
        """等待下一輪讀取，期間以 INPUT_POLL_INTERVAL 持續取樣輸入框狀態。
        input_open() 只讀一個已快取節點的旗標（實測 <0.1ms），可比讀聊天取樣得更密。
        不另開執行緒：WizChatReader 內部跑自己的 asyncio loop，
        跨執行緒併發呼叫會踩到彼此。"""
        stop = self._s.stop
        deadline = time.monotonic() + seconds
        while not stop.is_set():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return
            stop.wait(min(INPUT_POLL_INTERVAL, remaining))
            self._inputs.poll()
