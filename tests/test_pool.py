"""TranslationPool：平行翻譯、單則卡住不擋後續、失敗退避與重試。"""
import threading
import time

import pytest

from src.translation.pool import TranslationPool
from src.translation.translator import TranslatorBadOutput, TranslatorConfigError, TranslatorOffline

FAILED = "⚠  這則訊息翻譯不出來"


@pytest.fixture(autouse=True)
def fast_backoff(monkeypatch):
    """把退避縮到毫秒級：測的是「有沒有退避與重置」，不是真的等 5 秒。"""
    import src.translation.pool as pool_module
    monkeypatch.setattr(pool_module, "BACKOFF_STEPS", [0.01, 0.01, 0.01])
    monkeypatch.setattr(pool_module, "CONFIG_ERROR_INTERVAL", 0.01)


class Collector:
    """收集 on_result 回報，供測試等待與斷言。"""

    def __init__(self):
        self.results: dict[int, str] = {}
        self.failed: set[int] = set()
        self._lock = threading.Lock()
        self._event = threading.Event()

    def __call__(self, msg_id, text, failed):
        with self._lock:
            self.results[msg_id] = text
            if failed:
                self.failed.add(msg_id)
        self._event.set()

    def wait_for(self, count, timeout=30.0):
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
                           workers=workers, failed_notice_fn=lambda: FAILED)


class FailThenOk:
    """前 n 次拋出指定例外，之後成功。"""

    def __init__(self, exc, failures):
        self._exc = exc
        self._left = failures
        self._lock = threading.Lock()
        self.calls = 0

    def translate_incoming(self, text, context):
        with self._lock:
            self.calls += 1
            fail = self._left > 0
            if fail:
                self._left -= 1
        if fail:
            raise self._exc
        return f"譯:{text}"


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
                release.wait()   # 不設逾時：逾時到期會讓它自己解除阻塞，見下方 finally
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
            release.wait()   # 不設逾時：finally 一定會放行，逾時只會讓它提早跑完
            return "譯"

    c = Collector()
    pool = _pool(SlowTranslator(), c, workers=1)
    try:
        assert pool.in_flight == 0
        pool.submit("[A] one", [], msg_id=1)
        assert started.wait(30.0)
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


def test_submit_survives_concurrent_resize():
    """submit() 與 resize() 競速時：呼叫端不能看到例外，drain 後 in_flight 也要歸零
    （回歸測試：修 review 找到的 submit/resize/shutdown 競速導致 RuntimeError 洩漏 in_flight）。"""
    c = Collector()
    pool = _pool(OkTranslator(), c, workers=2)
    stop = threading.Event()
    errors = []
    submitted = []
    submitted_lock = threading.Lock()

    def submitter():
        i = 0
        while not stop.is_set() and i < 5000:
            try:
                pool.submit(f"[A] m{i}", [], msg_id=i)
                with submitted_lock:
                    submitted.append(i)
            except Exception as exc:            # 核心斷言：submit() 不可對呼叫端拋例外
                errors.append(exc)
            i += 1

    def resizer():
        for w in (1, 3, 2, 4, 1, 2) * 50:
            pool.resize(w)

    try:
        t_submit = threading.Thread(target=submitter)
        t_resize = threading.Thread(target=resizer)
        t_submit.start()
        t_resize.start()
        t_resize.join(5.0)
        stop.set()
        t_submit.join(5.0)
        assert not t_submit.is_alive(), "submitter 執行緒逾時未結束"
        assert not errors, f"submit() 對呼叫端拋出例外: {errors}"

        with submitted_lock:
            expected_ids = list(submitted)
        got = c.wait_for(len(expected_ids))
        assert set(got) == set(expected_ids)
        assert pool.in_flight == 0
    finally:
        stop.set()
        pool.shutdown(wait=True)


def test_shutdown_cancels_queued_work_without_leaking_in_flight():
    """shutdown() 對被 cancel_futures 取消的排隊工作也要讓 in_flight 歸零（回歸：取消的
    work item 不會經 _work 的 finally 遞減，之前會讓狀態指示永遠顯示『翻譯中…』）。"""
    release = threading.Event()
    started = threading.Event()

    class SlowTranslator:
        def translate_incoming(self, text, context):
            started.set()
            release.wait()   # 不設逾時：finally 一定會放行，逾時只會讓它提早跑完
            return "譯"

    c = Collector()
    pool = _pool(SlowTranslator(), c, workers=1)
    try:
        pool.submit("[A] running", [], msg_id=1)
        assert started.wait(30.0)                        # 唯一的 worker 卡在第一則翻譯中
        for i in range(2, 6):
            pool.submit(f"[A] queued{i}", [], msg_id=i)  # 排隊中，worker 尚未取用
        assert pool.in_flight == 5

        pool.shutdown(wait=False)                        # cancel_futures 同步取消佇列中 4 則
        assert pool.in_flight == 1                        # 只剩正在執行中的那一則

        release.set()                                     # 放行卡住的那一則
        assert c.wait_for(1) == {1: "譯"}
        assert pool.in_flight == 0
    finally:
        release.set()
        pool.shutdown(wait=True)


def test_offline_retries_until_success_and_clears_error_state():
    tr = FailThenOk(TranslatorOffline("down"), failures=2)
    c = Collector()
    pool = _pool(tr, c, workers=1)
    try:
        pool.submit("[A] one", [], msg_id=1)
        assert c.wait_for(1)[1] == "譯:[A] one"
        assert tr.calls == 3                  # 兩次失敗 + 一次成功
        assert pool.error_state is None       # 成功後狀態清除
    finally:
        pool.shutdown(wait=True)


def test_offline_sets_error_state_while_failing():
    blocked = threading.Event()

    class AlwaysOffline:
        def translate_incoming(self, text, context):
            blocked.set()
            raise TranslatorOffline("down")

    c = Collector()
    pool = _pool(AlwaysOffline(), c, workers=1)
    try:
        pool.submit("[A] one", [], msg_id=1)
        assert blocked.wait(30.0)
        deadline = time.monotonic() + 30.0
        while pool.error_state != "offline" and time.monotonic() < deadline:
            time.sleep(0.01)
        assert pool.error_state == "offline"
    finally:
        pool.shutdown(wait=True)


def test_error_detail_exposes_the_api_message_while_failing():
    blocked = threading.Event()

    class AlwaysRejected:
        def translate_incoming(self, text, context):
            blocked.set()
            raise TranslatorConfigError("Incorrect API key", status=401)

    c = Collector()
    pool = _pool(AlwaysRejected(), c, workers=1)
    try:
        pool.submit("[A] one", [], msg_id=1)
        assert blocked.wait(30.0)
        deadline = time.monotonic() + 30.0
        while pool.error_detail is None and time.monotonic() < deadline:
            time.sleep(0.01)
        assert pool.error_detail == (401, "Incorrect API key")
    finally:
        pool.shutdown(wait=True)


def test_error_detail_is_none_without_a_message_and_after_recovery():
    tr = FailThenOk(TranslatorOffline("upstream down", status=503), failures=1)
    c = Collector()
    pool = _pool(tr, c, workers=1)
    try:
        pool.submit("[A] one", [], msg_id=1)
        assert c.wait_for(1)[1] == "譯:[A] one"
        assert pool.error_detail is None          # 成功後連同 error_state 一起清掉
    finally:
        pool.shutdown(wait=True)
    tr = FailThenOk(TranslatorOffline(status=503), failures=99)
    pool = _pool(tr, Collector(), workers=1)
    try:
        pool.submit("[A] one", [], msg_id=1)
        deadline = time.monotonic() + 30.0
        while pool.error_state is None and time.monotonic() < deadline:
            time.sleep(0.01)
        assert pool.error_state == "offline"
        assert pool.error_detail is None          # API 沒說明：橫幅退回固定文案
    finally:
        pool.shutdown(wait=True)


def test_config_error_retries_and_reports_config_state():
    tr = FailThenOk(TranslatorConfigError("bad key", status=401), failures=1)
    c = Collector()
    pool = _pool(tr, c, workers=1)
    try:
        pool.submit("[A] one", [], msg_id=1)
        assert c.wait_for(1)[1] == "譯:[A] one"
        assert tr.calls == 2
        assert pool.error_state is None
    finally:
        pool.shutdown(wait=True)


def test_bad_output_is_not_retried():
    # temperature=0 下重試必得同一結果：截斷一律放棄，直接回失敗提示
    tr = FailThenOk(TranslatorBadOutput("truncated"), failures=99)
    c = Collector()
    pool = _pool(tr, c, workers=1)
    try:
        pool.submit("[A] one", [], msg_id=1)
        assert c.wait_for(1)[1] == FAILED
        assert tr.calls == 1
    finally:
        pool.shutdown(wait=True)


def test_backoff_gate_is_shared_across_workers():
    # 伺服器離線是全域事實：四個 worker 不得以四倍速重打
    hits = []
    lock = threading.Lock()

    class CountingOffline:
        def translate_incoming(self, text, context):
            with lock:
                hits.append(time.monotonic())
            raise TranslatorOffline("down")

    c = Collector()
    pool = _pool(CountingOffline(), c, workers=4)
    try:
        for i in range(4):
            pool.submit(f"[A] m{i}", [], msg_id=i)
        deadline = time.monotonic() + 30.0
        while time.monotonic() < deadline:
            with lock:
                if len(hits) >= 8:
                    break
            time.sleep(0.01)
        with lock:
            assert len(hits) >= 8       # 有持續重試
    finally:
        pool.shutdown(wait=True)


def test_backoff_index_advances_once_per_burst_not_per_worker(monkeypatch):
    """回歸測試（review 發現）：N 個 worker 同時撞上剛開啟的閘門時，只能推進一層退避，
    不能因同時失敗的 worker 數量一次跳好幾階。

    第二輪刻意卡住、由測試放行，藉此確定斷言時機 —— 不靠量時間，避免 flaky。"""
    import src.translation.pool as pool_module
    monkeypatch.setattr(pool_module, "BACKOFF_STEPS", [0.05, 0.1, 0.2])
    workers = 4
    barrier = threading.Barrier(workers)
    proceed_round_2 = threading.Event()

    class BurstOffline:
        def __init__(self):
            self._count_lock = threading.Lock()
            self.calls = 0

        def translate_incoming(self, text, context):
            with self._count_lock:
                self.calls += 1
                call_no = self.calls
            if call_no <= workers:
                # 保險絲不能拿掉：barrier 沒有像 proceed_round_2 那樣的 finally 放行路徑，
                # 無限等待會在 worker 數不足時掛住整個行程（executor 執行緒非 daemon）
                barrier.wait(60.0)             # 逼所有 worker 真正同時進入第一輪失敗
            else:
                proceed_round_2.wait()   # 不設逾時：finally 一定會放行
            raise TranslatorOffline("down")

    tr = BurstOffline()
    c = Collector()
    pool = _pool(tr, c, workers=workers)
    try:
        for i in range(workers):
            pool.submit(f"[A] m{i}", [], msg_id=i)

        # 等所有 worker 都各自跑完第一輪、進入（並卡在）第二輪嘗試 ——
        # 這保證每個 worker 自己的第一輪 _note_failure 都已經跑完。
        deadline = time.monotonic() + 30.0
        while tr.calls < workers * 2 and time.monotonic() < deadline:
            time.sleep(0.005)
        assert tr.calls == workers * 2

        assert pool._backoff_index == 1        # 同一輪的並發失敗只能推進一層
        assert pool.error_state == "offline"
    finally:
        proceed_round_2.set()
        pool.shutdown(wait=True)


def test_uses_the_supplied_translate_fn():
    class Tr:
        def translate_incoming(self, text, context):
            raise AssertionError("不該走收訊路徑")

        def translate_system_message(self, text):
            return f"譯:{text}"

    collector = Collector()
    tr = Tr()
    pool = TranslationPool(translator=tr, on_result=collector, workers=1,
                           failed_notice_fn=lambda: FAILED,
                           translate_fn=lambda text, context: tr.translate_system_message(text))
    pool.submit("你获得了 {0} 金币！", [], 1)
    assert collector.wait_for(1)[1] == "譯:你获得了 {0} 金币！"
    pool.shutdown(wait=True)


def test_two_pools_sharing_a_gate_never_exceed_the_total_limit():
    from src.translation.gate import ConcurrencyGate

    gate = ConcurrencyGate(2)
    peak = {"value": 0}
    active = threading.Lock()
    running = []

    def translate(text, context):
        with active:
            running.append(text)
            peak["value"] = max(peak["value"], len(running))
        time.sleep(0.05)
        with active:
            running.remove(text)
        return f"譯:{text}"

    class Tr:
        def translate_incoming(self, text, context):
            return translate(text, context)

    a_collector, b_collector = Collector(), Collector()
    a = TranslationPool(translator=Tr(), on_result=a_collector, workers=4,
                        failed_notice_fn=lambda: FAILED, gate=gate)
    b = TranslationPool(translator=Tr(), on_result=b_collector, workers=4,
                        failed_notice_fn=lambda: FAILED, gate=gate)
    for i in range(6):
        a.submit(f"a{i}", [], i)
        b.submit(f"b{i}", [], i)
    a_collector.wait_for(6)
    b_collector.wait_for(6)
    assert peak["value"] <= 2, f"總併發衝到 {peak['value']}，應不超過閘的上限"
    a.shutdown(wait=True)
    b.shutdown(wait=True)


def test_resize_also_raises_the_gate_limit():
    from src.translation.gate import ConcurrencyGate

    gate = ConcurrencyGate(1)

    class Tr:
        def translate_incoming(self, text, context):
            return text

    pool = TranslationPool(translator=Tr(), on_result=Collector(), workers=1,
                           failed_notice_fn=lambda: FAILED, gate=gate)
    pool.resize(4)
    stop = threading.Event()
    assert all(gate.acquire(stop) for _ in range(4)), "閘的上限應跟著 resize 調大"
    pool.shutdown(wait=True)


def test_backoff_index_escalates_across_separate_failure_rounds(monkeypatch):
    """回歸測試：burst 去重不能讓「真正分開的多輪失敗」不再逐階推進。單一 worker 保證
    每次呼叫都是獨立一輪（前一輪閘門到期才有下一次呼叫），不會被誤判成同輪的兄弟失敗。"""
    import src.translation.pool as pool_module
    monkeypatch.setattr(pool_module, "BACKOFF_STEPS", [0.02, 0.04, 0.08])

    class RecordingOffline:
        def __init__(self, failures):
            self._left = failures
            self.observed_indices = []
            self.pool = None  # 建 pool 後才補上

        def translate_incoming(self, text, context):
            if self._left > 0:
                self._left -= 1
                self.observed_indices.append(self.pool._backoff_index)
                raise TranslatorOffline("down")
            return "ok"

    tr = RecordingOffline(failures=3)
    c = Collector()
    pool = _pool(tr, c, workers=1)
    tr.pool = pool
    try:
        pool.submit("[A] one", [], msg_id=1)
        assert c.wait_for(1)[1] == "ok"
        assert tr.observed_indices == [0, 1, 2]   # 三輪各自獨立、逐階推進
    finally:
        pool.shutdown(wait=True)
