"""掛入路徑：連線失敗的分類、版本不相容判定、等待 hook 就緒的逾時、掛入接線（不需遊戲）。"""

from src.reader.mem_reader import (
    GameAccessDenied,
    GameNotRunning,
    GameVersionMismatch,
    WizChatReader,
    is_version_mismatch,
)
from src.reader.process import is_game_process_path


# --- 連線失敗的分類（不需遊戲：以假 ClientHandler 注入例外）---
class _FailingHandler:
    """get_new_clients 一律丟出指定例外；close 供 _teardown 呼叫。"""

    def __init__(self, exc):
        self._exc = exc

    def get_new_clients(self):
        raise self._exc

    async def close(self):
        pass


def _reader_failing_to_open(monkeypatch, exc):
    """讓 _connect 走到 get_new_clients 就丟出 exc 的 reader。"""
    import wizwalker

    from src.reader import mem_reader
    monkeypatch.setattr(mem_reader, "detect_install_path", lambda: None)
    monkeypatch.setattr(wizwalker, "ClientHandler", lambda **kw: _FailingHandler(exc))
    return WizChatReader()


def test_open_process_denied_raises_access_denied(monkeypatch):
    # 遊戲以較高權限執行時 pymem 開不了 handle：要能與「找不到遊戲」分辨開來
    from pymem.exception import CouldNotOpenProcess
    r = _reader_failing_to_open(monkeypatch, CouldNotOpenProcess(4321))
    try:
        r._connect()
        raise AssertionError("應丟 GameAccessDenied")
    except GameAccessDenied as exc:
        assert "4321" in str(exc)


def test_access_denied_is_a_game_not_running():
    # 繼承既有例外，上層的退避重連照舊生效，只在文案上分流
    assert issubclass(GameAccessDenied, GameNotRunning)


def test_other_connect_failure_stays_game_not_running(monkeypatch):
    r = _reader_failing_to_open(monkeypatch, RuntimeError("boom"))
    try:
        r._connect()
        raise AssertionError("應丟 GameNotRunning")
    except GameAccessDenied:
        raise AssertionError("非權限錯誤不得歸類為權限不足") from None
    except GameNotRunning:
        pass


def test_failed_connect_closes_event_loop(monkeypatch):
    # 連線失敗每 poll_interval 重試一輪，沒收掉 loop 會一輪洩漏一個
    r = _reader_failing_to_open(monkeypatch, RuntimeError("boom"))
    try:
        r._connect()
    except GameNotRunning:
        pass
    assert r._loop is None


# --- 掛入點與遊戲版本對不上的分類（純函式，不需遊戲）---
def test_pattern_failures_are_classified_as_version_mismatch():
    from wizwalker.errors import PatternFailed, PatternMultipleResults
    assert is_version_mismatch(PatternFailed(b"\x90\x48"))
    assert is_version_mismatch(PatternMultipleResults("got 2 results"))


def test_hook_ready_timeout_is_classified_as_version_mismatch():
    # pattern 掃得到、hook 也寫進去了，卻遲遲沒被觸發＝那段程式碼已不在執行路徑
    assert is_version_mismatch(TimeoutError("root window hook did not fire"))


def test_unrelated_connect_failures_are_not_version_mismatch():
    assert not is_version_mismatch(RuntimeError("boom"))
    assert not is_version_mismatch(OSError("nope"))


def test_version_mismatch_is_a_game_not_running():
    # 繼承既有例外，上層的退避重連照舊生效，只在文案上分流
    assert issubclass(GameVersionMismatch, GameNotRunning)


# --- 等待 hook 就緒：逾時而非無限等 ---
class _StubHookHandler:
    """read_current_root_window_base 依序回傳 values（例外物件即拋出，耗盡後一律回 0）。"""

    def __init__(self, values=(), activate_exc=None):
        self.values = list(values)
        self.activate_exc = activate_exc
        self.activate_kwargs = None
        self.process = type("P", (), {"base_address": 0x400000})()

    async def activate_root_window_hook(self, **kwargs):
        self.activate_kwargs = kwargs
        if self.activate_exc is not None:
            raise self.activate_exc

    async def read_current_root_window_base(self):
        value = self.values.pop(0) if self.values else 0
        if isinstance(value, Exception):
            raise value
        return value


class _StubClient:
    process_id = 4321

    def __init__(self, hook_handler):
        self.hook_handler = hook_handler
        self._pymem = type("M", (), {"base_address": 0x400000})()


def _waiting_reader(hook_handler):
    """已連上假 client、只差等待 hook 就緒的 reader。"""
    import asyncio
    r = WizChatReader()
    r._loop = asyncio.new_event_loop()
    r._client = _StubClient(hook_handler)
    return r


def test_wait_root_window_ready_returns_once_the_address_is_set(monkeypatch):
    from src.reader import mem_reader
    monkeypatch.setattr(mem_reader, "HOOK_READY_POLL", 0.0)
    r = _waiting_reader(_StubHookHandler([0, 0, 0x1234]))
    r._wait_root_window_ready()  # 不拋即通過


def test_wait_root_window_ready_ignores_transient_read_errors(monkeypatch):
    # hook 剛寫入時位址還沒有效，讀取失敗是常態，不該當成不相容
    from src.reader import mem_reader
    monkeypatch.setattr(mem_reader, "HOOK_READY_POLL", 0.0)
    r = _waiting_reader(_StubHookHandler([RuntimeError("read failed"), 0x1234]))
    r._wait_root_window_ready()


def test_wait_root_window_ready_times_out_instead_of_hanging_forever(monkeypatch):
    from src.reader import mem_reader
    monkeypatch.setattr(mem_reader, "HOOK_READY_TIMEOUT", 0.05)
    monkeypatch.setattr(mem_reader, "HOOK_READY_POLL", 0.0)
    r = _waiting_reader(_StubHookHandler())  # 位址永遠是 0
    try:
        r._wait_root_window_ready()
        raise AssertionError("應丟 TimeoutError")
    except TimeoutError:
        pass


# --- 掛入路徑的接線 ---
class _StubHandler:
    """get_new_clients 回傳固定的假 client；close 供 _teardown 呼叫。"""

    def __init__(self, client):
        self._client = client

    def get_new_clients(self):
        return [self._client]

    async def close(self):
        pass


def _connecting_reader(monkeypatch, tmp_path, hook_handler):
    """讓 _connect 走完整條掛入路徑的 reader（狀態檔改寫進 tmp_path）。"""
    import wizwalker

    from src.reader import hook_state, mem_reader
    monkeypatch.setattr(mem_reader, "detect_install_path", lambda: None)
    monkeypatch.setattr(mem_reader, "HOOK_READY_POLL", 0.0)
    monkeypatch.setattr(hook_state, "STATE_DIR", tmp_path)
    client = _StubClient(hook_handler)
    monkeypatch.setattr(wizwalker, "ClientHandler", lambda **kw: _StubHandler(client))
    return WizChatReader()


def test_pattern_failure_while_attaching_raises_version_mismatch(monkeypatch, tmp_path):
    from wizwalker.errors import PatternFailed
    r = _connecting_reader(monkeypatch, tmp_path,
                           _StubHookHandler(activate_exc=PatternFailed(b"\x90")))
    try:
        r._connect()
        raise AssertionError("應丟 GameVersionMismatch")
    except GameVersionMismatch:
        pass


def test_hook_never_firing_raises_version_mismatch(monkeypatch, tmp_path):
    from src.reader import mem_reader
    monkeypatch.setattr(mem_reader, "HOOK_READY_TIMEOUT", 0.05)
    r = _connecting_reader(monkeypatch, tmp_path, _StubHookHandler())
    try:
        r._connect()
        raise AssertionError("應丟 GameVersionMismatch")
    except GameVersionMismatch:
        pass


def test_other_attach_failure_stays_game_not_running(monkeypatch, tmp_path):
    r = _connecting_reader(monkeypatch, tmp_path,
                           _StubHookHandler(activate_exc=RuntimeError("boom")))
    try:
        r._connect()
        raise AssertionError("應丟 GameNotRunning")
    except GameVersionMismatch:
        raise AssertionError("非 pattern／逾時的失敗不得歸類為版本不相容") from None
    except GameNotRunning:
        pass


def test_hook_state_is_saved_before_waiting_for_the_hook_to_fire(monkeypatch, tmp_path):
    """逾時被硬砍也要修得回來：狀態檔必須在等待就緒之前就寫好。"""
    from src.reader import hook_state

    saved_when_waiting = []

    class _Watching(_StubHookHandler):
        async def read_current_root_window_base(self):
            saved_when_waiting.append(hook_state._state_path(4321).exists())
            return 0x1234

    handler = _Watching()
    handler._autobot_address = 0x500000
    handler._original_autobot_bytes = b"\x90" * 8
    r = _connecting_reader(monkeypatch, tmp_path, handler)
    r._connect()
    assert saved_when_waiting and saved_when_waiting[0], \
        "等待 hook 就緒時，還原狀態檔必須已經存在"


def test_attach_does_not_use_wizwalkers_unbounded_wait(monkeypatch, tmp_path):
    # wizwalker 內建的 wait_for_ready 預設無限等且不可中斷，一律自己等
    handler = _StubHookHandler([0x1234])
    r = _connecting_reader(monkeypatch, tmp_path, handler)
    r._connect()
    assert handler.activate_kwargs == {"wait_for_ready": False}


def test_is_game_process_path_matches_game_exe():
    assert is_game_process_path(r"C:\Games\Wizard101\Bin\WizardGraphicalClient.exe")
    assert is_game_process_path(r"c:\games\wizard101\bin\wizardgraphicalclient.exe")


def test_is_game_process_path_rejects_other_or_missing():
    assert not is_game_process_path(r"C:\Program Files\Mozilla Firefox\firefox.exe")
    assert not is_game_process_path(r"C:\Games\WizardGraphicalClient.exe.bak")
    assert not is_game_process_path("")
    assert not is_game_process_path(None)


# --- find_game_window：標題列按鈕路徑，遊戲不在前景時靠列舉找 ---
def _fake_enum_windows(visible: dict[int, int]):
    """假的 win32gui.EnumWindows：對 visible（hwnd → pid）裡的每個 hwnd 呼叫一次
    callback，其餘 hwnd（模擬看不見的視窗）不在字典內。
    `find_game_window` 內的 `import win32gui`／`import win32process` 拿到的是同一份
    已載入的模組物件，monkeypatch 真正的模組即可影響它。"""
    def enum_windows(callback, extra):
        for hwnd in visible:
            callback(hwnd, extra)
    return enum_windows


def test_find_game_window_returns_the_first_matching_visible_window(monkeypatch):
    import win32gui
    import win32process

    from src.reader import process

    pids = {1: 100, 2: 200, 3: 300}
    monkeypatch.setattr(win32gui, "EnumWindows", _fake_enum_windows(pids))
    monkeypatch.setattr(win32gui, "IsWindowVisible", lambda hwnd: True)
    monkeypatch.setattr(win32process, "GetWindowThreadProcessId",
                        lambda hwnd: (0, pids[hwnd]))
    paths = {100: r"C:\Program Files\Mozilla Firefox\firefox.exe",
            200: r"C:\Games\Wizard101\Bin\WizardGraphicalClient.exe",
            300: r"C:\Games\Wizard101\Bin\WizardGraphicalClient.exe"}
    monkeypatch.setattr(process, "process_exe_path", lambda pid: paths[pid])

    assert process.find_game_window() == 2


def test_find_game_window_returns_none_when_the_game_is_not_running(monkeypatch):
    import win32gui
    import win32process

    from src.reader import process

    pids = {1: 100, 2: 200}
    monkeypatch.setattr(win32gui, "EnumWindows", _fake_enum_windows(pids))
    monkeypatch.setattr(win32gui, "IsWindowVisible", lambda hwnd: True)
    monkeypatch.setattr(win32process, "GetWindowThreadProcessId",
                        lambda hwnd: (0, pids[hwnd]))
    monkeypatch.setattr(process, "process_exe_path",
                        lambda pid: r"C:\Program Files\Mozilla Firefox\firefox.exe")

    assert process.find_game_window() is None


def test_find_game_window_skips_invisible_windows(monkeypatch):
    import win32gui
    import win32process

    from src.reader import process

    monkeypatch.setattr(win32gui, "EnumWindows", _fake_enum_windows({1: 200}))
    monkeypatch.setattr(win32gui, "IsWindowVisible", lambda hwnd: False)
    monkeypatch.setattr(win32process, "GetWindowThreadProcessId", lambda hwnd: (0, 200))
    monkeypatch.setattr(process, "process_exe_path",
                        lambda pid: r"C:\Games\Wizard101\Bin\WizardGraphicalClient.exe")

    assert process.find_game_window() is None


def test_find_game_window_ignores_a_single_window_query_failure(monkeypatch):
    """單一視窗查詢失敗（權限、視窗剛消失）不該中斷整輪列舉，後面還找得到遊戲。"""
    import win32gui
    import win32process

    from src.reader import process

    monkeypatch.setattr(win32gui, "EnumWindows", _fake_enum_windows({1: 100, 2: 200}))
    monkeypatch.setattr(win32gui, "IsWindowVisible", lambda hwnd: True)

    def boom_or_pid(hwnd):
        if hwnd == 1:
            raise RuntimeError("window vanished")
        return (0, 200)

    monkeypatch.setattr(win32process, "GetWindowThreadProcessId", boom_or_pid)
    monkeypatch.setattr(process, "process_exe_path",
                        lambda pid: r"C:\Games\Wizard101\Bin\WizardGraphicalClient.exe")

    assert process.find_game_window() == 2


# --- 修復殘留 hook：寫回失敗與基址不符都要留下可追的 log ---
class _FakeHookHandler:
    def __init__(self, fail_at: set[int]):
        self._fail_at = fail_at
        self.written = []

    async def write_bytes(self, addr, data):
        if addr in self._fail_at:
            raise OSError(f"write failed at {addr:#x}")
        self.written.append((addr, data))


class _FakeClient:
    def __init__(self, base: int, fail_at: set[int] = frozenset()):
        self.hook_handler = _FakeHookHandler(set(fail_at))
        self._pymem = type("PM", (), {"base_address": base})()


def _repairing_reader(monkeypatch, tmp_path, base, fail_at=()):
    import asyncio

    from src.reader import hook_state, mem_reader
    monkeypatch.setattr(hook_state, "STATE_DIR", tmp_path)
    logged = []
    monkeypatch.setattr(mem_reader, "log", logged.append)
    r = WizChatReader()
    r._loop = asyncio.new_event_loop()
    r._client = _FakeClient(base, fail_at)
    return r, logged


def test_repair_reports_write_failures_instead_of_claiming_success(monkeypatch, tmp_path):
    from src.reader import hook_state
    r, logged = _repairing_reader(monkeypatch, tmp_path, base=0x1000, fail_at={0x3000})
    hook_state.save_state(77, 0x1000, [(0x2000, b"\xab"), (0x3000, b"\xcd")])
    r._repair_leaked_hooks(77)
    assert r._client.hook_handler.written == [(0x2000, b"\xab")]
    assert any("repaired" in line and "failed=1" in line for line in logged)
    assert hook_state.load_state(77) == (None, [])


def test_repair_skips_and_logs_a_stale_state_file(monkeypatch, tmp_path):
    from src.reader import hook_state
    r, logged = _repairing_reader(monkeypatch, tmp_path, base=0x9000)
    hook_state.save_state(77, 0x1000, [(0x2000, b"\xab")])
    r._repair_leaked_hooks(77)
    assert r._client.hook_handler.written == []
    assert any("stale" in line and "0x9000" in line for line in logged)
    assert hook_state.load_state(77) == (None, [])


def test_hook_state_save_failure_is_logged(monkeypatch, tmp_path):
    from src.reader import hook_state
    r, logged = _repairing_reader(monkeypatch, tmp_path, base=0x1000)
    r._client.hook_handler._active_hooks = {}

    def boom(pid, base, ops):
        raise OSError("disk full")

    monkeypatch.setattr(hook_state, "save_state", boom)
    r._save_hook_state(77)
    assert any("hook state" in line and "disk full" in line for line in logged)
