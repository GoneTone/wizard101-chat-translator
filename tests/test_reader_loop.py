"""reader_loop 行為:讀「有序尾段」→ 結尾新增偵測 → 翻譯 → overlay。
只翻接在結尾的新訊息(舊訊息在別處重現不冒出);離線時失敗的行下輪重試,橫幅正確。"""
import queue
import threading

import httpx

import src.main as main_module
from src.main import appended_lines, reader_loop
from src.reader.mem_reader import GameNotRunning


class FakeOverlay:
    def __init__(self):
        self.messages: list[tuple[str, str]] = []
        self.errors: list[str] = []
        self.clears = 0

    def add_message(self, original, translated):
        self.messages.append((original, translated))

    def set_error(self, text):
        self.errors.append(text)

    def clear_error(self):
        self.clears += 1


def run_cycles(cfg, translator, overlay, lines, stop_after_cycle, monkeypatch):
    """每輪 read_ordered_tail 回傳同一個 lines;跑到第 stop_after_cycle 輪停。"""
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    count = {"n": 0}

    def fake_tail(process_name="WizardGraphicalClient.exe", tail=80):
        count["n"] += 1
        if count["n"] >= stop_after_cycle:
            stop.set()
        return list(lines)

    monkeypatch.setattr(main_module, "read_ordered_tail", fake_tail)
    reader_loop(cfg, translator, overlay, ui_queue, stop)
    _drain(ui_queue)
    return overlay


def run_scripted(cfg, translator, overlay, reads, monkeypatch):
    """每輪 read_ordered_tail 依序回傳 reads[i];跑完 len(reads) 輪後停。"""
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    count = {"n": 0}

    def fake_tail(process_name="WizardGraphicalClient.exe", tail=80):
        i = count["n"]
        count["n"] += 1
        if count["n"] >= len(reads):
            stop.set()
        return list(reads[i])

    monkeypatch.setattr(main_module, "read_ordered_tail", fake_tail)
    reader_loop(cfg, translator, overlay, ui_queue, stop)
    _drain(ui_queue)


def _drain(ui_queue):
    while True:
        try:
            ui_queue.get_nowait()()
        except queue.Empty:
            break


# --- appended_lines 純函式 ---
def test_appended_lines_returns_after_anchor():
    assert appended_lines(["a", "b", "c"], ["a", "b", "c", "d", "e"]) == ["d", "e"]


def test_appended_lines_none_when_anchor_absent():
    assert appended_lines(["a", "b", "c"], ["x", "y", "z"]) is None


def test_appended_lines_empty_when_no_new():
    assert appended_lines(["a", "b"], ["a", "b"]) == []


def test_appended_lines_ignores_prefix_changes():
    # 舊訊息在前面重現、但結尾錨點不變 → 沒有新增
    assert appended_lines(["a", "b", "c"], ["old", "a", "b", "c"]) == []


# --- 結尾新增偵測 ---
class OkTranslator:
    def __init__(self):
        self.calls: list[str] = []

    def to_zh(self, text):
        self.calls.append(text)
        return f"譯:{text}"


def test_only_appended_messages_translated(monkeypatch):
    cfg = {"poll_interval": 0.01, "startup_tail": 0}
    tr = OkTranslator()
    ov = FakeOverlay()
    reads = [["[A] a", "[B] b"], ["[A] a", "[B] b", "[C] c"], ["[A] a", "[B] b", "[C] c", "[D] d"]]
    run_scripted(cfg, tr, ov, reads, monkeypatch)
    assert tr.calls == ["[C] c", "[D] d"]  # 啟動不翻,之後只翻結尾新增


def test_old_message_resurfacing_not_translated(monkeypatch):
    cfg = {"poll_interval": 0.01, "startup_tail": 0}
    tr = OkTranslator()
    ov = FakeOverlay()
    # 掃3:一則舊訊息 X 在前面重現,但結尾錨點 [D] d 不變 → 不翻 X
    reads = [
        ["[A] a", "[B] b"],
        ["[A] a", "[B] b", "[D] d"],
        ["[X] old", "[A] a", "[B] b", "[D] d"],
    ]
    run_scripted(cfg, tr, ov, reads, monkeypatch)
    assert tr.calls == ["[D] d"]


def test_anchor_lost_falls_back_to_seen_diff(monkeypatch):
    cfg = {"poll_interval": 0.01, "startup_tail": 0}
    tr = OkTranslator()
    ov = FakeOverlay()
    # 掃2 尾段全換(捲太快,錨點消失)→ 備援:翻沒看過的行
    reads = [["[A] a", "[B] b"], ["[X] x", "[Y] y"]]
    run_scripted(cfg, tr, ov, reads, monkeypatch)
    assert tr.calls == ["[X] x", "[Y] y"]


def test_startup_translates_ordered_tail(monkeypatch):
    cfg = {"poll_interval": 0.01, "startup_tail": 3}
    tr = OkTranslator()
    ov = FakeOverlay()
    backlog = [f"[P] m{i:02d}" for i in range(10)]
    run_scripted(cfg, tr, ov, [backlog], monkeypatch)
    assert tr.calls == backlog[-3:]  # 有序尾段的最後 3 句(正確的最新)


def test_startup_tail_zero_translates_nothing(monkeypatch):
    cfg = {"poll_interval": 0.01, "startup_tail": 0}
    tr = OkTranslator()
    ov = FakeOverlay()
    run_scripted(cfg, tr, ov, [["[A] a", "[B] b", "[C] c"]], monkeypatch)
    assert tr.calls == []


# --- 離線恢復 ---
class FlakyTranslator:
    def __init__(self):
        self.calls = 0

    def to_zh(self, text):
        self.calls += 1
        if self.calls == 1:
            raise httpx.HTTPError("offline")
        return f"譯:{text}"


def test_failed_line_is_retranslated_after_recovery(monkeypatch):
    monkeypatch.setattr(main_module, "BACKOFF_STEPS", [0.01, 0.01, 0.01])
    cfg = {"poll_interval": 0.01, "startup_tail": 100}
    tr = FlakyTranslator()
    ov = FakeOverlay()
    run_cycles(cfg, tr, ov, ["hello there"], stop_after_cycle=2, monkeypatch=monkeypatch)
    assert tr.calls == 2
    assert ov.messages == [("hello there", "譯:hello there")]


def test_banner_set_while_offline_and_cleared_only_after_success(monkeypatch):
    monkeypatch.setattr(main_module, "BACKOFF_STEPS", [0.01, 0.01, 0.01])
    cfg = {"poll_interval": 0.01, "startup_tail": 100}
    tr = FlakyTranslator()
    ov = FakeOverlay()
    run_cycles(cfg, tr, ov, ["hello there"], stop_after_cycle=2, monkeypatch=monkeypatch)
    assert ov.errors == ["⚠ 翻譯伺服器離線，重試中…"]
    assert ov.clears == 1


class BatchFlakyTranslator:
    def __init__(self):
        self.calls: list[str] = []
        self._failed_once = False

    def to_zh(self, text):
        self.calls.append(text)
        if not self._failed_once:
            self._failed_once = True
            raise httpx.HTTPError("offline")
        return f"譯:{text}"


def test_batch_remainder_is_retried_after_recovery(monkeypatch):
    monkeypatch.setattr(main_module, "BACKOFF_STEPS", [0.01, 0.01, 0.01])
    cfg = {"poll_interval": 0.01, "startup_tail": 100}
    tr = BatchFlakyTranslator()
    ov = FakeOverlay()
    run_cycles(cfg, tr, ov, ["line one", "line two", "line three"],
               stop_after_cycle=2, monkeypatch=monkeypatch)
    # 第一輪 line one 失敗 → 整批下輪重試,三行最後都翻到
    assert tr.calls == ["line one", "line one", "line two", "line three"]
    assert ov.messages == [
        ("line one", "譯:line one"),
        ("line two", "譯:line two"),
        ("line three", "譯:line three"),
    ]


class NeverTranslator:
    def to_zh(self, text):
        raise AssertionError("找不到遊戲時不應嘗試翻譯")


def test_game_not_running_shows_banner_once(monkeypatch):
    monkeypatch.setattr(main_module, "GAME_MISSING_INTERVAL", 0.01)
    cfg = {"poll_interval": 0.01, "startup_tail": 0}
    ov = FakeOverlay()
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    count = {"n": 0}

    def fake_tail(process_name="WizardGraphicalClient.exe", tail=80):
        count["n"] += 1
        if count["n"] >= 2:
            stop.set()
        raise GameNotRunning("no game")

    monkeypatch.setattr(main_module, "read_ordered_tail", fake_tail)
    reader_loop(cfg, NeverTranslator(), ov, ui_queue, stop)
    _drain(ui_queue)
    assert ov.errors == ["⚠ 找不到遊戲程序，等待中…"]
    assert ov.messages == []
