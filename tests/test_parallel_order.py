"""端到端：完成順序與顯示順序脫鉤——後到的訊息先翻完，overlay 仍維持讀取順序。"""
import queue
import threading
import time

from src.translation.context import ChatContext
from src.i18n import t
from src.reader.overlay import OverlayWindow
from src.translation.pool import TranslationPool

FAILED = "⚠  這則訊息翻譯不出來"


class ReverseOrderTranslator:
    """讓後提交的先完成：第一則等到其餘各則都翻完才放行。"""

    def __init__(self, hold_line: str, others: int):
        self._hold_line = hold_line
        self._others_done = threading.Semaphore(0)
        self._others = others

    def translate_incoming(self, text, context):
        if text == self._hold_line:
            for _ in range(self._others):
                assert self._others_done.acquire(timeout=5.0)
            return f"譯:{text}"
        result = f"譯:{text}"
        self._others_done.release()
        return result


def test_display_order_follows_read_order_not_completion_order(root):
    lines = ["[A] one", "[B] two", "[C] three"]
    overlay = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ui_queue: queue.Queue = queue.Queue()
    context = ChatContext()
    pool = TranslationPool(
        translator=ReverseOrderTranslator("[A] one", others=2),
        on_result=lambda mid, text, failed: ui_queue.put(
            lambda: overlay.update_message(mid, text, failed=failed)),
        workers=3, failed_notice_fn=lambda: FAILED)
    try:
        for i, line in enumerate(lines, start=1):
            ctx = context.snapshot()
            context.push(line)
            overlay.add_message(line, t("notice.pending"), msg_id=i)
            pool.submit(line, ctx, msg_id=i)
        for _ in range(500):                       # 至多 5 秒，收滿三則就停
            while True:
                try:
                    ui_queue.get_nowait()()
                except queue.Empty:
                    break
            if all(translated != t("notice.pending")
                  for _, translated in overlay.visible_messages()):
                break
            time.sleep(0.01)   # 主執行緒得讓出時間，worker 才有機會回報
        assert overlay.visible_messages() == [
            ("[A] one", "譯:[A] one"),
            ("[B] two", "譯:[B] two"),
            ("[C] three", "譯:[C] three"),
        ]
    finally:
        pool.shutdown(wait=True)
