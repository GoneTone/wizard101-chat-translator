"""收訊翻譯工作池：以固定數量的 worker 平行翻譯，單則卡住不影響其他則。

呼叫端只負責提交（line, context, msg_id）與接收 on_result(msg_id, text)；
顯示順序不由完成順序決定——overlay 在提交當下就已佔好位置（見 reader_loop）。
"""
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor

from src.translator import TranslatorBadOutput, TranslatorConfigError, TranslatorOffline

BACKOFF_STEPS = [5, 15, 30]   # 翻譯伺服器離線時的重試間隔（秒）
CONFIG_ERROR_INTERVAL = 15.0  # API 設定錯誤時的重試間隔（秒）；使用者修正後自動恢復


class TranslationPool:
    """平行收訊翻譯。`on_result(msg_id, text)` 於 worker 執行緒呼叫，
    呼叫端負責把它轉交回 UI 執行緒。"""

    def __init__(self, translator, on_result, workers: int, failed_notice: str):
        self._translator = translator
        self._on_result = on_result
        self._failed_notice = failed_notice
        self._workers = workers
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._in_flight = 0
        self._error_state: str | None = None
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

    def submit(self, line: str, context: list[str], msg_id: int) -> None:
        """提交一則翻譯。context 為提交當下的快照，重試時沿用同一份、不隨後續訊息漂移。

        「檢查 _stop → 讀取 executor → 呼叫 executor.submit()」整段都在 _lock 內完成，
        與 resize()／shutdown() 交換／關閉 executor 的臨界區互斥；否則會出現 submit()
        讀到即將被換掉的 executor、鎖外才真正送件，而 executor 已 shutdown 導致
        RuntimeError 直接炸出呼叫端，且 in_flight 已加計卻永遠等不到 _work 遞減的競速。
        """
        with self._lock:
            if self._stop.is_set():
                return
            self._in_flight += 1
            try:
                future = self._executor.submit(self._work, line, context, msg_id)
            except RuntimeError:
                # 理論上已被上面同一把鎖排除；留著防呆，避免未來重構重新打開競速窗口。
                self._in_flight -= 1
                print(f"[translate] submit rejected, executor already shut down: {line}",
                      file=sys.stderr)
                return
        future.add_done_callback(self._on_future_done)

    def _on_future_done(self, future) -> None:
        """work item 若在真正開始執行前就被 shutdown(cancel_futures=True) 取消，
        _work 永遠不會跑、它的 finally 也就不會遞減 in_flight——這裡補上那次遞減。
        _work 正常完成（成功或例外）時 future 不是 cancelled 狀態，這裡不重複計數。"""
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
        print(f"[translate] pool resized to {workers} workers", file=sys.stderr)

    def shutdown(self, wait: bool = False) -> None:
        """停止接受新工作並要求 worker 盡快收手。"""
        self._stop.set()
        with self._lock:
            executor = self._executor
        executor.shutdown(wait=wait, cancel_futures=True)

    def _work(self, line: str, context: list[str], msg_id: int) -> None:
        # 單次呼叫至多一次 return（成功／不可重試失敗／閘門關閉時放棄），
        # finally 因此每則工作恰好遞減一次 in_flight——與 _on_future_done
        # 對 cancelled future 的遞減互斥（見模組層筆記）。
        attempts = 0
        try:
            while not self._stop.is_set():
                if not self._wait_for_gate():
                    return          # 關閉中：放棄這則，不回報
                attempts += 1
                try:
                    translated = self._translator.translate_incoming(line, context)
                except TranslatorOffline as exc:
                    self._note_failure("offline", exc)
                    continue        # 該則留著重試，伺服器恢復就補上
                except TranslatorConfigError as exc:
                    self._note_failure("config", exc)
                    continue        # 等使用者修正設定後自動恢復
                except Exception as exc:
                    # 譯文被截斷或回傳格式異常：temperature=0 下重試必得同一結果，
                    # 直接放棄該則，讓 overlay 至少顯示原文而不是無聲消失。
                    reason = ("bad model output" if isinstance(exc, TranslatorBadOutput)
                              else "unexpected error")
                    print(f"[translate] line dropped after {attempts} attempt(s), "
                          f"{reason} ({exc}): {line}", file=sys.stderr)
                    self._on_result(msg_id, self._failed_notice)
                    return
                self._note_success()
                self._on_result(msg_id, translated)
                return
        finally:
            with self._lock:
                self._in_flight -= 1

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
        """推進全域退避閘門並記錄狀態。閘門是全域的——伺服器離線本就是全域事實，
        否則 N 個 worker 會以 N 倍速重打同一台掛掉的伺服器。"""
        with self._lock:
            if state == "config":
                delay = CONFIG_ERROR_INTERVAL
            else:
                steps = BACKOFF_STEPS
                delay = steps[min(self._backoff_index, len(steps) - 1)]
                self._backoff_index += 1
            self._gate_until = time.monotonic() + delay
            changed = self._error_state != state
            self._error_state = state
        if changed:
            print(f"[translate] provider {state} error: {exc}; "
                  f"retrying with backoff", file=sys.stderr)

    def _note_success(self) -> None:
        """任何一次成功都代表伺服器與設定已恢復：清狀態、重置退避、放開閘門。"""
        with self._lock:
            changed = self._error_state is not None
            self._error_state = None
            self._backoff_index = 0
            self._gate_until = 0.0
        if changed:
            print("[translate] recovered, resuming normal speed", file=sys.stderr)
