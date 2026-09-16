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


def test_no_ops_without_pyi_splash(monkeypatch):
    monkeypatch.setattr(splash, "_splash", None)
    monkeypatch.setattr(splash, "_closed", False)
    splash.update("anything")   # 開發模式的常態，不該拋例外
    splash.close()
    assert splash.is_available() is False


def test_forwards_to_pyi_splash(monkeypatch):
    fake = _FakeSplash()
    monkeypatch.setattr(splash, "_splash", fake)
    monkeypatch.setattr(splash, "_closed", False)
    splash.update("Loading components...")
    splash.close()
    assert fake.texts == ["Loading components..."]
    assert fake.closed is True
    assert splash.is_available() is True


def test_update_after_close_is_ignored(monkeypatch):
    """關掉之後再更新不該再碰 pyi_splash —— 首次執行精靈那條路徑就會這樣走，
    真的送出去只會換來一行誤導人的失敗 log。"""
    fake = _FakeSplash()
    monkeypatch.setattr(splash, "_splash", fake)
    monkeypatch.setattr(splash, "_closed", False)
    splash.close()
    splash.update("Starting...")
    assert fake.texts == []


def test_close_is_idempotent(monkeypatch):
    fake = _FakeSplash()
    monkeypatch.setattr(splash, "_splash", fake)
    monkeypatch.setattr(splash, "_closed", False)
    splash.close()
    fake.closed = False        # 第二次呼叫若真的轉過去，這裡會被改回 True
    splash.close()
    assert fake.closed is False


def test_failures_are_swallowed(monkeypatch):
    monkeypatch.setattr(splash, "_splash", _FakeSplash(fail=True))
    monkeypatch.setattr(splash, "_closed", False)
    splash.update("x")   # ConnectionError 不該逃出去
    splash.close()       # RuntimeError 同理
