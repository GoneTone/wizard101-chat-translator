"""收訊執行緒的主迴圈：讀遊戲聊天 → 推進上下文、在 overlay 佔位 → 交給翻譯池。

本迴圈不做翻譯（單則翻譯卡住不會延誤後續訊息的讀取與顯示），也不直接碰 Tk——
所有 UI 更新都包成回呼排進 ui_queue，由主執行緒的 pump 依序執行。
"""
import itertools
import queue
import sys
import threading
import time
from typing import TYPE_CHECKING

from src.i18n import t
from src.reader.mem_reader import (
    GameAccessDenied, GameNotRunning, GameVersionMismatch, WizChatReader,
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


def banner_for(game_issue: str | None, error_state: str | None) -> str | None:
    """決定該顯示哪一條錯誤橫幅的文案 key（None＝不顯示）。
    game_issue（遊戲端問題的文案 key）優先於翻譯錯誤：連不上遊戲時翻譯狀態已無意義。"""
    if game_issue:
        return game_issue
    if error_state == "config":
        return "notice.config_error"
    if error_state == "offline":
        return "notice.offline"
    return None


def reader_loop(cfg: dict, overlay: "OverlayWindow", ui_queue: queue.Queue,
                stop: threading.Event, context: ChatContext, pool: TranslationPool,
                on_input_open=None, on_input_close=None,
                message_log: MessageLog | None = None,
                system_pool: TranslationPool | None = None,
                cache: TranslationCache | None = None) -> None:
    """收訊執行緒的進入點；stop 被設定後解除 wizwalker hook 再返回。
    on_input_open／on_input_close＝遊戲聊天輸入框開關的邊緣觸發（auto_show_input）。"""
    reader = WizChatReader(game_path=cfg.get("game_path"), message_log=message_log)
    reader.emit_system = cfg.get("translate_system_messages", False)
    msg_ids = itertools.count(1)
    game_issue: str | None = None  # 遊戲端問題的橫幅文案 key（None＝遊戲正常）
    game_input_open = False
    last_status: str | None = None
    last_banner: str | None = None

    def set_status(state: str) -> None:
        nonlocal last_status
        if state == last_status:
            return
        last_status = state
        ui_queue.put(lambda s=state: overlay.set_status(s))

    def check_input() -> None:
        """遊戲聊天輸入框開／關的邊緣觸發：開 → 呼出翻譯輸入；關 → 收回。"""
        nonlocal game_input_open
        if on_input_open is None or not cfg.get("auto_show_input", True):
            return
        now_open = reader.input_open()
        if now_open == game_input_open:
            return
        game_input_open = now_open
        print(f"[reader] game chat input {'opened' if now_open else 'closed'}",
              file=sys.stderr)
        if now_open:
            on_input_open()
        elif on_input_close is not None:
            on_input_close()

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
            check_input()

    def translation_error() -> str | None:
        """兩條翻譯佇列任一有錯就顯示：玩家對話優先（它才是主要用途）。"""
        return pool.error_state or (system_pool.error_state if system_pool else None)

    def set_banner(key: str | None) -> None:
        nonlocal last_banner
        if key == last_banner:
            return
        last_banner = key
        if key is None:
            ui_queue.put(overlay.clear_error)
        else:
            ui_queue.put(lambda k=key: overlay.set_error(k))

    while not stop.is_set():
        reader.emit_system = cfg.get("translate_system_messages", False)
        set_status("listening" if reader.anchored else "locating")
        try:
            new_lines = reader.read_new()
        except GameNotRunning as exc:
            if isinstance(exc, GameAccessDenied):
                status, issue = "access_denied", "notice.access_denied"
            elif isinstance(exc, GameVersionMismatch):
                status, issue = "version_mismatch", "notice.version_mismatch"
            else:
                status, issue = "waiting_game", "notice.game_missing"
            set_status(status)
            if issue != game_issue:  # 只在原因改變時記錄，否則每輪重試都灌一行
                print(f"[reader] game not ready: {exc}", file=sys.stderr)
            game_issue = issue
            set_banner(banner_for(game_issue, translation_error()))
            if game_input_open:
                game_input_open = False  # 遊戲斷線＝輸入框已不存在，同步收回
                if on_input_close is not None:
                    on_input_close()
            stop.wait(GAME_MISSING_INTERVAL)
            continue
        except Exception as exc:  # 收訊偶發錯誤：略過該輪，不讓執行緒死掉
            print(f"[reader] poll skipped: {exc}", file=sys.stderr)
            stop.wait(cfg["poll_interval"])
            continue

        if game_issue:
            game_issue = None
            print("[reader] game back, resuming", file=sys.stderr)

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

        set_banner(banner_for(game_issue, translation_error()))
        if pool.in_flight or (system_pool is not None and system_pool.in_flight):
            set_status("translating")
        else:
            set_status("listening" if reader.anchored else "locating")

        check_input()
        wait_watching_input(cfg["poll_interval"])

    reader.close()
