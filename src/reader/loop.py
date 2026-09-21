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
from typing import TYPE_CHECKING

from src.i18n import t
from src.log import log
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
from src.translation.context import ChatContext
from src.translation.pool import TranslationPool

if TYPE_CHECKING:   # 只當型別用：reader 不該在執行期依賴 ui
    from src.ui.overlay import OverlayWindow

GAME_MISSING_INTERVAL = 5.0  # 找不到遊戲時的重試間隔（秒）
# 遊戲聊天輸入框的取樣間隔（秒）：只讀一個可見性旗標，可比 poll_interval 密得多，
# 讓翻譯輸入框幾乎在聊天欄打開的當下就彈出
INPUT_POLL_INTERVAL = 0.05


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

    def __init__(self, reader: WizChatReader, hwnd: int, on_input_open, on_input_close):
        self._reader = reader
        self._hwnd = hwnd
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
            log(f"[reader] game chat input opened (hwnd={self._hwnd:#x}, anchor={anchor})")
            self._on_open(self._hwnd, anchor)
        else:
            self._report_closed()

    def force_closed(self) -> None:
        """遊戲斷線＝輸入框已不存在：開著就同步收回。"""
        if self.open:
            self.open = False
            self._report_closed()

    def _report_closed(self) -> None:
        log(f"[reader] game chat input closed (hwnd={self._hwnd:#x})")
        if self._on_close is not None:
            self._on_close(self._hwnd)


def reader_loop(cfg: dict, hwnd: int, slot: int, overlay: "OverlayWindow",
                ui_queue: queue.Queue, stop: threading.Event, pool: TranslationPool,
                board: StatusBoard, msg_ids: MessageIds,
                on_input_open=None, on_input_close=None,
                message_log: MessageLog | None = None,
                system_pool: TranslationPool | None = None,
                cache: TranslationCache | None = None,
                context: ChatContext | None = None) -> None:
    """一個遊戲客戶端（hwnd）的收訊執行緒進入點；stop 被設定或該視窗消失後解除 hook 再返回。
    slot＝這個客戶端的顯示編號；狀態一律經 board 回報，由它彙整多個客戶端。
    on_input_open(hwnd, anchor)／on_input_close(hwnd)＝遊戲聊天輸入框開關的邊緣觸發，
    不受 auto_show_input 影響（要不要自動呼出由呼叫端決定，錨點則熱鍵呼出也用得到）；
    anchor＝遊戲輸入框的螢幕矩形 (x, y, w, h)，讀不到為 None。
    context 預設每條執行緒自建一份；呼叫端要拿去給發話用時可傳入。"""
    # 全用關鍵字：測試以 `lambda **kw` 替換 WizChatReader
    reader = WizChatReader(hwnd=hwnd, game_path=cfg.get("game_path"),
                           message_log=message_log, slot=slot)
    context = context if context is not None else ChatContext()
    inputs = _InputWatch(reader, hwnd, on_input_open, on_input_close)
    game_issue: str | None = None  # 遊戲端問題的橫幅文案 key（None＝遊戲正常）

    def wait_watching_input(seconds: float) -> None:
        """等待下一輪讀取，期間以 INPUT_POLL_INTERVAL 持續取樣輸入框狀態。
        input_open() 只讀一個已快取節點的旗標（實測 <0.1ms），讀聊天記錄則約 10ms，
        故兩者節奏分開。不另開執行緒：WizChatReader 內部跑自己的 asyncio loop，
        跨執行緒併發呼叫會踩到彼此。"""
        deadline = time.monotonic() + seconds
        while not stop.is_set():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return
            stop.wait(min(INPUT_POLL_INTERVAL, remaining))
            inputs.poll()

    while not stop.is_set():
        if not window_exists(hwnd):
            log(f"[reader] game window gone (slot={slot}, hwnd={hwnd:#x}); stopping this reader")
            inputs.force_closed()
            break
        # 每輪重讀設定：設定視窗可能在執行中切換系統訊息開關
        reader.emit_system = cfg.get("translate_system_messages", False)
        board.report(slot, "listening" if reader.anchored else "locating", game_issue)
        try:
            new_lines = reader.read_new()
        except GameNotRunning as exc:
            if isinstance(exc, GameAccessDenied):
                status, issue = "access_denied", "notice.access_denied"
            elif isinstance(exc, GameVersionMismatch):
                status, issue = "version_mismatch", "notice.version_mismatch"
            else:
                status, issue = "waiting_game", "notice.game_missing"
            if issue != game_issue:  # 只在原因改變時記錄，否則每輪重試都灌一行
                log(f"[reader] game not ready (slot={slot}, hwnd={hwnd:#x}): {exc}")
            game_issue = issue
            board.report(slot, status, game_issue)
            inputs.force_closed()
            stop.wait(GAME_MISSING_INTERVAL)
            continue
        except Exception as exc:  # 收訊偶發錯誤：略過該輪，不讓執行緒死掉
            log(f"[reader] poll skipped (slot={slot}): {type(exc).__name__}: {exc}\n"
                f"{traceback.format_exc()}")
            stop.wait(cfg["poll_interval"])
            continue

        if game_issue:
            game_issue = None
            log(f"[reader] game back, resuming (slot={slot})")

        for line in new_lines:
            msg_id = msg_ids.next()
            if line.system:
                # 系統訊息不進上下文；快取命中就直接以完成態顯示，不佔位也不進 pool
                cached = cache.get(line.text) if cache is not None else None
                if cached is not None:
                    ui_queue.put(lambda o=line.text, tr=cached, c=line.color:
                                 overlay.add_message(o, tr, color=c, slot=slot))
                    continue
                ui_queue.put(lambda o=line.text, c=line.color, m=msg_id:
                             overlay.add_message(o, t("notice.pending"), msg_id=m,
                                                 pending=True, color=c, slot=slot))
                if system_pool is not None:
                    system_pool.submit(line.text, [], msg_id)
                continue
            ctx = context.snapshot()   # 該行之前的行；提交後即固定，重試不漂移
            context.push(line.text)
            ui_queue.put(lambda o=line.text, c=line.color, m=msg_id:
                         overlay.add_message(o, t("notice.pending"), msg_id=m,
                                             pending=True, color=c, slot=slot))
            pool.submit(line.text, ctx, msg_id)

        if pool.in_flight or (system_pool is not None and system_pool.in_flight):
            board.report(slot, "translating", game_issue)
        else:
            board.report(slot, "listening" if reader.anchored else "locating", game_issue)

        inputs.poll()
        wait_watching_input(cfg["poll_interval"])

    board.drop(slot)
    reader.close()
