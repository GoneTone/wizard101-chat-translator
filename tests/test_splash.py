"""啟動畫面包裝：沒有 pyi_splash 時要安靜不動，有的時候要確實轉呼叫過去。"""
from src import splash


class _FakeSplash:
    """假的 pyi_splash：記下收到的呼叫，或依 `fail` 拋出真實會遇到的例外。"""

    def __init__(self, fail=False):
        self.texts = []
        self.closed = False
        self._fail = fail

    def update_text(self, msg):
        if self._fail:
            raise ConnectionError("socket gone")
        self.texts.append(msg)

    def close(self):
        if self._fail:
            raise RuntimeError("this module is not initialized")
        self.closed = True


def _reset(monkeypatch, fake) -> None:
    """每個測試獨立重置模組狀態：`_pending_logs`／`_failed` 是全域可變狀態，
    上一個測試留下的殘值會讓斷言誤判成功或失敗；`monkeypatch.setattr` 收尾時
    自動還原，不必額外寫 teardown。"""
    monkeypatch.setattr(splash, "_splash", fake)
    monkeypatch.setattr(splash, "_closed", False)
    monkeypatch.setattr(splash, "_failed", False)
    monkeypatch.setattr(splash, "_pending_logs", [])


def test_phase_constants_match_the_literals_run_py_and_main_py_used_to_pass():
    """`tools/splash_progress.py` 在 build 時 import 這兩個常數、原封不動烤進 Tcl
    去比對 bootloader 回報的 status_text；`run.py`／`src/main.py` 也改成傳常數而不是
    字面值。這裡釘住常數本身的字面值——改了這裡卻沒對應更新 Tcl 那邊的比對邏輯，
    進度條會卡在解壓上限、永遠推不到後段，但不會有任何測試變紅，除非釘住這兩個值。
    """
    assert splash.PHASE_LOADING == "Loading components..."
    assert splash.PHASE_STARTING == "Starting..."


def test_no_ops_without_pyi_splash(monkeypatch):
    _reset(monkeypatch, None)
    splash.update("anything")   # 開發模式的常態，不該拋例外
    splash.close()
    assert splash.is_available() is False


def test_forwards_to_pyi_splash(monkeypatch):
    fake = _FakeSplash()
    _reset(monkeypatch, fake)
    splash.update("Loading components...")
    splash.close()
    assert fake.texts == ["Loading components..."]
    assert fake.closed is True
    assert splash.is_available() is True


def test_update_after_close_is_ignored(monkeypatch):
    """關掉之後再更新不該再碰 pyi_splash —— 首次執行精靈那條路徑就會這樣走，
    真的送出去只會換來一行誤導人的失敗 log。"""
    fake = _FakeSplash()
    _reset(monkeypatch, fake)
    splash.close()
    splash.update("Starting...")
    assert fake.texts == []


def test_close_is_idempotent(monkeypatch):
    fake = _FakeSplash()
    _reset(monkeypatch, fake)
    splash.close()
    fake.closed = False        # 第二次呼叫若真的轉過去，這裡會被改回 True
    splash.close()
    assert fake.closed is False


def test_failures_are_swallowed(monkeypatch):
    """update() 與 close() 各自的例外都要吞掉，但診斷不能跟著消失 —— 若兩個 except
    子句被偷懶改成 `pass`，例外一樣不會逃出去，這個測試會是唯一抓到差異的地方。"""
    _reset(monkeypatch, _FakeSplash(fail=True))
    splash.update("x")   # ConnectionError 不該逃出去
    splash.close()       # RuntimeError 同理

    assert any(m.startswith("[splash] update failed") for m in splash._pending_logs)
    assert any(m.startswith("[splash] close failed") for m in splash._pending_logs)
    assert splash.had_failure() is True


def test_drain_logs_flushes_buffer_then_switches_to_direct_logging(monkeypatch):
    """`run.py` 在 `redirect_output()` 之前呼叫 `update()` 時，`log()` 還寫不出去；
    `drain_logs()` 要把那段期間累積的訊息補寫出來，之後的訊息不再進 buffer、直接記 log。"""
    recorded = []
    monkeypatch.setattr(splash, "log", lambda msg: recorded.append(msg))
    monkeypatch.setattr(splash, "_pending_logs", ["[splash] update failed: buffered"])

    splash.drain_logs()
    assert recorded == ["[splash] update failed: buffered"]
    assert splash._pending_logs is None

    splash._record("[splash] close failed: direct")
    assert recorded == ["[splash] update failed: buffered", "[splash] close failed: direct"]

    splash.drain_logs()   # 重複呼叫安全：已經是 None 就不再重放
    assert recorded == ["[splash] update failed: buffered", "[splash] close failed: direct"]
