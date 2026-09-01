"""翻譯請求的總量閘：限制同時進行的請求數，與「哪一條佇列送出的」無關。

兩個 TranslationPool（玩家對話、系統訊息）各自管佇列隔離，總量由本閘統一把關——
`max_parallel_translations` 因此維持它字面的語意（同時進行的收訊翻譯則數），
而不是每條通道各一份。

不用 threading.Semaphore：上限可在設定視窗即時調整，而號誌容量調不了。
"""
import threading

_WAIT_SLICE = 0.5   # 分段等待的長度（秒）：讓 stop 能及時打斷，不必等到有額度才醒


class ConcurrencyGate:
    """可調上限的併發計數閘。`acquire` 成功的每一次都必須對應一次 `release`。"""

    def __init__(self, limit: int):
        self._limit = max(1, limit)
        self._in_use = 0
        self._cond = threading.Condition()

    @property
    def in_use(self) -> int:
        with self._cond:
            return self._in_use

    def acquire(self, stop: threading.Event) -> bool:
        """取得一個額度；額度滿時等待。`stop` 被設定即放棄並回傳 False（關閉中）。"""
        with self._cond:
            while not stop.is_set():
                if self._in_use < self._limit:
                    self._in_use += 1
                    return True
                self._cond.wait(_WAIT_SLICE)
            return False

    def release(self) -> None:
        with self._cond:
            self._in_use -= 1
            # notify_all 而非 notify：被叫醒的那一個若正好因 stop 而放棄（關閉中），
            # 剛空出來的額度就沒人接手，其餘等待者得再空等一個 _WAIT_SLICE。
            self._cond.notify_all()

    def set_limit(self, limit: int) -> None:
        """變更上限。調大時喚醒等待者；調小不收回已發出的額度——
        已在飛行中的請求跑完自然收斂，中途抽掉會讓 release 與 acquire 對不上。"""
        with self._cond:
            limit = max(1, limit)
            if limit == self._limit:
                return
            self._limit = limit
            self._cond.notify_all()
