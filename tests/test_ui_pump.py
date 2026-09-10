"""drain_ui_queue：單一回呼拋錯不能讓佇列裡其餘回呼被跳過，也不能讓例外往外傳。
on_hotkey：只有遊戲在前景（或輸入框已開著）才把 input_box.show 排進佇列。"""
import queue

from src.main import drain_ui_queue


def test_raising_callback_does_not_propagate():
    q: queue.Queue = queue.Queue()

    def boom():
        raise ValueError("bad callback")

    q.put(boom)
    drain_ui_queue(q)  # 不應該拋出例外
    assert q.empty()


def test_later_callbacks_still_run_after_one_raises():
    q: queue.Queue = queue.Queue()
    calls = []

    def boom():
        raise RuntimeError("bad")

    def ok_one():
        calls.append("one")

    def ok_two():
        calls.append("two")

    q.put(boom)
    q.put(ok_one)
    q.put(ok_two)

    drain_ui_queue(q)

    assert calls == ["one", "two"]
    assert q.empty()


def test_empty_queue_is_a_noop():
    q: queue.Queue = queue.Queue()
    drain_ui_queue(q)  # 不應該拋出例外
    assert q.empty()


class _FakeInputBox:
    def __init__(self, is_open: bool):
        self.is_open = is_open

    def show(self) -> None:
        pass


def test_hotkey_opens_input_box_when_game_is_foreground(monkeypatch):
    import src.main as main
    monkeypatch.setattr(main, "foreground_exe",
                        lambda: r"C:\Wizard101\Bin\WizardGraphicalClient.exe")
    q = queue.Queue()
    box = _FakeInputBox(is_open=False)
    main.on_hotkey(box, q)
    assert q.get_nowait() == box.show


def test_hotkey_ignored_when_other_window_is_foreground(monkeypatch):
    import src.main as main
    monkeypatch.setattr(main, "foreground_exe", lambda: r"C:\Tools\notepad.exe")
    q = queue.Queue()
    main.on_hotkey(_FakeInputBox(is_open=False), q)
    assert q.empty()


def test_hotkey_ignored_when_foreground_unknown(monkeypatch):
    import src.main as main
    monkeypatch.setattr(main, "foreground_exe", lambda: None)
    q = queue.Queue()
    main.on_hotkey(_FakeInputBox(is_open=False), q)
    assert q.empty()


def test_hotkey_refocuses_open_input_box_regardless_of_foreground(monkeypatch):
    # 輸入框已開著（它自己就是前景）再按熱鍵：照舊對焦，不做前景檢查
    import src.main as main
    monkeypatch.setattr(main, "foreground_exe", lambda: r"C:\Tools\notepad.exe")
    q = queue.Queue()
    box = _FakeInputBox(is_open=True)
    main.on_hotkey(box, q)
    assert q.get_nowait() == box.show


# --- 熱鍵註冊：config.json 手改成不認得的鍵名不能讓程式無聲退出 ---
class _FakeKeyboard:
    def __init__(self, bad: str):
        self._bad = bad
        self.registered = []

    def add_hotkey(self, hotkey, callback):
        if hotkey == self._bad:
            raise ValueError(f"unknown key {hotkey}")
        self.registered.append(hotkey)
        return object()


def test_register_hotkey_falls_back_to_the_default_on_an_unknown_key(monkeypatch):
    from src import main
    from src.config import DEFAULT_CONFIG
    fake = _FakeKeyboard(bad="ctrl+nope")
    monkeypatch.setattr(main, "keyboard", fake)
    logged = []
    monkeypatch.setattr(main, "log", logged.append)
    handle, used = main.register_hotkey("ctrl+nope", lambda: None)
    assert handle is not None
    assert used == DEFAULT_CONFIG["hotkey"]
    assert fake.registered == [DEFAULT_CONFIG["hotkey"]]
    assert any("ctrl+nope" in line for line in logged)


def test_register_hotkey_uses_the_requested_key_when_valid(monkeypatch):
    from src import main
    fake = _FakeKeyboard(bad="")
    monkeypatch.setattr(main, "keyboard", fake)
    handle, used = main.register_hotkey("f8", lambda: None)
    assert used == "f8" and fake.registered == ["f8"]
