"""收訊翻譯工作池：以固定數量的 worker 平行翻譯，單則卡住不影響其他則。

呼叫端只負責提交（line, context, msg_id）與接收 on_result(msg_id, text)；
顯示順序不由完成順序決定——overlay 在提交當下就已佔好位置（見 reader_loop）。
"""
import sys
import threading
from concurrent.futures import ThreadPoolExecutor

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
        self._executor = ThreadPoolExecutor(max_workers=workers,
                                            thread_name_prefix="translate")

    @property
    def in_flight(self) -> int:
        """已提交但尚未回報結果的則數（供狀態指示判斷是否顯示『翻譯中…』）。"""
        with self._lock:
            return self._in_flight

    @property
    def error_state(self) -> str | None:
        """目前的翻譯錯誤狀態：None／"offline"／"config"（Task 5 起有實質值）。"""
        return None

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
        try:
            translated = self._translator.translate_incoming(line, context)
        except Exception as exc:
            print(f"[translate] line dropped, unexpected error ({exc}): {line}",
                  file=sys.stderr)
            self._on_result(msg_id, self._failed_notice)
        else:
            self._on_result(msg_id, translated)
        finally:
            with self._lock:
                self._in_flight -= 1
