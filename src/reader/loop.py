"""收訊執行緒的主迴圈：讀遊戲聊天 → 推進上下文、在 overlay 佔位 → 交給翻譯池。

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
from src.translation.cache import TranslationCache
from src.translation.context import ChatContext
from src.translation.pool import TranslationPool

if TYPE_CHECKING:   # 只當型別用：reader 不該在執行期依賴 ui
    from src.ui.overlay import OverlayWindow

GAME_MISSING_INTERVAL = 5.0  # 找不到遊戲時的重試間隔（秒）
# 遊戲聊天輸入框的取樣間隔（秒）：只讀一個可見性旗標，可比 poll_interval 密得多，
# 讓翻譯輸入框幾乎在聊天欄打開的當下就彈出
INPUT_POLL_INTERVAL = 0.05


_PLAIN_NOTICE = {"config": "notice.config_error", "offline": "notice.offline"}


def banner_for(game_issue: str | None, error_state: str | None,
               error_detail: tuple[int | None, str] | None = None) -> tuple[str, dict] | None:
    """決定該顯示哪一條錯誤橫幅：（文案 key，format 變數）或 None＝不顯示。
    game_issue（遊戲端問題的文案 key）優先於翻譯錯誤：連不上遊戲時翻譯狀態已無意義。
    error_detail 是 pool 記下的（HTTP 狀態碼，API 說明）：有就照實顯示，
    沒有才退回只靠狀態猜的固定文案。"""
    if game_issue:
        return game_issue, {}
    if error_state not in _PLAIN_NOTICE:
        return None
    if not error_detail:
        return _PLAIN_NOTICE[error_state], {}
    status, message = error_detail
    if error_state == "config":
        return "notice.config_error_detail", {"status": status, "message": message}
    if status is not None:
        return "notice.offline_http", {"status": status, "message": message}
    return "notice.offline_detail", {"message": message}


def translation_banner(game_issue: str | None, pool: TranslationPool,
                       system_pool: TranslationPool | None) -> tuple[str, dict] | None:
    """兩條翻譯佇列任一有錯就顯示：玩家對話優先（它才是主要用途）。"""
    failing = pool if pool.error_state else system_pool
    if failing is None:
        return banner_for(game_issue, None)
    return banner_for(game_issue, failing.error_state, failing.error_detail)


class _OverlayFeed:
    """reader 執行緒對 overlay 的推送：狀態字與橫幅都只在變化時才排進 ui_queue，
    否則每輪 poll 都會塞一個沒有意義的重繪回呼。"""

    def __init__(self, overlay: "OverlayWindow", ui_queue: queue.Queue):
        self._overlay = overlay
        self._queue = ui_queue
        self._status: str | None = None
        self._banner: tuple[str, dict] | None = None

    def set_status(self, state: str) -> None:
        if state == self._status:
            return
        self._status = state
        self._queue.put(lambda: self._overlay.set_status(state))

    def set_banner(self, banner: tuple[str, dict] | None) -> None:
        if banner == self._banner:
            return
        self._banner = banner
        if banner is None:
            self._queue.put(self._overlay.clear_error)
        else:
            key, kwargs = banner
            self._queue.put(lambda: self._overlay.set_error(key, **kwargs))


class _InputWatch:
    """遊戲聊天輸入框開／關的邊緣觸發：開 → 回報錨點；關 → 回報關閉。
    on_input_open 為 None 時完全不取樣（呼叫端不關心輸入框）。"""

    def __init__(self, reader: WizChatReader, on_input_open, on_input_close):
        self._reader = reader
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
            log(f"[reader] game chat input opened (anchor={anchor})")
            self._on_open(anchor)
        else:
            self._report_closed()

    def force_closed(self) -> None:
        """遊戲斷線＝輸入框已不存在：開著就同步收回。"""
        if self.open:
            self.open = False
            self._report_closed()

    def _report_closed(self) -> None:
        log("[reader] game chat input closed")
        if self._on_close is not None:
            self._on_close()


def reader_loop(cfg: dict, overlay: "OverlayWindow", ui_queue: queue.Queue,
                stop: threading.Event, context: ChatContext, pool: TranslationPool,
                on_input_open=None, on_input_close=None,
                message_log: MessageLog | None = None,
                system_pool: TranslationPool | None = None,
                cache: TranslationCache | None = None) -> None:
    """收訊執行緒的進入點；stop 被設定後解除 wizwalker hook 再返回。
    on_input_open(anchor)／on_input_close＝遊戲聊天輸入框開關的邊緣觸發，不受 auto_show_input
    影響（要不要自動呼出由呼叫端決定，錨點則熱鍵呼出也用得到）；
    anchor＝遊戲輸入框的螢幕矩形 (x, y, w, h)，讀不到為 None。"""
    reader = WizChatReader(game_path=cfg.get("game_path"), message_log=message_log)
    feed = _OverlayFeed(overlay, ui_queue)
    inputs = _InputWatch(reader, on_input_open, on_input_close)
    msg_ids = itertools.count(1)
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
        # 每輪重讀設定：設定視窗可能在執行中切換系統訊息開關
        reader.emit_system = cfg.get("translate_system_messages", False)
        feed.set_status("listening" if reader.anchored else "locating")
        try:
            new_lines = reader.read_new()
        except GameNotRunning as exc:
            if isinstance(exc, GameAccessDenied):
                status, issue = "access_denied", "notice.access_denied"
            elif isinstance(exc, GameVersionMismatch):
                status, issue = "version_mismatch", "notice.version_mismatch"
            else:
                status, issue = "waiting_game", "notice.game_missing"
            feed.set_status(status)
            if issue != game_issue:  # 只在原因改變時記錄，否則每輪重試都灌一行
                log(f"[reader] game not ready: {exc}")
            game_issue = issue
            feed.set_banner(translation_banner(game_issue, pool, system_pool))
            inputs.force_closed()
            stop.wait(GAME_MISSING_INTERVAL)
            continue
        except Exception as exc:  # 收訊偶發錯誤：略過該輪，不讓執行緒死掉
            log(f"[reader] poll skipped: {type(exc).__name__}: {exc}\n"
                f"{traceback.format_exc()}")
            stop.wait(cfg["poll_interval"])
            continue

        if game_issue:
            game_issue = None
            log("[reader] game back, resuming")

        for line in new_lines:
            msg_id = next(msg_ids)
            if line.system:
                # 系統訊息不進上下文；快取命中就直接以完成態顯示，不佔位也不進 pool
                cached = cache.get(line.text) if cache is not None else None
                if cached is not None:
                    ui_queue.put(lambda o=line.text, tr=cached, c=line.color:
                                 overlay.add_message(o, tr, color=c))
                    continue
                ui_queue.put(lambda o=line.text, c=line.color, m=msg_id:
                             overlay.add_message(o, t("notice.pending"), msg_id=m,
                                                 pending=True, color=c))
                if system_pool is not None:
                    system_pool.submit(line.text, [], msg_id)
                continue
            ctx = context.snapshot()   # 該行之前的行；提交後即固定，重試不漂移
            context.push(line.text)
            ui_queue.put(lambda o=line.text, c=line.color, m=msg_id:
                         overlay.add_message(o, t("notice.pending"), msg_id=m,
                                             pending=True, color=c))
            pool.submit(line.text, ctx, msg_id)

        feed.set_banner(translation_banner(game_issue, pool, system_pool))
        if pool.in_flight or (system_pool is not None and system_pool.in_flight):
            feed.set_status("translating")
        else:
            feed.set_status("listening" if reader.anchored else "locating")

        inputs.poll()
        wait_watching_input(cfg["poll_interval"])

    reader.close()
