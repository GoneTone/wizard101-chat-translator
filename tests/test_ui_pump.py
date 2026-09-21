"""drain_ui_queue：單一回呼拋錯不能讓佇列裡其餘回呼被跳過，也不能讓例外往外傳。
on_hotkey：只有遊戲在前景（或輸入框已開著）才把 input_box.show 排進佇列；
遊戲在前景時一併改綁 target_hwnd（雙開切客戶端）。"""
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
        self.calls: list = []

    def retarget(self, hwnd) -> None:
        self.calls.append(("retarget", hwnd))

    def show(self) -> None:
        self.calls.append("show")


def _drain(q: queue.Queue) -> None:
    while True:
        try:
            q.get_nowait()()
        except queue.Empty:
            break


def test_hotkey_opens_input_box_when_game_is_foreground(monkeypatch):
    import src.main as main
    monkeypatch.setattr(main, "foreground_exe",
                        lambda: r"C:\Wizard101\Bin\WizardGraphicalClient.exe")
    monkeypatch.setattr(main.win32gui, "GetForegroundWindow", lambda: 0xA)
    q = queue.Queue()
    box = _FakeInputBox(is_open=False)
    main.on_hotkey(box, q)
    _drain(q)
    assert box.calls == [("retarget", 0xA), "show"]


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
    # 輸入框已開著（它自己就是前景）再按熱鍵：照舊對焦、不改 target_hwnd，不做前景檢查
    import src.main as main
    monkeypatch.setattr(main, "foreground_exe", lambda: r"C:\Tools\notepad.exe")
    q = queue.Queue()
    box = _FakeInputBox(is_open=True)
    main.on_hotkey(box, q)
    _drain(q)
    assert box.calls == ["show"]


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


def test_register_hotkey_falls_back_to_the_given_fallback_on_an_unknown_key(monkeypatch):
    # 區域熱鍵壞掉時要退回自己的預設值，不能撞上輸入框熱鍵的預設值
    from src import main
    fake = _FakeKeyboard(bad="ctrl+nope")
    monkeypatch.setattr(main, "keyboard", fake)
    handle, used = main.register_hotkey("ctrl+nope", lambda: None,
                                        fallback="ctrl+shift+space")
    assert used == "ctrl+shift+space"
    assert fake.registered == ["ctrl+shift+space"]


# --- 啟動時的熱鍵撞名防護：region_hotkey 與 hotkey 相同就不註冊 ---
def test_region_hotkey_to_register_returns_none_on_collision():
    import src.main as main
    cfg = {"hotkey": "ctrl+shift+space", "region_hotkey": "ctrl+shift+space"}
    assert main.region_hotkey_to_register(cfg) is None


def test_region_hotkey_to_register_returns_the_hotkey_when_different():
    import src.main as main
    cfg = {"hotkey": "f8", "region_hotkey": "ctrl+shift+space"}
    assert main.region_hotkey_to_register(cfg) == "ctrl+shift+space"


# --- 框選熱鍵：選取層開著就取消、遊戲在前景才開始框選 ---
class _FakeRegionFlow:
    def __init__(self, is_selecting: bool):
        self.is_selecting = is_selecting
        self.toggled = []

    def toggle(self, hwnd):
        self.toggled.append(hwnd)


def test_region_hotkey_cancels_when_the_selector_is_already_open():
    import src.main as main
    q = queue.Queue()
    flow = _FakeRegionFlow(is_selecting=True)
    main.on_region_hotkey(flow, q)
    q.get_nowait()()
    assert flow.toggled == [0]


def test_region_hotkey_starts_a_selection_when_game_is_foreground(monkeypatch):
    import src.main as main
    monkeypatch.setattr(main, "foreground_exe",
                        lambda: r"C:\Wizard101\Bin\WizardGraphicalClient.exe")
    monkeypatch.setattr(main.win32gui, "GetForegroundWindow", lambda: 0x1234)
    q = queue.Queue()
    flow = _FakeRegionFlow(is_selecting=False)
    main.on_region_hotkey(flow, q)
    q.get_nowait()()
    assert flow.toggled == [0x1234]


def test_region_hotkey_ignored_when_other_window_is_foreground(monkeypatch):
    import src.main as main
    monkeypatch.setattr(main, "foreground_exe", lambda: r"C:\Tools\notepad.exe")
    q = queue.Queue()
    flow = _FakeRegionFlow(is_selecting=False)
    main.on_region_hotkey(flow, q)
    assert q.empty()
    assert flow.toggled == []


# --- Ctrl+V 攔截：只在設定開啟且遊戲在前景時接手 ---
def test_paste_intercepted_only_when_enabled_and_game_is_foreground(monkeypatch):
    import src.main as main
    game = r"C:\Wizard101\Bin\WizardGraphicalClient.exe"
    monkeypatch.setattr(main, "foreground_exe", lambda: game)
    assert main.should_intercept_paste({"paste_hotkey": True}) is True
    assert main.should_intercept_paste({"paste_hotkey": False}) is False
    monkeypatch.setattr(main, "foreground_exe", lambda: r"C:\Tools\chrome.exe")
    assert main.should_intercept_paste({"paste_hotkey": True}) is False
    monkeypatch.setattr(main, "foreground_exe", lambda: None)
    assert main.should_intercept_paste({"paste_hotkey": True}) is False


def test_paste_hotkey_types_single_line_only_while_the_game_chat_box_is_open(monkeypatch):
    import src.main as main
    pasted = []
    monkeypatch.setattr(main, "paste_clipboard",
                        lambda hwnd, delay, single_line: pasted.append((hwnd, delay, single_line)))
    monkeypatch.setattr(main.win32gui, "GetForegroundWindow", lambda: 0x77)
    tracker = main.ChatInputTracker()
    cfg = {"type_delay": 0.03}
    main.on_paste_hotkey(cfg, tracker).join(timeout=5)
    tracker.opened(0x77)
    main.on_paste_hotkey(cfg, tracker).join(timeout=5)
    assert pasted == [(0x77, 0.03, False), (0x77, 0.03, True)]
