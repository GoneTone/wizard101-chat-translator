# 多客戶端（雙開）收訊 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 同時開多個遊戲客戶端時每個客戶端的聊天都被翻譯、疊加視窗以帶圈編號標示來源，發話／Ctrl+V／自動呼出輸入框都對準使用者當下面對的客戶端；單開外觀與行為完全不變。

**Architecture:** 一條 supervisor 執行緒定期列舉遊戲視窗，每個視窗配一個編號並起一條 `reader_loop` 執行緒；`WizChatReader` 改成綁定單一視窗 handle。各執行緒只向共用的 `StatusBoard` 回報自己的狀態，board 彙整後才推 overlay；翻譯池、快取、`ui_queue`、overlay 共用，翻譯上下文每客戶端一份。

**Tech Stack:** Python 3.14、wizwalker（LaurenzNotHere fork）、pywin32、tkinter、pytest；以 `uv run` 執行。

**Spec:** `docs/superpowers/specs/2026-09-21-multi-client-design.md`

## Global Constraints

- 回答、註解、docstring、UI 文字用繁體中文（台灣）與全形標點；log 訊息一律英文、不記金鑰；commit message 英文 conventional commits。
- 註解預設一行、只寫 WHY；模組與公開函式一行 docstring。
- 語言不可寫死；本案不碰翻譯。
- 只掛 root_window hook；不新增任何 hook。
- 不新增任何 config 欄位。
- 每個 task 結束前：`uv run ruff check src tests tools` 零錯誤、該 task 相關測試通過；整套 `uv run pytest` 只在最後一個 task 跑一次（避免打斷正在執行的遊戲，見專案記憶）。
- 測試不可搶焦點、不可開真實視窗到螢幕上（conftest 已停放視窗）；不得對真實遊戲掛 hook。
- 不主動 `git push`。

## Review Focus

1. **第二個客戶端還在更新器／登入畫面時**，第一個客戶端的收訊與狀態列不得受影響（不跳「找不到遊戲」橫幅）。→ Task 3 `test_transient_failure_on_one_slot_is_hidden_while_another_listens`。
2. **兩個客戶端幾乎同時關掉、再開一個新的**，新的必須拿到編號 1，且既有訊息的標記不消失。→ Task 6 `test_freed_slot_is_reused_by_the_next_window`、Task 4 `test_multi_client_mode_never_turns_off`。
3. **使用者在客戶端 B 開聊天框，而 A 的聊天框也開著**：翻譯輸入框只為 B 呼出，A 關聊天框不得收掉它。→ Task 7 `test_close_from_another_window_does_not_hide_the_box`。
4. **在 B 貼多行、只有 B 的聊天框開著**：必須走單行模式。→ Task 7 `test_paste_single_line_follows_the_foreground_window`。
5. **關閉程式時兩條 reader 都要 unhook**：supervisor 必須等所有子執行緒 `close()` 完才返回。→ Task 6 `test_stop_joins_every_reader_thread`。

---

### Task 1: `WizChatReader` 綁定單一遊戲視窗

**Files:**
- Modify: `src/reader/mem_reader.py:105-127`（`__init__`）、`:517-565`（`_connect`）、`:656-668`（`_teardown`）
- Modify: `tests/test_reader_connect.py`、`tests/test_input_anchor.py:77,115`、`tests/test_chatlog_diff.py:500,520`

**Interfaces:**
- Produces: `WizChatReader(hwnd: int, game_path: str | None = None, message_log: MessageLog | None = None, slot: int = 0)`；`_connect()` 以 `wizwalker.Client(hwnd)` 建客戶端；所有 `[reader]` log 帶 `slot=`。

- [ ] **Step 1: 改寫連線測試的假物件**

在 `tests/test_reader_connect.py` 把 `_FailingHandler`／`_reader_failing_to_open`（第 14-34 行）換成：

```python
# --- 連線失敗的分類（不需遊戲：以假 wizwalker.Client 注入例外）---
def _reader_failing_to_open(monkeypatch, exc):
    """讓 _connect 走到建 Client 就丟出 exc 的 reader。"""
    import wizwalker

    from src.reader import mem_reader

    def failing_client(hwnd):
        raise exc

    monkeypatch.setattr(mem_reader, "detect_install_path", lambda: None)
    monkeypatch.setattr(wizwalker, "Client", failing_client)
    return WizChatReader(0x1)
```

把 `_StubClient`（第 118-124 行）加上 `close`：

```python
class _StubClient:
    process_id = 4321

    def __init__(self, hook_handler):
        self.hook_handler = hook_handler
        self._pymem = type("M", (), {"base_address": 0x400000})()
        self.closed = False

    async def close(self):
        self.closed = True
```

刪掉 `_StubHandler`（第 162-172 行），`_connecting_reader` 改成：

```python
def _connecting_reader(monkeypatch, tmp_path, hook_handler):
    """讓 _connect 走完整條掛入路徑的 reader（狀態檔改寫進 tmp_path）。"""
    import wizwalker

    from src.reader import hook_state, mem_reader
    monkeypatch.setattr(mem_reader, "detect_install_path", lambda: None)
    monkeypatch.setattr(mem_reader, "HOOK_READY_POLL", 0.0)
    monkeypatch.setattr(hook_state, "STATE_DIR", tmp_path)
    client = _StubClient(hook_handler)
    monkeypatch.setattr(wizwalker, "Client", lambda hwnd: client)
    return WizChatReader(0x1)
```

檔內其餘 `WizChatReader()`（第 129、373 行）改成 `WizChatReader(0x1)`。新增一個測試：

```python
def test_connect_binds_the_given_window_handle(monkeypatch, tmp_path):
    """雙開時每個 reader 只能掛自己那個視窗：建 Client 時必須帶入指定的 hwnd。"""
    import wizwalker

    from src.reader import hook_state, mem_reader
    monkeypatch.setattr(mem_reader, "detect_install_path", lambda: None)
    monkeypatch.setattr(mem_reader, "HOOK_READY_POLL", 0.0)
    monkeypatch.setattr(hook_state, "STATE_DIR", tmp_path)
    seen = []

    def build(hwnd):
        seen.append(hwnd)
        return _StubClient(_StubHookHandler(values=[0x1234]))

    monkeypatch.setattr(wizwalker, "Client", build)
    r = WizChatReader(0xBEEF)
    r._connect()
    assert seen == [0xBEEF]
    assert r.anchored


def test_teardown_closes_the_client(monkeypatch, tmp_path):
    r = _connecting_reader(monkeypatch, tmp_path, _StubHookHandler(values=[0x1234]))
    r._connect()
    client = r._client
    r.close()
    assert client.closed
    assert not r.anchored
```

- [ ] **Step 2: 其他建構點補 hwnd**

`tests/test_input_anchor.py` 第 77 行 `WizChatReader()` → `WizChatReader(0x1)`，第 115 行 `WizChatReader().input_box_screen_rect()` → `WizChatReader(0x1).input_box_screen_rect()`；`tests/test_chatlog_diff.py` 第 500、520 行同樣改成 `WizChatReader(0x1)`。

- [ ] **Step 3: 跑測試確認失敗**

Run: `uv run pytest tests/test_reader_connect.py tests/test_input_anchor.py tests/test_chatlog_diff.py -q`
Expected: FAIL，`TypeError: WizChatReader.__init__() takes from 1 to 3 positional arguments`（或 `Client` 未被呼叫）。

- [ ] **Step 4: 改 `WizChatReader`**

`src/reader/mem_reader.py` 的 `__init__` 簽名與開頭改成：

```python
    def __init__(self, hwnd: int, game_path: str | None = None,
                 message_log: MessageLog | None = None, slot: int = 0):
        self._hwnd = hwnd
        self._slot = slot          # 只用在 log：雙開時分辨這條是哪個客戶端
        self._game_path = game_path
```

`_connect` 改成（取代第 517-565 行整個方法）：

```python
    def _connect(self) -> None:
        import wizwalker
        import wizwalker.utils
        from pymem.exception import CouldNotOpenProcess

        path = self._game_path or detect_install_path()
        log(f"[reader] game path: {path!r} "
            f"(source={'config' if self._game_path else 'detected'}, slot={self._slot})")
        if path:
            wizwalker.utils._OVERRIDE_PATH = path  # Steam 版無登錄檔安裝路徑，需覆寫

        self._loop = asyncio.new_event_loop()
        try:
            # 綁定指定視窗：雙開時每個 reader 各掛自己的客戶端，不能拿列舉結果的第一個
            self._client = wizwalker.Client(self._hwnd)
        except CouldNotOpenProcess as exc:
            # 程序在、handle 開不了＝完整性等級對不上（medium 開不了 high）。
            # 這裡不 teardown 會每輪重試漏掉一個 event loop。
            self._teardown()
            raise GameAccessDenied(
                f"cannot open game process handle ({exc}); the game is likely "
                f"running elevated while this program is not") from exc
        except Exception as exc:
            self._teardown()
            raise GameNotRunning(f"failed to open game process: {exc}") from exc
        self._pid = self._client.process_id
        hook_state.sweep(pid_alive)          # 清掉已不在執行的程序的殘留狀態檔
        self._repair_leaked_hooks(self._pid)  # 修復上次髒退出遺留的 hook（免重開遊戲）
        try:
            # 只啟讀聊天所需的 root_window hook（不啟 player/duel/quest 等）：注入最小化、
            # 不受是否在世界內影響。掛入與等待就緒拆成兩步：wizwalker 內建的等待無限期
            # （見 HOOK_READY_TIMEOUT），且 hook 此刻已寫進遊戲記憶體，要先把還原狀態存起來，
            # 等待途中被硬砍才修得回來。
            self._run(self._client.hook_handler.activate_root_window_hook(
                wait_for_ready=False))
            self._save_hook_state(self._pid)
            waited = self._wait_root_window_ready()
        except Exception as exc:
            self._teardown()
            if is_version_mismatch(exc):
                raise GameVersionMismatch(
                    f"hook does not match this game build: {exc}") from exc
            raise GameNotRunning(f"failed to attach to game: {exc}") from exc
        self._connected = True
        log(f"[reader] attached to game (slot={self._slot}, pid={self._pid}, "
            f"hwnd={self._hwnd:#x}, hook_ready_in={waited:.1f}s)")
```

`__init__` 裡的 `self._handler = None` 刪掉。`_teardown` 開頭改成：

```python
        unhooked = False
        try:
            if self._client is not None and self._loop is not None:
                self._loop.run_until_complete(self._client.close())
                unhooked = True
```

並把後面的 `self._handler = None` 刪掉。檔內其餘 `log(f"[reader] ...` 若有 `pid=` 欄位（`_wait_root_window_ready` 的逾時訊息、`_repair_leaked_hooks`、`_save_hook_state`）在 `pid=` 前補 `slot={self._slot}, `。`_read_chatlog_texts` 的 `GameNotRunning` 訊息不動。

- [ ] **Step 5: 跑測試確認通過**

Run: `uv run pytest tests/test_reader_connect.py tests/test_input_anchor.py tests/test_chatlog_diff.py -q && uv run ruff check src tests tools`
Expected: 全 PASS、`All checks passed!`

- [ ] **Step 6: Commit**

```bash
git add src/reader/mem_reader.py tests/test_reader_connect.py tests/test_input_anchor.py tests/test_chatlog_diff.py
git commit -m "refactor(reader): bind WizChatReader to one game window handle"
```

---

### Task 2: `MessageLog` 帶編號前綴

**Files:**
- Modify: `src/reader/message_log.py:33-40,70-71`
- Test: `tests/test_message_log.py`

**Interfaces:**
- Produces: `MessageLog(stream, slot: int | None = None)`；`slot` 給定時每行以 `[slot=N] ` 開頭。

- [ ] **Step 1: 寫失敗測試**

在 `tests/test_message_log.py` 末尾加：

```python
def test_slot_prefix_marks_every_line(monkeypatch):
    # 雙開時兩條 reader 共寫同一份 messages.log，沒前綴就分不出哪個帳號的判定
    buf = io.StringIO()
    mlog = MessageLog(buf, slot=2)
    mlog.snapshot([SAY], nodes=1, sizes_fn=lambda: [1], input_open=False)
    mlog.decision("append", 1, ["[Bob] hi <3"])
    lines = buf.getvalue().splitlines()
    assert lines and all(line.startswith("[slot=2] ") for line in lines)


def test_no_slot_keeps_the_legacy_format():
    buf, mlog = _log()
    mlog.decision("append", 1, ["x"])
    assert buf.getvalue().startswith("[poll=")
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_message_log.py -q`
Expected: FAIL，`TypeError: __init__() got an unexpected keyword argument 'slot'`

- [ ] **Step 3: 實作**

```python
    def __init__(self, stream, slot: int | None = None):
        self._stream = stream
        self._prefix = f"[slot={slot}] " if slot is not None else ""
        self._prev: list[str] = []
        self._poll = 0
        self._changed = False
```

`_write` 改成：

```python
    def _write(self, line: str) -> None:
        self._stream.write(self._prefix + line + "\n")
```

模組 docstring 末尾加一句：「雙開時每個客戶端一個實例共寫同一串流，以 `[slot=N]` 前綴區分。」

- [ ] **Step 4: 跑測試確認通過**

Run: `uv run pytest tests/test_message_log.py -q && uv run ruff check src tests tools`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/reader/message_log.py tests/test_message_log.py
git commit -m "feat(reader): prefix messages.log lines with the client slot"
```

---

### Task 3: `src/reader/status.py`：橫幅函式搬家＋`StatusBoard`

**Files:**
- Create: `src/reader/status.py`
- Modify: `src/reader/loop.py:37-68`（把 `_PLAIN_NOTICE`、`banner_for`、`translation_banner` 移走，改為 import）
- Test: `tests/test_status.py`（新）

**Interfaces:**
- Produces:
  - `aggregate_state(states: Iterable[str]) -> str`
  - `aggregate_game_issue(reports: Iterable[SlotReport]) -> str | None`
  - `SlotReport(NamedTuple)`: `state: str`, `game_issue: str | None`
  - `StatusBoard(overlay, ui_queue, pool, system_pool=None)`，方法 `report(slot, state, game_issue)`、`drop(slot)`、`refresh()`
  - `banner_for`、`translation_banner` 由 `src.reader.status` 提供，`src.reader.loop` 繼續 re-export（既有測試 `from src.reader.loop import banner_for` 不必改）。
- Consumes: `TranslationPool.error_state`／`error_detail`（既有）。

- [ ] **Step 1: 寫失敗測試**

建 `tests/test_status.py`：

```python
"""多客戶端狀態彙整：優先序、橫幅規則（純函式），以及 StatusBoard 只在變化時推 UI。"""
import queue

from src.reader.status import (
    SlotReport,
    StatusBoard,
    aggregate_game_issue,
    aggregate_state,
)


class FakePool:
    def __init__(self):
        self.error_state = None
        self.error_detail = None
        self.in_flight = 0


class FakeOverlay:
    def __init__(self):
        self.statuses: list[str] = []
        self.errors: list[tuple[str, dict]] = []
        self.clears = 0

    def set_status(self, state):
        self.statuses.append(state)

    def set_error(self, key, **kwargs):
        self.errors.append((key, kwargs))

    def clear_error(self):
        self.clears += 1


def _drain(q):
    while True:
        try:
            q.get_nowait()()
        except queue.Empty:
            break


def test_state_priority_translating_beats_everything():
    assert aggregate_state(["listening", "translating", "locating"]) == "translating"


def test_state_priority_listening_beats_locating_and_failures():
    assert aggregate_state(["locating", "listening", "access_denied"]) == "listening"


def test_permanent_failure_shows_when_nothing_is_attached():
    assert aggregate_state(["waiting_game", "version_mismatch"]) == "version_mismatch"


def test_no_slots_means_waiting_for_the_game():
    assert aggregate_state([]) == "waiting_game"
    assert aggregate_state(["waiting_game", "waiting_game"]) == "waiting_game"


def test_permanent_issue_is_reported_even_if_another_slot_works():
    # 權限不足要使用者動手才解得掉，不能被另一個正常的客戶端蓋掉
    reports = [SlotReport("listening", None), SlotReport("access_denied", "notice.access_denied")]
    assert aggregate_game_issue(reports) == "notice.access_denied"


def test_transient_failure_on_one_slot_is_hidden_while_another_listens():
    # 第二個客戶端還在更新器畫面：第一個正常收訊時不跳「找不到遊戲」
    reports = [SlotReport("listening", None), SlotReport("waiting_game", "notice.game_missing")]
    assert aggregate_game_issue(reports) is None


def test_all_slots_failing_reports_game_missing():
    reports = [SlotReport("waiting_game", "notice.game_missing")]
    assert aggregate_game_issue(reports) == "notice.game_missing"


def test_no_slots_reports_game_missing():
    assert aggregate_game_issue([]) == "notice.game_missing"


def test_all_slots_still_locating_shows_no_banner():
    # 剛啟動、第一輪掛入還沒完成：現況也不跳橫幅
    assert aggregate_game_issue([SlotReport("locating", None)]) is None


def test_board_pushes_only_on_change():
    ov, q = FakeOverlay(), queue.Queue()
    board = StatusBoard(ov, q, FakePool())
    board.report(1, "listening", None)
    board.report(1, "listening", None)
    board.report(2, "locating", None)
    _drain(q)
    assert ov.statuses == ["listening"]


def test_board_drop_recomputes():
    ov, q = FakeOverlay(), queue.Queue()
    board = StatusBoard(ov, q, FakePool())
    board.report(1, "listening", None)
    board.report(2, "locating", None)
    board.drop(1)
    _drain(q)
    assert ov.statuses == ["listening", "locating"]


def test_board_banner_follows_game_issue_and_pool_errors():
    ov, q, pool = FakeOverlay(), queue.Queue(), FakePool()
    board = StatusBoard(ov, q, pool)
    board.report(1, "waiting_game", "notice.game_missing")
    board.report(1, "listening", None)
    pool.error_state = "offline"
    board.refresh()
    _drain(q)
    assert ov.errors == [("notice.game_missing", {}), ("notice.offline", {})]
    assert ov.clears == 1


def test_board_refresh_with_no_slots_reports_waiting_game():
    ov, q = FakeOverlay(), queue.Queue()
    board = StatusBoard(ov, q, FakePool())
    board.refresh()
    _drain(q)
    assert ov.statuses == ["waiting_game"]
    assert ov.errors == [("notice.game_missing", {})]
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_status.py -q`
Expected: FAIL，`ModuleNotFoundError: No module named 'src.reader.status'`

- [ ] **Step 3: 建 `src/reader/status.py`**

```python
"""收訊狀態的彙整：多個客戶端各自回報，這裡算出 overlay 唯一一條狀態列與橫幅該顯示什麼。

`banner_for`／`translation_banner` 決定橫幅文案（純函式）；`StatusBoard` 收各編號的回報、
彙整後只在結果改變時排進 ui_queue。
"""
import queue
import threading
from typing import TYPE_CHECKING, Iterable, NamedTuple

if TYPE_CHECKING:   # 只當型別用：reader 不該在執行期依賴 ui
    from src.translation.pool import TranslationPool
    from src.ui.overlay import OverlayWindow

_PLAIN_NOTICE = {"config": "notice.config_error", "offline": "notice.offline"}
# 狀態字的優先序：只要有一個客戶端在忙就顯示忙；要使用者動手的永久性失敗排在「連線中」
# 之後，只在沒有任何客戶端正常時才浮上來
_STATE_PRIORITY = ("translating", "listening", "locating", "access_denied", "version_mismatch")
_PERMANENT_ISSUES = ("notice.access_denied", "notice.version_mismatch")
_ATTACHED_STATES = ("listening", "translating")


def banner_for(game_issue: str | None, error_state: str | None,
               error_detail: tuple[int | None, str] | None = None) -> tuple[str, dict] | None:
    """決定該顯示哪一條錯誤橫幅：（文案 key，format 變數）或 None＝不顯示。
    game_issue（遊戲端問題的文案 key）優先於翻譯錯誤：連不上遊戲時翻譯狀態已無意義。
    error_detail 是 pool 記下的（HTTP 狀態碼，API 說明）：有就照實顯示，
    沒有才退回只靠狀態猜的固定文案。"""
    if game_issue:
        return game_issue, {}
    if error_state not in _PLAIN_NOTICE:
        return None
    if not error_detail:
        return _PLAIN_NOTICE[error_state], {}
    status, message = error_detail
    if error_state == "config":
        return "notice.config_error_detail", {"status": status, "message": message}
    if status is not None:
        return "notice.offline_http", {"status": status, "message": message}
    return "notice.offline_detail", {"message": message}


def translation_banner(game_issue: str | None, pool: "TranslationPool",
                       system_pool: "TranslationPool | None") -> tuple[str, dict] | None:
    """兩條翻譯佇列任一有錯就顯示：玩家對話優先（它才是主要用途）。"""
    failing = pool if pool.error_state else system_pool
    if failing is None:
        return banner_for(game_issue, None)
    return banner_for(game_issue, failing.error_state, failing.error_detail)


class SlotReport(NamedTuple):
    """一個客戶端最近一次回報：狀態字 key 與遊戲端問題的文案 key（None＝遊戲正常）。"""
    state: str
    game_issue: str | None


def aggregate_state(states: Iterable[str]) -> str:
    """多個客戶端的狀態字取一個給狀態列；沒有客戶端或全部一般性失敗＝等待遊戲。"""
    states = set(states)
    for state in _STATE_PRIORITY:
        if state in states:
            return state
    return "waiting_game"


def aggregate_game_issue(reports: Iterable[SlotReport]) -> str | None:
    """橫幅要顯示的遊戲端問題。永久性問題（權限、版本）任一客戶端有就顯示；
    否則只要有一個客戶端掛入成功就不顯示（另一個還在啟動中很正常）；
    沒有任何客戶端、或全部失敗，才顯示找不到遊戲；全部仍在掛入中則不顯示。"""
    reports = list(reports)
    for r in reports:
        if r.game_issue in _PERMANENT_ISSUES:
            return r.game_issue
    if any(r.state in _ATTACHED_STATES for r in reports):
        return None
    if not reports or any(r.game_issue for r in reports):
        return "notice.game_missing"
    return None


class StatusBoard:
    """各客戶端的收訊執行緒只回報自己；這裡彙整成 overlay 的狀態字與橫幅，
    只在結果改變時排進 ui_queue（否則每輪 poll 都會塞一個沒意義的重繪）。"""

    def __init__(self, overlay: "OverlayWindow", ui_queue: queue.Queue,
                 pool: "TranslationPool", system_pool: "TranslationPool | None" = None):
        self._overlay = overlay
        self._queue = ui_queue
        self._pool = pool
        self._system_pool = system_pool
        self._lock = threading.Lock()
        self._slots: dict[int, SlotReport] = {}
        self._status: str | None = None
        self._banner: tuple[str, dict] | None = None

    def report(self, slot: int, state: str, game_issue: str | None) -> None:
        """某編號的最新狀態。"""
        with self._lock:
            self._slots[slot] = SlotReport(state, game_issue)
            self._push()

    def drop(self, slot: int) -> None:
        """某編號的執行緒結束了。"""
        with self._lock:
            self._slots.pop(slot, None)
            self._push()

    def refresh(self) -> None:
        """回報沒變但翻譯池錯誤狀態可能變了（或還沒有任何客戶端）時重算一次。"""
        with self._lock:
            self._push()

    def _push(self) -> None:
        reports = list(self._slots.values())
        status = aggregate_state(r.state for r in reports)
        banner = translation_banner(aggregate_game_issue(reports), self._pool, self._system_pool)
        if status != self._status:
            self._status = status
            self._queue.put(lambda: self._overlay.set_status(status))
        if banner != self._banner:
            self._banner = banner
            if banner is None:
                self._queue.put(self._overlay.clear_error)
            else:
                key, kwargs = banner
                self._queue.put(lambda: self._overlay.set_error(key, **kwargs))
```

- [ ] **Step 4: `loop.py` 改為 import**

刪掉 `src/reader/loop.py` 第 37-68 行（`_PLAIN_NOTICE`、`banner_for`、`translation_banner`），在 import 區加：

```python
from src.reader.status import banner_for, translation_banner  # noqa: F401  既有測試自此取用
```

（`_OverlayFeed` 在 Task 5 才移除，此處仍使用 `translation_banner`，故不是純 re-export。）

- [ ] **Step 5: 跑測試確認通過**

Run: `uv run pytest tests/test_status.py tests/test_reader_loop.py -q && uv run ruff check src tests tools`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/reader/status.py src/reader/loop.py tests/test_status.py
git commit -m "feat(reader): add StatusBoard that merges per-client status into one overlay status"
```

---

### Task 4: overlay 的來源標記

**Files:**
- Modify: `src/ui/message_list.py:65-74`（`_Message`）、`:76-90`（`__init__`）、`:231-256`（`add_message`）、末尾新增 `slot_marker`／`set_multi_client`
- Modify: `src/ui/overlay.py:551-558`（`add_message`）、新增 `set_multi_client`
- Test: `tests/test_overlay.py`

**Interfaces:**
- Produces: `slot_marker(slot: int) -> str`；`MessageList.add_message(..., slot: int | None = None)`；`MessageList.set_multi_client()`；`OverlayWindow.add_message(..., slot=None)`；`OverlayWindow.set_multi_client()`。

- [ ] **Step 1: 寫失敗測試**

在 `tests/test_overlay.py` 的 import 區把 `from src.ui.message_list import should_stick_to_bottom` 改成 `from src.ui.message_list import should_stick_to_bottom, slot_marker`，並在檔尾加：

```python
def _original_text(ov, index=0):
    """取某則訊息原文行實際畫出的字（本色那份）。"""
    line = ov._list._messages[index].row.winfo_children()[0]
    return line.itemcget(line.find_withtag("fg")[0], "text")


def test_slot_marker_uses_circled_digits_then_falls_back():
    assert slot_marker(1) == "①"
    assert slot_marker(2) == "②"
    assert slot_marker(20) == "⑳"
    assert slot_marker(21) == "[21]"


def test_single_client_rows_carry_no_marker(root):
    # 單開的使用者外觀完全不變：有 slot 但沒進多客戶端模式就不畫
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "甲", slot=1)
    assert _original_text(ov) == "[A] one"


def test_multi_client_mode_marks_existing_and_new_rows(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "甲", slot=1)
    ov.set_multi_client()
    ov.add_message("[B] two", "乙", slot=2)
    assert _original_text(ov, 0) == "① [A] one"
    assert _original_text(ov, 1) == "② [B] two"
    # 存的原文保持乾淨：選取複製拿到的不含標記
    assert ov.visible_messages() == [("[A] one", "甲"), ("[B] two", "乙")]


def test_rows_without_slot_stay_unmarked_in_multi_client_mode(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_multi_client()
    ov.add_message("m", "t")
    assert _original_text(ov) == "m"


def test_multi_client_mode_never_turns_off(root):
    # 第二次呼叫是 no-op：不重畫、不重複加標記
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "甲", slot=1)
    ov.set_multi_client()
    ov.set_multi_client()
    assert _original_text(ov) == "① [A] one"
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_overlay.py -q -k "slot or multi_client or marker"`
Expected: FAIL，`ImportError: cannot import name 'slot_marker'`

- [ ] **Step 3: 實作 `message_list.py`**

在 `should_stick_to_bottom` 之後加：

```python
def slot_marker(slot: int) -> str:
    """客戶端編號的顯示字：帶圈數字 ①…⑳，超過退回 `[n]`。"""
    if 1 <= slot <= 20:
        return chr(0x2460 + slot - 1)
    return f"[{slot}]"
```

`_Message` 加欄位：

```python
    color: str | None = None
    slot: int | None = None   # 來自哪個遊戲客戶端（None＝不標示）
```

`__init__` 在 `self._follow = True` 之後加：

```python
        # 多客戶端模式：偵測到第二個客戶端才開、開了不關（見 set_multi_client）
        self._multi_client = False
```

`add_message` 簽名加 `slot: int | None = None`，docstring 末尾加「slot＝來源客戶端編號，只在多客戶端模式下畫成前綴。」；建原文行那行改成：

```python
        original_line = _outlined_line(row, self._display_original(original, slot),
                                       dimmed(color) if color else FG_ORIGINAL,
                                       ui_font(9), self.wrap)
```

`_messages.append(...)` 加上 `slot`：

```python
        self._messages.append(_Message(now if now is not None else time.time(),
                                       original, translated, row, msg_id, color, slot))
```

在 `update_message` 之後加：

```python
    def _display_original(self, original: str, slot: int | None) -> str:
        if self._multi_client and slot is not None:
            return f"{slot_marker(slot)} {original}"
        return original

    def set_multi_client(self) -> None:
        """進入多客戶端模式：既有各列補上來源標記，之後新列直接帶標記。只開不關 ——
        剛關掉的那個客戶端的訊息還在淡出期內，此時正需要看清是誰的。"""
        if self._multi_client:
            return
        self._multi_client = True
        anchor = self._view_anchor()
        for m in self._messages:
            if m.slot is None:
                continue
            line = m.row.winfo_children()[0]  # 0＝原文行
            if self._selection.holds(m.row):
                self._selection.clear("original line relabelled")
            line.itemconfigure("txt", text=self._display_original(m.original, m.slot))
            _fit_line_height(line)
        self.refresh_scroll(anchor)
```

- [ ] **Step 4: 實作 `overlay.py`**

`add_message` 簽名加 `slot: int | None = None`，內部改為 `self._list.add_message(original, translated, now, msg_id, pending, color, slot)`。在 `update_message` 之後加：

```python
    def set_multi_client(self) -> None:
        """偵測到第二個遊戲客戶端：訊息開始標示來源（見 MessageList.set_multi_client）。"""
        self._list.set_multi_client()
```

- [ ] **Step 5: 跑測試確認通過**

Run: `uv run pytest tests/test_overlay.py tests/test_selection.py -q && uv run ruff check src tests tools`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/ui/message_list.py src/ui/overlay.py tests/test_overlay.py
git commit -m "feat(ui): mark each message with its game client once a second client appears"
```

---

### Task 5: `reader_loop` 改為每視窗一條、向 `StatusBoard` 回報

**Files:**
- Modify: `src/reader/process.py`（新增 `window_exists`）
- Modify: `src/reader/loop.py`（移除 `_OverlayFeed`；新簽名；`MessageIds`；視窗消失即結束）
- Test: `tests/test_reader_loop.py`、`tests/test_reader_connect.py`（`window_exists` 小測試放這裡）

**Interfaces:**
- Consumes: `StatusBoard.report/drop`（Task 3）、`WizChatReader(hwnd, game_path, message_log, slot)`（Task 1）、`overlay.add_message(..., slot=)`（Task 4）。
- Produces:
  - `process.window_exists(hwnd: int) -> bool`
  - `loop.MessageIds()` 帶 `next() -> int`
  - `reader_loop(cfg, hwnd: int, slot: int, overlay, ui_queue, stop, pool, board, msg_ids, on_input_open=None, on_input_close=None, message_log=None, system_pool=None, cache=None, context: ChatContext | None = None)`
  - `on_input_open(hwnd, anchor)`／`on_input_close(hwnd)`

- [ ] **Step 1: 改測試輔助與所有呼叫點**

`tests/test_reader_loop.py`：import 區加

```python
import pytest

from src.reader.loop import MessageIds
from src.reader.status import StatusBoard
```

在 `FakeOverlay.add_message` 加 `slot=None` 參數並記錄 `self.slots.append(slot)`（`__init__` 加 `self.slots: list[int | None] = []`）。在 `FakeReader.__init__` 加 `self.closed = False`，`close()` 改成 `self.closed = True`。新增模組級 autouse fixture 與 `_loop` 輔助：

```python
@pytest.fixture(autouse=True)
def _window_present(monkeypatch):
    """預設視窗一直存在；要測「視窗消失」的測試自己再覆寫。"""
    monkeypatch.setattr(loop_module, "window_exists", lambda hwnd: True)


def _loop(cfg, overlay, ui_queue, stop, pool, *, context=None, system_pool=None, cache=None,
          on_input_open=None, on_input_close=None, hwnd=0x1, slot=1, board=None):
    """以單一客戶端跑 reader_loop：board 一個編號時的行為等同舊的單執行緒版本。"""
    board = board or StatusBoard(overlay, ui_queue, pool, system_pool)
    reader_loop(cfg, hwnd, slot, overlay, ui_queue, stop, pool, board, MessageIds(),
                on_input_open=on_input_open, on_input_close=on_input_close,
                system_pool=system_pool, cache=cache, context=context)
    return board
```

`run_scripted` 裡的 `reader_loop(cfg, overlay, ui_queue, stop, context, pool, system_pool=system_pool, cache=cache)` 改成 `_loop(cfg, overlay, ui_queue, stop, pool, context=context, system_pool=system_pool, cache=cache)`。其餘 8 個呼叫點逐一改：

- 第 227 行 `reader_loop(cfg, FakeOverlay(), ui_queue, stop, ChatContext(), FakePool(), ...)` → `_loop(cfg, FakeOverlay(), ui_queue, stop, FakePool(), ...)`（保留原有關鍵字參數）。
- 第 345、357、415、432、449 行 `reader_loop(cfg, ov, ui_queue, stop, ChatContext(), X)` → `_loop(cfg, ov, ui_queue, stop, X)`。
- `_run_with_input`（第 383 行）→
  ```python
      _loop(cfg, FakeOverlay(), ui_queue, stop, FakePool(),
            on_input_open=lambda hwnd, anchor: events.append(("open", anchor)),
            on_input_close=lambda hwnd: events.append("close"))
  ```
- 第 479 行 → `_loop(cfg, FakeOverlay(), ui_queue, stop, FakePool(), on_input_open=lambda hwnd, anchor: opened_at.append(time.monotonic() - start))`。

檔尾新增測試：

```python
def test_loop_ends_and_closes_the_reader_when_its_window_is_gone(monkeypatch):
    # 客戶端關掉：這條執行緒要 unhook 並結束，編號交還 supervisor
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    stop = threading.Event()
    reader = FakeReader([[], [], []], stop)
    monkeypatch.setattr(loop_module, "WizChatReader", lambda **kw: reader)
    alive = iter([True, False])
    monkeypatch.setattr(loop_module, "window_exists", lambda hwnd: next(alive, False))
    ui_queue: queue.Queue = queue.Queue()
    board = _loop(cfg, ov, ui_queue, stop, FakePool())
    _drain(ui_queue)
    assert reader.closed
    assert not stop.is_set(), "視窗消失只結束這一條，不該停掉整個程式"
    assert board._slots == {}   # drop(slot) 已呼叫


def test_messages_carry_the_slot(monkeypatch):
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    run_scripted(cfg, ov, [["[A] hi"], []], monkeypatch)
    assert ov.slots == [1]


def test_input_callbacks_carry_the_window_handle(monkeypatch):
    cfg = {"poll_interval": 0.01, "auto_show_input": True}
    events = []
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    monkeypatch.setattr(loop_module, "WizChatReader",
                        lambda **kw: InputFakeReader([[], [], []], stop, [False, True, False]))
    _loop(cfg, FakeOverlay(), ui_queue, stop, FakePool(), hwnd=0xABC,
          on_input_open=lambda hwnd, anchor: events.append(("open", hwnd)),
          on_input_close=lambda hwnd: events.append(("close", hwnd)))
    assert events == [("open", 0xABC), ("close", 0xABC)]


def test_message_ids_are_unique_across_threads():
    ids = MessageIds()
    got: list[int] = []
    lock = threading.Lock()

    def take():
        for _ in range(500):
            v = ids.next()
            with lock:
                got.append(v)

    threads = [threading.Thread(target=take) for _ in range(4)]
    for th in threads:
        th.start()
    for th in threads:
        th.join()
    assert len(set(got)) == 2000
```

`tests/test_reader_connect.py` 檔尾加：

```python
def test_window_exists_distinguishes_a_live_window_from_a_dead_handle():
    import win32gui

    from src.reader.process import window_exists
    assert window_exists(win32gui.GetDesktopWindow())
    assert not window_exists(0)
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_reader_loop.py tests/test_reader_connect.py -q`
Expected: FAIL，`ImportError: cannot import name 'MessageIds'`

- [ ] **Step 3: `process.py` 加 `window_exists`**

在 `find_game_window` 之後加：

```python
def window_exists(hwnd: int) -> bool:
    """視窗 handle 是否仍指向一個存在的視窗（客戶端關掉後為 False）；查不到當作不存在。"""
    try:
        import win32gui
        return bool(win32gui.IsWindow(hwnd))
    except Exception:
        return False
```

- [ ] **Step 4: 改寫 `loop.py`**

import 區：保留 `itertools`（`MessageIds` 用）；新增 `from src.reader.process import window_exists`；Task 3 加的那行改成 `from src.reader.status import StatusBoard, banner_for, translation_banner  # noqa: F401`（`banner_for`、`translation_banner` 為既有測試 re-export，loop 本身已不再用）。刪除整個 `_OverlayFeed` 類別。在 `_InputWatch` 之前加：

```python
class MessageIds:
    """跨所有收訊執行緒共用的訊息序號：翻譯池以它回填 overlay 的佔位列，必須全域唯一。"""

    def __init__(self):
        self._count = itertools.count(1)
        self._lock = threading.Lock()

    def next(self) -> int:
        with self._lock:
            return next(self._count)
```

`_InputWatch` 改為帶 hwnd 回報：

```python
class _InputWatch:
    """遊戲聊天輸入框開／關的邊緣觸發：開 → 回報（hwnd，錨點）；關 → 回報 hwnd。
    on_input_open 為 None 時完全不取樣（呼叫端不關心輸入框）。"""

    def __init__(self, reader: WizChatReader, hwnd: int, on_input_open, on_input_close):
        self._reader = reader
        self._hwnd = hwnd
        self._on_open = on_input_open
        self._on_close = on_input_close
        self.open = False

    def poll(self) -> None:
        """取樣一次，狀態翻轉才回報。"""
        if self._on_open is None:
            return
        now_open = self._reader.input_open()
        if now_open == self.open:
            return
        self.open = now_open
        if now_open:
            anchor = self._reader.input_box_screen_rect()
            log(f"[reader] game chat input opened (hwnd={self._hwnd:#x}, anchor={anchor})")
            self._on_open(self._hwnd, anchor)
        else:
            self._report_closed()

    def force_closed(self) -> None:
        """遊戲斷線＝輸入框已不存在：開著就同步收回。"""
        if self.open:
            self.open = False
            self._report_closed()

    def _report_closed(self) -> None:
        log(f"[reader] game chat input closed (hwnd={self._hwnd:#x})")
        if self._on_close is not None:
            self._on_close(self._hwnd)
```

`reader_loop` 整個改成：

```python
def reader_loop(cfg: dict, hwnd: int, slot: int, overlay: "OverlayWindow",
                ui_queue: queue.Queue, stop: threading.Event, pool: TranslationPool,
                board: StatusBoard, msg_ids: MessageIds,
                on_input_open=None, on_input_close=None,
                message_log: MessageLog | None = None,
                system_pool: TranslationPool | None = None,
                cache: TranslationCache | None = None,
                context: ChatContext | None = None) -> None:
    """一個遊戲客戶端（hwnd）的收訊執行緒進入點；stop 被設定或該視窗消失後解除 hook 再返回。
    slot＝這個客戶端的顯示編號；狀態一律經 board 回報，由它彙整多個客戶端。
    on_input_open(hwnd, anchor)／on_input_close(hwnd)＝遊戲聊天輸入框開關的邊緣觸發，
    不受 auto_show_input 影響（要不要自動呼出由呼叫端決定，錨點則熱鍵呼出也用得到）；
    anchor＝遊戲輸入框的螢幕矩形 (x, y, w, h)，讀不到為 None。
    context 預設每條執行緒自建一份；呼叫端要拿去給發話用時可傳入。"""
    # 全用關鍵字：測試以 `lambda **kw` 替換 WizChatReader
    reader = WizChatReader(hwnd=hwnd, game_path=cfg.get("game_path"),
                           message_log=message_log, slot=slot)
    context = context if context is not None else ChatContext()
    inputs = _InputWatch(reader, hwnd, on_input_open, on_input_close)
    game_issue: str | None = None  # 遊戲端問題的橫幅文案 key（None＝遊戲正常）

    def wait_watching_input(seconds: float) -> None:
        """等待下一輪讀取，期間以 INPUT_POLL_INTERVAL 持續取樣輸入框狀態。
        input_open() 只讀一個已快取節點的旗標（實測 <0.1ms），讀聊天記錄則約 10ms，
        故兩者節奏分開。不另開執行緒：WizChatReader 內部跑自己的 asyncio loop，
        跨執行緒併發呼叫會踩到彼此。"""
        deadline = time.monotonic() + seconds
        while not stop.is_set():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return
            stop.wait(min(INPUT_POLL_INTERVAL, remaining))
            inputs.poll()

    while not stop.is_set():
        if not window_exists(hwnd):
            log(f"[reader] game window gone (slot={slot}, hwnd={hwnd:#x}); stopping this reader")
            inputs.force_closed()
            break
        # 每輪重讀設定：設定視窗可能在執行中切換系統訊息開關
        reader.emit_system = cfg.get("translate_system_messages", False)
        board.report(slot, "listening" if reader.anchored else "locating", game_issue)
        try:
            new_lines = reader.read_new()
        except GameNotRunning as exc:
            if isinstance(exc, GameAccessDenied):
                status, issue = "access_denied", "notice.access_denied"
            elif isinstance(exc, GameVersionMismatch):
                status, issue = "version_mismatch", "notice.version_mismatch"
            else:
                status, issue = "waiting_game", "notice.game_missing"
            if issue != game_issue:  # 只在原因改變時記錄，否則每輪重試都灌一行
                log(f"[reader] game not ready (slot={slot}, hwnd={hwnd:#x}): {exc}")
            game_issue = issue
            board.report(slot, status, game_issue)
            inputs.force_closed()
            stop.wait(GAME_MISSING_INTERVAL)
            continue
        except Exception as exc:  # 收訊偶發錯誤：略過該輪，不讓執行緒死掉
            log(f"[reader] poll skipped (slot={slot}): {type(exc).__name__}: {exc}\n"
                f"{traceback.format_exc()}")
            stop.wait(cfg["poll_interval"])
            continue

        if game_issue:
            game_issue = None
            log(f"[reader] game back, resuming (slot={slot})")

        for line in new_lines:
            msg_id = msg_ids.next()
            if line.system:
                # 系統訊息不進上下文；快取命中就直接以完成態顯示，不佔位也不進 pool
                cached = cache.get(line.text) if cache is not None else None
                if cached is not None:
                    ui_queue.put(lambda o=line.text, tr=cached, c=line.color:
                                 overlay.add_message(o, tr, color=c, slot=slot))
                    continue
                ui_queue.put(lambda o=line.text, c=line.color, m=msg_id:
                             overlay.add_message(o, t("notice.pending"), msg_id=m,
                                                 pending=True, color=c, slot=slot))
                if system_pool is not None:
                    system_pool.submit(line.text, [], msg_id)
                continue
            ctx = context.snapshot()   # 該行之前的行；提交後即固定，重試不漂移
            context.push(line.text)
            ui_queue.put(lambda o=line.text, c=line.color, m=msg_id:
                         overlay.add_message(o, t("notice.pending"), msg_id=m,
                                             pending=True, color=c, slot=slot))
            pool.submit(line.text, ctx, msg_id)

        if pool.in_flight or (system_pool is not None and system_pool.in_flight):
            board.report(slot, "translating", game_issue)
        else:
            board.report(slot, "listening" if reader.anchored else "locating", game_issue)

        inputs.poll()
        wait_watching_input(cfg["poll_interval"])

    board.drop(slot)
    reader.close()
```

模組 docstring 第一句改成「單一遊戲客戶端的收訊執行緒主迴圈：讀聊天 → 推進上下文、在 overlay 佔位 → 交給翻譯池。多個客戶端各跑一條，由 supervisor 起／收。」

- [ ] **Step 5: 跑測試確認通過**

Run: `uv run pytest tests/test_reader_loop.py tests/test_reader_connect.py tests/test_status.py -q && uv run ruff check src tests tools`
Expected: PASS（`src/main.py` 此時仍呼叫舊簽名，但不在測試路徑上；Task 7 修正）。

- [ ] **Step 6: Commit**

```bash
git add src/reader/process.py src/reader/loop.py tests/test_reader_loop.py tests/test_reader_connect.py
git commit -m "refactor(reader): run reader_loop per game window and report status through StatusBoard"
```

---

### Task 6: `src/reader/supervisor.py`：發現迴圈與編號配置

**Files:**
- Create: `src/reader/supervisor.py`
- Test: `tests/test_supervisor.py`（新）

**Interfaces:**
- Consumes: `StatusBoard.refresh()`（Task 3）。
- Produces:
  - `allocate_slot(used: Iterable[int]) -> int`
  - `game_windows() -> list[int]`（wizwalker `get_all_wizard_handles` 的薄包裝）
  - `supervise(stop: threading.Event, spawn: Callable[[int, int], threading.Thread], enumerate_windows: Callable[[], list[int]], on_multi_client: Callable[[], None], board: StatusBoard, interval: float = SCAN_INTERVAL) -> None`
  - `SCAN_INTERVAL = 5.0`

- [ ] **Step 1: 寫失敗測試**

建 `tests/test_supervisor.py`：

```python
"""supervisor：遊戲視窗列舉 → 編號配置 → 起／收 reader 執行緒 → 多客戶端模式一次性觸發。
以假列舉與假執行緒驗證，不需遊戲。"""
import threading

from src.reader.supervisor import allocate_slot, supervise


class FakeBoard:
    def __init__(self):
        self.refreshes = 0

    def refresh(self):
        self.refreshes += 1


class Harness:
    """腳本化的視窗列舉：每 tick 回傳 script 的下一項，跑完設 stop。
    spawn 起的假執行緒在它的 hwnd 從列舉消失時結束（列舉端先 join 它，讓 reap 確定看得到）。"""

    def __init__(self, script):
        self.script = list(script)
        self.stop = threading.Event()
        self.spawned: list[tuple[int, int]] = []
        self.gone: dict[int, threading.Event] = {}
        self.threads: dict[int, threading.Thread] = {}
        self.multi_calls = 0
        self.current: list[int] = []

    def enumerate(self):
        if not self.script:
            self.stop.set()
            return self.current
        self.current = self.script.pop(0)
        for hwnd, ev in self.gone.items():
            if hwnd not in self.current and not ev.is_set():
                ev.set()
                self.threads[hwnd].join(timeout=2)
        return self.current

    def spawn(self, hwnd, slot):
        self.spawned.append((hwnd, slot))
        ev = threading.Event()
        self.gone[hwnd] = ev
        th = threading.Thread(target=lambda: ev.wait(), daemon=True)
        th.start()
        self.threads[hwnd] = th
        return th

    def on_multi_client(self):
        self.multi_calls += 1

    def run(self):
        board = FakeBoard()
        supervise(self.stop, self.spawn, self.enumerate, self.on_multi_client, board,
                  interval=0.005)
        for ev in self.gone.values():
            ev.set()
        return board


def test_allocate_slot_takes_the_smallest_free_number():
    assert allocate_slot([]) == 1
    assert allocate_slot([1]) == 2
    assert allocate_slot([2]) == 1
    assert allocate_slot([1, 2, 4]) == 3


def test_each_new_window_gets_a_thread_and_the_next_slot():
    h = Harness([[0xA], [0xA, 0xB]])
    h.run()
    assert h.spawned == [(0xA, 1), (0xB, 2)]


def test_freed_slot_is_reused_by_the_next_window():
    # A 關掉後 C 才開：C 拿回編號 1
    h = Harness([[0xA, 0xB], [0xB], [0xB, 0xC]])
    h.run()
    assert h.spawned == [(0xA, 1), (0xB, 2), (0xC, 1)]


def test_multi_client_mode_fires_once_when_two_windows_coexist():
    h = Harness([[0xA], [0xA, 0xB], [0xB], [0xB, 0xC]])
    h.run()
    assert h.multi_calls == 1


def test_single_window_never_enters_multi_client_mode():
    h = Harness([[0xA], [0xA], []])
    h.run()
    assert h.multi_calls == 0


def test_board_is_refreshed_every_scan():
    h = Harness([[], []])
    board = h.run()
    assert board.refreshes >= 2


def test_stop_joins_every_reader_thread():
    # 關閉程式：兩條 reader 都要跑完 close()（unhook）supervisor 才能返回
    h = Harness([[0xA, 0xB]])
    joined = []

    def spawn(hwnd, slot):
        ev = h.gone.setdefault(hwnd, threading.Event())

        def body():
            ev.wait()
            joined.append(hwnd)

        th = threading.Thread(target=body, daemon=True)
        th.start()
        h.threads[hwnd] = th
        h.spawned.append((hwnd, slot))
        return th

    def enumerate():
        if not h.script:
            h.stop.set()
            for ev in h.gone.values():
                ev.set()   # 模擬 stop 讓 reader_loop 自己結束
            return h.current
        h.current = h.script.pop(0)
        return h.current

    supervise(h.stop, spawn, enumerate, h.on_multi_client, FakeBoard(), interval=0.005)
    assert sorted(joined) == [0xA, 0xB]


def test_enumeration_failure_does_not_kill_the_loop():
    calls = {"n": 0}
    stop = threading.Event()

    def flaky():
        calls["n"] += 1
        if calls["n"] == 1:
            raise RuntimeError("EnumWindows exploded")
        stop.set()
        return []

    supervise(stop, lambda h, s: None, flaky, lambda: None, FakeBoard(), interval=0.005)
    assert calls["n"] == 2
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_supervisor.py -q`
Expected: FAIL，`ModuleNotFoundError: No module named 'src.reader.supervisor'`

- [ ] **Step 3: 建 `src/reader/supervisor.py`**

```python
"""收訊端的 supervisor：定期列舉遊戲視窗，每個視窗配一個編號並起一條 reader_loop 執行緒，
視窗消失就釋出編號；第一次同時有兩個客戶端時通知 overlay 進入多客戶端模式。

reader_loop 本身怎麼跑由 `spawn(hwnd, slot)` 決定（main 提供、閉包帶著共用物件），
這裡只管生命週期。
"""
import threading
import traceback
from typing import Callable, Iterable

from src.log import log
from src.reader.status import StatusBoard

SCAN_INTERVAL = 5.0   # 列舉遊戲視窗的間隔（秒）；與舊版找不到遊戲的重試間隔相同
JOIN_TIMEOUT = 8.0    # 關閉時等每條 reader unhook 的上限（秒），與 main.shutdown 對齊


def allocate_slot(used: Iterable[int]) -> int:
    """最小的未使用正整數：客戶端關掉後編號釋出，下一個新視窗補進來。"""
    taken = set(used)
    slot = 1
    while slot in taken:
        slot += 1
    return slot


def game_windows() -> list[int]:
    """目前所有遊戲客戶端的視窗 handle（wizwalker 以視窗類別名辨識）。"""
    from wizwalker.utils import get_all_wizard_handles
    return list(get_all_wizard_handles())


def supervise(stop: threading.Event, spawn: Callable[[int, int], threading.Thread],
              enumerate_windows: Callable[[], list[int]], on_multi_client: Callable[[], None],
              board: StatusBoard, interval: float = SCAN_INTERVAL) -> None:
    """supervisor 執行緒的進入點；stop 被設定後等所有 reader 執行緒結束再返回。"""
    active: dict[int, tuple[int, threading.Thread]] = {}   # hwnd → (slot, thread)
    multi_client = False
    while not stop.is_set():
        try:
            for hwnd, (slot, thread) in list(active.items()):
                if not thread.is_alive():
                    del active[hwnd]
                    log(f"[reader] client window gone hwnd={hwnd:#x} slot={slot} freed")
            for hwnd in enumerate_windows():
                if hwnd in active:
                    continue
                slot = allocate_slot(s for s, _ in active.values())
                log(f"[reader] client window appeared hwnd={hwnd:#x} slot={slot}")
                active[hwnd] = (slot, spawn(hwnd, slot))
            if not multi_client and len(active) >= 2:
                multi_client = True   # 一次性：之後客戶端減回一個也不關（見 spec）
                log(f"[reader] multi-client mode on (slots={sorted(s for s, _ in active.values())})")
                on_multi_client()
            board.refresh()
        except Exception as exc:   # 列舉或起執行緒失敗不可讓整個收訊端死掉
            log(f"[reader] supervisor scan failed: {type(exc).__name__}: {exc}\n"
                f"{traceback.format_exc()}")
        stop.wait(interval)
    for hwnd, (slot, thread) in active.items():
        thread.join(timeout=JOIN_TIMEOUT)
        if thread.is_alive():
            log(f"[reader] reader thread still alive after {JOIN_TIMEOUT}s "
                f"(slot={slot}, hwnd={hwnd:#x}); hooks may be left for next-launch repair")
```

（`test_enumeration_failure_does_not_kill_the_loop` 傳入的 `spawn` 回傳 None，但那個測試從不列舉出視窗，故不會被呼叫。）

- [ ] **Step 4: 跑測試確認通過**

Run: `uv run pytest tests/test_supervisor.py -q && uv run ruff check src tests tools`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/reader/supervisor.py tests/test_supervisor.py
git commit -m "feat(reader): supervise one reader thread per game window"
```

---

### Task 7: `main.py` 接線：supervisor、前景客戶端的輸入框事件、Ctrl+V 單行判定

**Files:**
- Modify: `src/main.py`（import 區、`on_paste_hotkey`、`build_app`、`main()`、`shutdown` log 字串）
- Modify: `src/ui/input_box.py`（新增 `target_hwnd` 屬性）
- Test: `tests/test_main.py`

**Interfaces:**
- Consumes: `supervise`／`game_windows`（Task 6）、`StatusBoard`（Task 3）、`reader_loop`／`MessageIds`（Task 5）、`MessageLog(stream, slot)`（Task 2）、`overlay.set_multi_client()`（Task 4）。
- Produces:
  - `main.ChatInputTracker`：`opened(hwnd)`、`closed(hwnd)`、`is_open(hwnd) -> bool`
  - `main.GameInputEvents(cfg, input_box, tracker, foreground=win32gui.GetForegroundWindow)`：`opened(hwnd, anchor)`、`closed(hwnd)`
  - `main.on_paste_hotkey(cfg, tracker) -> threading.Thread`
  - `InputBox.target_hwnd` 屬性（`int | None`）
  - `build_app(cfg, root, message_stream)`（第三個參數由 `MessageLog` 改為串流）

- [ ] **Step 1: 寫失敗測試**

在 `tests/test_main.py` 檔尾加：

```python
# --- 雙開：聊天框事件只認前景客戶端，Ctrl+V 單行判定看前景視窗 ---
class _FakeInputBox:
    def __init__(self):
        self.calls: list = []

    def set_anchor(self, rect):
        self.calls.append(("anchor", rect))

    def clear_anchor(self):
        self.calls.append("clear_anchor")

    def show(self):
        self.calls.append("show")

    def hide(self):
        self.calls.append("hide")


def _events(auto_show=True, foreground=0xA):
    box = _FakeInputBox()
    tracker = main.ChatInputTracker()
    ev = main.GameInputEvents({"auto_show_input": auto_show}, box, tracker,
                              foreground=lambda: foreground)
    return ev, box, tracker


def test_open_in_the_foreground_window_anchors_and_shows():
    ev, box, tracker = _events(foreground=0xA)
    ev.opened(0xA, (1, 2, 3, 4))
    assert box.calls == [("anchor", (1, 2, 3, 4)), "show"]
    assert tracker.is_open(0xA)


def test_open_in_a_background_window_is_tracked_but_not_shown():
    # 使用者正在玩 B，A 的聊天框開了：不彈框、不改錨點，但記得 A 開著（貼上判定要用）
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
    assert box.calls == [("anchor", (1, 2, 3, 4)), "clear_anchor"]


def test_paste_single_line_follows_the_foreground_window(monkeypatch):
    # 在 B 貼多行、只有 B 的聊天框開著：要走單行；A 開著、前景是 B 且 B 沒開：多行
    seen = []
    monkeypatch.setattr(main, "paste_clipboard",
                        lambda hwnd, delay, single_line: seen.append((hwnd, single_line)))
    monkeypatch.setattr(main.win32gui, "GetForegroundWindow", lambda: 0xB)
    tracker = main.ChatInputTracker()
    tracker.opened(0xB)
    main.on_paste_hotkey({"type_delay": 0}, tracker).join()
    tracker.closed(0xB)
    tracker.opened(0xA)
    main.on_paste_hotkey({"type_delay": 0}, tracker).join()
    assert seen == [(0xB, True), (0xB, False)]


def test_build_app_spawns_readers_through_the_supervisor():
    """接線只在 build_app 裡（完整啟動才跑得到），以原始碼釘住關鍵 token。"""
    source = (ROOT / "src" / "main.py").read_text(encoding="utf-8")
    assert "target=supervise" in source
    assert "MessageLog(message_stream, slot=slot)" in source
    assert "overlay.set_multi_client" in source
```

（`ROOT` 已在檔內定義供其他源碼釘住測試使用；若沒有則加 `ROOT = Path(__file__).resolve().parents[1]`。）

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_main.py -q`
Expected: FAIL，`AttributeError: module 'src.main' has no attribute 'ChatInputTracker'`

- [ ] **Step 3: `InputBox.target_hwnd`**

`src/ui/input_box.py` 在既有 `@property`（第 66 行後的第一個屬性）之前加：

```python
    @property
    def target_hwnd(self) -> int | None:
        """呼出當下記住的前景遊戲視窗（譯文要打回去的那個）；未呼出過為 None。"""
        return self._target_hwnd
```

- [ ] **Step 4: 改 `main.py`**

import 區：`from src.reader.loop import reader_loop` → `from src.reader.loop import MessageIds, reader_loop`；加 `from src.reader.status import StatusBoard`、`from src.reader.supervisor import game_windows, supervise`。

`on_paste_hotkey`（第 250-259 行）改成：

```python
def on_paste_hotkey(cfg: dict, tracker: "ChatInputTracker") -> threading.Thread:
    """攔到遊戲內的 Ctrl+V：記下當下的前景視窗，交給背景執行緒鍵入剪貼簿（回傳該執行緒）。
    不能在 hook 回呼裡直接打字（Windows 會判定 hook 逾時而整個拔掉）。**前景那個客戶端**
    的聊天輸入框開著時走單行模式（換行改空格），其他地方換行照打。"""
    hwnd = win32gui.GetForegroundWindow()
    worker = threading.Thread(target=paste_clipboard,
                              args=(hwnd, cfg["type_delay"], tracker.is_open(hwnd)),
                              daemon=True)
    worker.start()
    return worker
```

在 `on_paste_hotkey` 之前加兩個類別：

```python
class ChatInputTracker:
    """哪些遊戲視窗的聊天輸入框正開著。reader 執行緒（經 ui_queue）寫、貼上的鍵盤 hook 讀，
    雙開時每個客戶端各自開關，不能用一個布林。"""

    def __init__(self) -> None:
        self._open: set[int] = set()
        self._lock = threading.Lock()

    def opened(self, hwnd: int) -> None:
        with self._lock:
            self._open.add(hwnd)

    def closed(self, hwnd: int) -> None:
        with self._lock:
            self._open.discard(hwnd)

    def is_open(self, hwnd: int) -> bool:
        with self._lock:
            return hwnd in self._open


class GameInputEvents:
    """遊戲聊天輸入框開／關事件的分派（UI 執行緒）。只認前景客戶端：使用者正在玩 B 時，
    A 的聊天框開關不該彈出或收掉他的翻譯輸入框。"""

    def __init__(self, cfg: dict, input_box: InputBox, tracker: ChatInputTracker,
                 foreground=win32gui.GetForegroundWindow) -> None:
        self._cfg = cfg
        self._box = input_box
        self._tracker = tracker
        self._foreground = foreground
        self._shown_for: int | None = None   # 目前的錨點／自動呼出是為哪個客戶端

    def opened(self, hwnd: int, anchor) -> None:
        """聊天框開了：記下來；是前景客戶端才設錨點（熱鍵呼出也要貼齊）、依設定自動呼出。"""
        self._tracker.opened(hwnd)
        if self._foreground() != hwnd:
            log(f"[app] game chat opened in a background client (hwnd={hwnd:#x}); ignoring")
            return
        self._shown_for = hwnd
        if anchor is not None:
            self._box.set_anchor(anchor)
        if self._cfg["auto_show_input"]:
            self._box.show()

    def closed(self, hwnd: int) -> None:
        """聊天框關了：只有當初為它設錨點／呼出的那個客戶端才收起，打到一半的文字留到下次。"""
        self._tracker.closed(hwnd)
        if hwnd != self._shown_for:
            return
        self._shown_for = None
        self._box.clear_anchor()
        if self._cfg["auto_show_input"]:
            self._box.hide()
```

`build_app(cfg, root, message_log)` 簽名改成 `build_app(cfg: dict, root: tk.Tk, message_stream) -> App`，docstring 補「`message_stream`＝messages.log 的串流，每個客戶端各建一個 `MessageLog` 共寫」。函式內：

- 刪掉 `context = ChatContext()`；改在 `deliver` 之前加 `contexts: dict[int, ChatContext] = {}`，並定義

  ```python
  def context_for(hwnd: int | None) -> list[str]:
      """發話時帶的上下文：譯文要打回哪個客戶端，就用那個客戶端的聊天。"""
      ctx = contexts.get(hwnd) if hwnd is not None else None
      return ctx.snapshot() if ctx is not None else []
  ```

- `InputBox(...)` 的翻譯回呼改成 `lambda text, cancel: translators[SLOT_OUTGOING].translate_outgoing(text, context_for(input_box.target_hwnd), cancel=cancel)`（`input_box` 在 lambda 執行時已綁定）。
- 刪掉 `game_chat_open = threading.Event()` 與其註解，改成 `tracker = ChatInputTracker()`；`install_paste_hook(... lambda: on_paste_hotkey(cfg, tracker))`。
- 刪掉 `on_game_input_open`／`on_game_input_close` 兩個函式，改成 `input_events = GameInputEvents(cfg, input_box, tracker)`。
- 把起 `reader_thread` 那段換成：

  ```python
      stop = threading.Event()
      board = StatusBoard(overlay, ui_queue, pool, system_pool)   # pool／system_pool 已在上方解包
      msg_ids = MessageIds()

      def spawn(hwnd: int, slot: int) -> threading.Thread:
          """supervisor 發現一個遊戲視窗：為它起一條收訊執行緒（上下文每客戶端一份）。"""
          contexts[hwnd] = ChatContext()
          thread = threading.Thread(
              target=reader_loop,
              args=(cfg, hwnd, slot, overlay, ui_queue, stop, pool, board, msg_ids),
              kwargs={"on_input_open": lambda h, anchor: ui_queue.put(
                          lambda: input_events.opened(h, anchor)),
                      "on_input_close": lambda h: ui_queue.put(lambda: input_events.closed(h)),
                      "message_log": MessageLog(message_stream, slot=slot),
                      "system_pool": system_pool,
                      "cache": cache,
                      "context": contexts[hwnd]},
              daemon=True, name=f"reader-{slot}")
          thread.start()
          return thread

      reader_thread = threading.Thread(
          target=supervise,
          args=(stop, spawn, game_windows, lambda: ui_queue.put(overlay.set_multi_client), board),
          daemon=True, name="reader-supervisor")
      reader_thread.start()
  ```

`main()` 裡 `message_log = MessageLog(TimestampedStream(open_session_log("messages.log")))` 改成 `message_stream = TimestampedStream(open_session_log("messages.log"))`，並把傳給 `build_app` 的參數改成 `message_stream`。若 `MessageLog` 在 `main()` 已無其他使用，import 保留（`spawn` 用到）。

`shutdown` 的 log 字串 `"[app] shutting down, waiting for reader to unhook"` → `"[app] shutting down, waiting for readers to unhook"`；註解「等 reader 跑完 reader.close()」改為「等 supervisor 把每條 reader join 完（各自 unhook）」。模組 docstring 改成「進入點：supervisor 執行緒（每個遊戲客戶端一條 wizwalker 收訊執行緒）+ 全域熱鍵 + tkinter 主迴圈（UI 事件經 ui_queue 序列化）。」

- [ ] **Step 5: 跑測試確認通過**

Run: `uv run pytest tests/test_main.py tests/test_input_box.py tests/test_paste.py -q && uv run ruff check src tests tools`
Expected: PASS

- [ ] **Step 6: 實機冒煙（需要遊戲登入在世界內，由使用者操作）**

Run: `uv run run.py`，看 `app.log` 出現 `client window appeared hwnd=… slot=1` 與 `attached to game (slot=1, …)`，聊天照常翻譯；關閉程式後 `shutdown complete` 前沒有 `still alive` 訊息。若使用者可雙開：第二個客戶端進世界後應見 `slot=2` 與 `multi-client mode on`，overlay 訊息帶 ①／②。

- [ ] **Step 7: Commit**

```bash
git add src/main.py src/ui/input_box.py tests/test_main.py
git commit -m "feat(app): read chat from every running game client and follow the foreground one for input"
```

---

### Task 8: 文件、spec 補記與全套驗證

**Files:**
- Modify: `README.md:82`、`README_ZH-TW.md:82`、`README_ZH-CN.md:82`
- Modify: `docs/superpowers/specs/2026-09-21-multi-client-design.md`（`reader_loop` 一節）

- [ ] **Step 1: README 三份各加一條功能**

在「不會看錯字、不會漏訊息」那條（第 82 行）之後各插入一行：

`README_ZH-TW.md`：
```
- **雙開也照顧到**：同時開多個遊戲客戶端時，每個客戶端的聊天都會翻譯，訊息前面標示來自哪一個；發話與貼上都打進你當下操作的那個
```

`README_ZH-CN.md`：
```
- **双开也照顾到**：同时开多个游戏客户端时，每个客户端的聊天都会翻译，消息前面标示来自哪一个；发言与粘贴都打进你当下操作的那个
```

`README.md`：
```
- **Multiple game clients are covered**: with several clients running at once, every client's chat is translated and each message is marked with the client it came from; speaking and pasting go to the client you are currently using
```

- [ ] **Step 2: spec 補記實作時確定的兩點**

在 spec 的 `### reader_loop` 一節，把「`ChatContext` 在迴圈內建立（每執行緒一份），不再由 `main` 傳入。」改成：

```
- `ChatContext` 每客戶端一份，由 `main` 的 `spawn` 建立並登錄在 hwnd→context 對照表傳入
  `reader_loop`：發話翻譯要帶的上下文改取「譯文要打回去的那個客戶端」的聊天
  （`InputBox.target_hwnd`），不再有全域一份。
```

在 `### src/reader/supervisor.py（新）` 一節，`StatusBoard` 的方法列表 `report_no_clients()` 改為 `refresh()`，並補一句「零客戶端由空的回報表自然算出等待遊戲中＋找不到遊戲，不需專門方法」。狀態改成「狀態：已實作。」。

- [ ] **Step 3: 全套 lint 與測試**

Run: `uv run ruff check src tests tools && uv run pytest -q`
Expected: `All checks passed!`、全部 PASS。

- [ ] **Step 4: Commit**

```bash
git add README.md README_ZH-TW.md README_ZH-CN.md docs/superpowers/specs/2026-09-21-multi-client-design.md
git commit -m "docs: describe multi-client chat translation"
```
