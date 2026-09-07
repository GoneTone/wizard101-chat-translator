"""收訊翻譯工作池：以固定數量的 worker 平行翻譯，單則卡住不影響其他則。

呼叫端只負責提交（line, context, msg_id）與接收 on_result(msg_id, text, failed)；
顯示順序不由完成順序決定——overlay 在提交當下就已佔好位置（見 reader_loop）。
"""
import threading
import time
from concurrent.futures import ThreadPoolExecutor

from src.log import log
from src.translation.translator import TranslatorBadOutput, TranslatorConfigError, TranslatorOffline

BACKOFF_STEPS = [5, 15, 30]   # 翻譯伺服器離線時的重試間隔（秒）
CONFIG_ERROR_INTERVAL = 15.0  # API 設定錯誤時的重試間隔（秒）；使用者修正後自動恢復


class TranslationPool:
    """平行收訊翻譯。`on_result(msg_id, text, failed)` 於 worker 執行緒呼叫，
    failed＝放棄該則、text 是失敗提示而非譯文。
    呼叫端負責把它轉交回 UI 執行緒。"""

    def __init__(self, translator, on_result, workers: int, failed_notice_fn,
                 translate_fn=None, gate=None):
        self._translator = translator
        self._on_result = on_result
        # 取 callable 而非字串：介面語言可能在執行中被改掉，定案的字串會停在舊語言。
        self._failed_notice_fn = failed_notice_fn
        # 哪一條翻譯路徑：預設收訊（吃 context），系統訊息 pool 傳入自己的。
        self._translate_fn = translate_fn or (
            lambda text, context: translator.translate_incoming(text, context))
        # 總量閘（None＝不限，測試與單 pool 用）。與退避閘門（_gate_until）是兩回事：
        # 那是「伺服器掛了、全體暫停」，這是「同時最多幾則」。
        self._concurrency = gate
        self._workers = workers
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._in_flight = 0
        self._error_state: str | None = None
        self._error_detail: tuple[int | None, str] | None = None
        self._backoff_index = 0
        self._gate_until = 0.0   # time.monotonic() 之前不得送出新請求
        self._executor = ThreadPoolExecutor(max_workers=workers,
                                            thread_name_prefix="translate")

    @property
    def in_flight(self) -> int:
        """已提交但尚未回報結果的則數（供狀態指示判斷是否顯示『翻譯中…』）。"""
        with self._lock:
            return self._in_flight

    @property
    def error_state(self) -> str | None:
        """目前的翻譯錯誤狀態：None／"offline"／"config"，供上層決定錯誤橫幅。"""
        with self._lock:
            return self._error_state

    @property
    def error_detail(self) -> tuple[int | None, str] | None:
        """最近一次失敗的（HTTP 狀態碼，API 說明）；沒在失敗或 API 沒給說明時為 None，
        橫幅據此決定要照實顯示 API 訊息還是退回固定文案。"""
        with self._lock:
            return self._error_detail

    def submit(self, line: str, context: list[str], msg_id: int) -> None:
        """提交一則翻譯。context 為提交當下的快照，重試時沿用同一份。

        「檢查 _stop → executor.submit()」整段都在 _lock 內，與 resize()／shutdown()
        換掉／關閉 executor 的臨界區互斥；否則鎖外送件會撞上已 shutdown 的 executor
        炸出 RuntimeError，且 in_flight 已加計卻永遠等不到 _work 遞減。"""
        with self._lock:
            if self._stop.is_set():
                return
            self._in_flight += 1
            try:
                future = self._executor.submit(self._work, line, context, msg_id)
            except RuntimeError:
                # 已被同一把鎖排除；留著防呆，避免重構重新打開競速窗口。
                self._in_flight -= 1
                log(f"[translate] submit rejected, executor already shut down: {line}")
                return
        future.add_done_callback(self._on_future_done)

    def _on_future_done(self, future) -> None:
        """被 shutdown(cancel_futures=True) 取消、尚未開跑的 work item，_work 的 finally
        不會執行，這裡補上那次 in_flight 遞減；正常完成的 future 不是 cancelled，不重複計數。"""
        if future.cancelled():
            with self._lock:
                self._in_flight -= 1

    def resize(self, workers: int) -> None:
        """變更平行度。舊 executor 放生（手上的工作跑完仍會經 on_result 回報，
        msg_id 不受影響），本物件身分不變——持有本 pool 參考的呼叫端不需更新。"""
        with self._lock:
            if workers == self._workers:
                return
            old = self._executor
            self._workers = workers
            self._executor = ThreadPoolExecutor(max_workers=workers,
                                                thread_name_prefix="translate")
        old.shutdown(wait=False)
        if self._concurrency is not None:
            self._concurrency.set_limit(workers)
        log(f"[translate] pool resized to {workers} workers")

    def shutdown(self, wait: bool = False) -> None:
        """停止接受新工作並要求 worker 盡快收手。"""
        self._stop.set()
        with self._lock:
            executor = self._executor
        executor.shutdown(wait=wait, cancel_futures=True)

    def _decrement_in_flight(self) -> None:
        with self._lock:
            self._in_flight -= 1

    def _work(self, line: str, context: list[str], msg_id: int) -> None:
        """每則工作至多回報一次 on_result。

        in_flight 在呼叫 on_result 之前遞減，而非事後靠 finally——否則 on_result 回呼內
        讀 in_flight 會把已有結果的這則算進行中。`decremented` 防止 finally 再扣一次；
        沒有呼叫 on_result 的路徑（stop 跳出、_wait_for_gate 放棄）則交給 finally。
        合計每則恰好遞減一次，與 _on_future_done 對「送出前就被取消」的遞減互斥。"""
        attempts = 0
        decremented = False
        try:
            while not self._stop.is_set():
                if not self._wait_for_gate():
                    return          # 關閉中：放棄這則，不回報
                attempts += 1
                try:
                    translated = self._call_translate(line, context)
                except TranslatorOffline as exc:
                    self._note_failure("offline", exc)
                    continue        # 該則留著重試，伺服器恢復就補上
                except TranslatorConfigError as exc:
                    self._note_failure("config", exc)
                    continue        # 等使用者修正設定後自動恢復
                except Exception as exc:
                    # 截斷或格式異常：temperature=0 下重試必得同一結果，直接放棄，
                    # 讓 overlay 至少顯示原文而不是無聲消失。
                    reason = ("bad model output" if isinstance(exc, TranslatorBadOutput)
                              else "unexpected error")
                    log(f"[translate] line dropped after {attempts} attempt(s), "
                        f"{reason} ({exc}): {line}")
                    self._decrement_in_flight()
                    decremented = True
                    self._on_result(msg_id, self._failed_notice_fn(), True)
                    return
                self._note_success()
                self._decrement_in_flight()
                decremented = True
                self._on_result(msg_id, translated, False)
                return
        finally:
            if not decremented:
                self._decrement_in_flight()

    def _call_translate(self, line: str, context: list[str]) -> str:
        """在總量閘的額度內送出一次翻譯請求。取不到額度（關閉中）視為離線、
        交給既有的重試路徑處理——此時 _stop 已設定，下一圈就會收手。"""
        if self._concurrency is None:
            return self._translate_fn(line, context)
        if not self._concurrency.acquire(self._stop):
            raise TranslatorOffline("shutting down while waiting for a concurrency slot")
        try:
            return self._translate_fn(line, context)
        finally:
            self._concurrency.release()

    def _wait_for_gate(self) -> bool:
        """等到退避閘門開啟；關閉中回傳 False。分段等待讓 shutdown 能及時打斷。"""
        while not self._stop.is_set():
            with self._lock:
                remaining = self._gate_until - time.monotonic()
            if remaining <= 0:
                return True
            self._stop.wait(min(remaining, 0.5))
        return False

    def _note_failure(self, state: str, exc: Exception) -> None:
        """推進全域退避閘門並記錄狀態。閘門全域：伺服器離線是全域事實，否則 N 個 worker
        會以 N 倍速重打同一台掛掉的伺服器。閘門未到期代表另一個 worker 已替這一輪推進過，
        沿用即可——否則 N 個 worker 同時撞上會把 backoff_index 一口氣推 N 階、直接封頂。"""
        with self._lock:
            now = time.monotonic()
            if now >= self._gate_until:
                if state == "config":
                    delay = CONFIG_ERROR_INTERVAL
                else:
                    steps = BACKOFF_STEPS
                    delay = steps[min(self._backoff_index, len(steps) - 1)]
                    self._backoff_index += 1
                self._gate_until = now + delay
            changed = self._error_state != state
            self._error_state = state
            self._error_detail = (exc.status, exc.detail) if exc.detail else None
        if changed:
            log(f"[translate] provider {state} error: {exc}; "
                f"retrying with backoff")

    def _note_success(self) -> None:
        """任何一次成功都代表伺服器與設定已恢復：清狀態、重置退避、放開閘門。"""
        with self._lock:
            changed = self._error_state is not None
            self._error_state = None
            self._error_detail = None
            self._backoff_index = 0
            self._gate_until = 0.0
        if changed:
            log("[translate] recovered, resuming normal speed")
