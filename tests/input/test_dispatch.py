"""全域熱鍵與遊戲聊天輸入框事件的分派：只有遊戲在前景（或輸入框已開著）才接手，
雙開時聊天框事件與 Ctrl+V 單行判定只認前景客戶端。"""
import queue

from src.input import dispatch


# --- 雙開：聊天框事件只認前景客戶端，Ctrl+V 單行判定看前景視窗 ---
class _FakeInputBox:
    def __init__(self, is_open=False):
        self.calls: list = []
        self.is_open = is_open

    def set_anchor(self, rect):
        self.calls.append(("anchor", rect))

    def clear_anchor(self):
        self.calls.append("clear_anchor")

    def retarget(self, hwnd):
        self.calls.append(("retarget", hwnd))

    def show(self):
        self.calls.append("show")

    def hide(self):
        self.calls.append("hide")


def _events(auto_show=True, foreground=0xA):
    box = _FakeInputBox()
    tracker = dispatch.ChatInputTracker()
    ev = dispatch.GameInputEvents({"auto_show_input": auto_show}, box, tracker,
                              foreground=lambda: foreground)
    return ev, box, tracker


def test_open_in_the_foreground_window_anchors_and_shows():
    ev, box, tracker = _events(foreground=0xA)
    ev.opened(0xA, (1, 2, 3, 4))
    assert box.calls == [("anchor", (1, 2, 3, 4)), ("retarget", 0xA), "show"]
    assert tracker.is_open(0xA)


def test_open_in_the_foreground_window_retargets_before_showing():
    # 框已開著、綁在別的客戶端：切過來開聊天框要先改綁再呼出，不能打進舊客戶端
    ev, box, tracker = _events(foreground=0xB)
    ev.opened(0xB, (1, 2, 3, 4))
    assert box.calls.index(("retarget", 0xB)) < box.calls.index("show")


def test_open_in_a_background_window_is_tracked_but_not_shown():
    # 使用者正在玩 B，A 的聊天框開了：不彈框、不改錨點、不改標，但記得 A 開著（貼上判定要用）
    ev, box, tracker = _events(foreground=0xB)
    ev.opened(0xA, (1, 2, 3, 4))
    assert box.calls == []
    assert tracker.is_open(0xA)


def test_close_from_another_window_does_not_hide_the_box():
    ev, box, tracker = _events(foreground=0xB)
    ev.opened(0xB, None)
    ev.opened(0xA, None)     # A 在背景開著
    ev.closed(0xA)           # A 關了：不能收掉為 B 呼出的框
    assert "hide" not in box.calls
    assert not tracker.is_open(0xA) and tracker.is_open(0xB)
    ev.closed(0xB)
    assert box.calls[-2:] == ["clear_anchor", "hide"]


def test_auto_show_off_still_tracks_and_anchors():
    ev, box, tracker = _events(auto_show=False, foreground=0xA)
    ev.opened(0xA, (1, 2, 3, 4))
    ev.closed(0xA)
    assert box.calls == [("anchor", (1, 2, 3, 4)), ("retarget", 0xA), "clear_anchor"]


def test_paste_single_line_follows_the_foreground_window(monkeypatch):
    # 在 B 貼多行、只有 B 的聊天框開著：要走單行；A 開著、前景是 B 且 B 沒開：多行
    seen = []
    monkeypatch.setattr(dispatch, "paste_clipboard",
                        lambda hwnd, delay, single_line: seen.append((hwnd, single_line)))
    monkeypatch.setattr(dispatch.win32gui, "GetForegroundWindow", lambda: 0xB)
    tracker = dispatch.ChatInputTracker()
    tracker.opened(0xB)
    dispatch.on_paste_hotkey({"type_delay": 0}, tracker).join()
    tracker.closed(0xB)
    tracker.opened(0xA)
    dispatch.on_paste_hotkey({"type_delay": 0}, tracker).join()
    assert seen == [(0xB, True), (0xB, False)]


def test_on_hotkey_retargets_before_showing_when_game_is_foreground(monkeypatch):
    # 雙開：框開著、綁在客戶端 A，使用者切到 B 按熱鍵 → 先改綁再呼出
    monkeypatch.setattr(dispatch, "foreground_exe",
                        lambda: r"C:\Wizard101\Bin\WizardGraphicalClient.exe")
    monkeypatch.setattr(dispatch.win32gui, "GetForegroundWindow", lambda: 0xB)
    box = _FakeInputBox(is_open=True)
    ui_queue: queue.Queue = queue.Queue()
    dispatch.on_hotkey(box, ui_queue)
    _drain(ui_queue)
    assert box.calls == [("retarget", 0xB), "show"]


def test_on_hotkey_only_refocuses_when_the_box_itself_is_foreground(monkeypatch):
    # 前景讀不到遊戲（框自己是前景）：只重新對焦，不改動 target_hwnd
    monkeypatch.setattr(dispatch, "foreground_exe", lambda: None)
    box = _FakeInputBox(is_open=True)
    ui_queue: queue.Queue = queue.Queue()
    dispatch.on_hotkey(box, ui_queue)
    _drain(ui_queue)
    assert box.calls == ["show"]


def test_on_hotkey_ignored_when_box_closed_and_game_not_foreground(monkeypatch):
    monkeypatch.setattr(dispatch, "foreground_exe", lambda: None)
    box = _FakeInputBox(is_open=False)
    ui_queue: queue.Queue = queue.Queue()
    dispatch.on_hotkey(box, ui_queue)
    assert ui_queue.empty()


def test_on_hotkey_does_not_log_ignored_when_box_open_and_foreground_is_not_the_game(monkeypatch):
    # R1：框已開著、前景是別的視窗（如瀏覽器）—— 只是重新對焦，不該印出 ignored 的誤導 log
    logged = []
    monkeypatch.setattr(dispatch, "log", lambda msg: logged.append(msg))
    monkeypatch.setattr(dispatch, "foreground_exe", lambda: r"C:\Tools\notepad.exe")
    box = _FakeInputBox(is_open=True)
    ui_queue: queue.Queue = queue.Queue()
    dispatch.on_hotkey(box, ui_queue)
    _drain(ui_queue)
    assert box.calls == ["show"]
    assert not any("ignored" in msg for msg in logged)


def _drain(q: queue.Queue) -> None:
    while True:
        try:
            q.get_nowait()()
        except queue.Empty:
            break


def test_hotkey_opens_input_box_when_game_is_foreground(monkeypatch):
    monkeypatch.setattr(dispatch, "foreground_exe",
                        lambda: r"C:\Wizard101\Bin\WizardGraphicalClient.exe")
    monkeypatch.setattr(dispatch.win32gui, "GetForegroundWindow", lambda: 0xA)
    q = queue.Queue()
    box = _FakeInputBox(is_open=False)
    assert dispatch.on_hotkey(box, q) is True
    _drain(q)
    assert box.calls == [("retarget", 0xA), "show"]


def test_hotkey_ignored_when_other_window_is_foreground(monkeypatch):
    monkeypatch.setattr(dispatch, "foreground_exe", lambda: r"C:\Tools\notepad.exe")
    q = queue.Queue()
    assert dispatch.on_hotkey(_FakeInputBox(is_open=False), q) is False
    assert q.empty()


def test_hotkey_ignored_when_foreground_unknown(monkeypatch):
    monkeypatch.setattr(dispatch, "foreground_exe", lambda: None)
    q = queue.Queue()
    assert dispatch.on_hotkey(_FakeInputBox(is_open=False), q) is False
    assert q.empty()


def test_hotkey_refocuses_open_input_box_regardless_of_foreground(monkeypatch):
    # 輸入框已開著（它自己就是前景）再按熱鍵：照舊對焦、不改 target_hwnd，不做前景檢查
    monkeypatch.setattr(dispatch, "foreground_exe", lambda: r"C:\Tools\notepad.exe")
    q = queue.Queue()
    box = _FakeInputBox(is_open=True)
    assert dispatch.on_hotkey(box, q) is True
    _drain(q)
    assert box.calls == ["show"]


# --- 啟動時的熱鍵撞名防護：region_hotkey 與 hotkey 相同就不註冊 ---
def test_region_hotkey_to_register_returns_none_on_collision():
    cfg = {"hotkey": "ctrl+shift+space", "region_hotkey": "ctrl+shift+space"}
    assert dispatch.region_hotkey_to_register(cfg) is None


def test_region_hotkey_to_register_returns_the_hotkey_when_different():
    cfg = {"hotkey": "f8", "region_hotkey": "ctrl+shift+space"}
    assert dispatch.region_hotkey_to_register(cfg) == "ctrl+shift+space"


# --- 框選熱鍵：選取層開著就取消、遊戲在前景才開始框選 ---
class _FakeRegionFlow:
    def __init__(self, is_selecting: bool):
        self.is_selecting = is_selecting
        self.toggled = []

    def toggle(self, hwnd):
        self.toggled.append(hwnd)


def test_region_hotkey_cancels_when_the_selector_is_already_open():
    q = queue.Queue()
    flow = _FakeRegionFlow(is_selecting=True)
    assert dispatch.on_region_hotkey(flow, q) is True
    q.get_nowait()()
    assert flow.toggled == [0]


def test_region_hotkey_starts_a_selection_when_game_is_foreground(monkeypatch):
    monkeypatch.setattr(dispatch, "foreground_exe",
                        lambda: r"C:\Wizard101\Bin\WizardGraphicalClient.exe")
    monkeypatch.setattr(dispatch.win32gui, "GetForegroundWindow", lambda: 0x1234)
    q = queue.Queue()
    flow = _FakeRegionFlow(is_selecting=False)
    assert dispatch.on_region_hotkey(flow, q) is True
    q.get_nowait()()
    assert flow.toggled == [0x1234]


def test_region_hotkey_ignored_when_other_window_is_foreground(monkeypatch):
    monkeypatch.setattr(dispatch, "foreground_exe", lambda: r"C:\Tools\notepad.exe")
    q = queue.Queue()
    flow = _FakeRegionFlow(is_selecting=False)
    assert dispatch.on_region_hotkey(flow, q) is False
    assert q.empty()
    assert flow.toggled == []


# --- Ctrl+V 攔截：只在設定開啟且遊戲在前景時接手 ---
def test_paste_intercepted_only_when_enabled_and_game_is_foreground(monkeypatch):
    game = r"C:\Wizard101\Bin\WizardGraphicalClient.exe"
    monkeypatch.setattr(dispatch, "foreground_exe", lambda: game)
    assert dispatch.should_intercept_paste({"paste_hotkey": True}) is True
    assert dispatch.should_intercept_paste({"paste_hotkey": False}) is False
    monkeypatch.setattr(dispatch, "foreground_exe", lambda: r"C:\Tools\chrome.exe")
    assert dispatch.should_intercept_paste({"paste_hotkey": True}) is False
    monkeypatch.setattr(dispatch, "foreground_exe", lambda: None)
    assert dispatch.should_intercept_paste({"paste_hotkey": True}) is False


def test_paste_hotkey_types_single_line_only_while_the_game_chat_box_is_open(monkeypatch):
    pasted = []
    monkeypatch.setattr(dispatch, "paste_clipboard",
                        lambda hwnd, delay, single_line: pasted.append((hwnd, delay, single_line)))
    monkeypatch.setattr(dispatch.win32gui, "GetForegroundWindow", lambda: 0x77)
    tracker = dispatch.ChatInputTracker()
    cfg = {"type_delay": 0.03}
    dispatch.on_paste_hotkey(cfg, tracker).join(timeout=5)
    tracker.opened(0x77)
    dispatch.on_paste_hotkey(cfg, tracker).join(timeout=5)
    assert pasted == [(0x77, 0.03, False), (0x77, 0.03, True)]
