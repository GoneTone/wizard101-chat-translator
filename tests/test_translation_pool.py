"""TranslationPool：平行翻譯、單則卡住不擋後續、失敗退避與重試。"""
import threading

from src.translation_pool import TranslationPool

FAILED = "⚠  這則訊息翻譯不出來"


class Collector:
    """收集 on_result 回報，供測試等待與斷言。"""

    def __init__(self):
        self.results: dict[int, str] = {}
        self._lock = threading.Lock()
        self._event = threading.Event()

    def __call__(self, msg_id, text):
        with self._lock:
            self.results[msg_id] = text
        self._event.set()

    def wait_for(self, count, timeout=5.0):
        """等到收滿 count 則結果並回傳；逾時即斷言失敗（不靠 sleep 猜時間）。"""
        while True:
            with self._lock:
                if len(self.results) >= count:
                    return dict(self.results)
            if not self._event.wait(timeout):
                with self._lock:
                    raise AssertionError(f"只收到 {len(self.results)} 則，預期 {count}")
            self._event.clear()


class OkTranslator:
    def translate_incoming(self, text, context):
        return f"譯:{text}|ctx={len(context)}"


def _pool(translator, collector, workers=4):
    return TranslationPool(translator=translator, on_result=collector,
                           workers=workers, failed_notice=FAILED)


def test_submitted_lines_are_translated_and_reported():
    c = Collector()
    pool = _pool(OkTranslator(), c)
    try:
        for i in range(4):
            pool.submit(f"[A] m{i}", ["[Z] ctx"], msg_id=i)
        assert c.wait_for(4) == {i: f"譯:[A] m{i}|ctx=1" for i in range(4)}
    finally:
        pool.shutdown(wait=True)


def test_blocked_line_does_not_block_the_others():
    # 需求核心：一則卡住時，其餘各則仍須照常完成
    release = threading.Event()

    class BlockingTranslator:
        def translate_incoming(self, text, context):
            if text == "[A] stuck":
                release.wait(5.0)
            return f"譯:{text}"

    c = Collector()
    pool = _pool(BlockingTranslator(), c, workers=4)
    try:
        pool.submit("[A] stuck", [], msg_id=0)
        for i in range(1, 4):
            pool.submit(f"[A] m{i}", [], msg_id=i)
        got = c.wait_for(3)                       # 卡住那則還沒回來，其餘三則已完成
        assert set(got) == {1, 2, 3}
        release.set()
        assert c.wait_for(4)[0] == "譯:[A] stuck"
    finally:
        release.set()
        pool.shutdown(wait=True)


def test_in_flight_counts_outstanding_work():
    release = threading.Event()
    started = threading.Event()

    class SlowTranslator:
        def translate_incoming(self, text, context):
            started.set()
            release.wait(5.0)
            return "譯"

    c = Collector()
    pool = _pool(SlowTranslator(), c, workers=1)
    try:
        assert pool.in_flight == 0
        pool.submit("[A] one", [], msg_id=1)
        assert started.wait(5.0)
        assert pool.in_flight == 1
        release.set()
        c.wait_for(1)
        assert pool.in_flight == 0
    finally:
        release.set()
        pool.shutdown(wait=True)


def test_resize_keeps_pool_usable():
    c = Collector()
    pool = _pool(OkTranslator(), c, workers=2)
    try:
        pool.submit("[A] before", [], msg_id=1)
        c.wait_for(1)
        pool.resize(4)
        pool.submit("[A] after", [], msg_id=2)
        assert c.wait_for(2)[2] == "譯:[A] after|ctx=0"
    finally:
        pool.shutdown(wait=True)
