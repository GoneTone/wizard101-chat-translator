"""reader_loop 行為:記憶體收訊 → 去重 → 翻譯 → overlay。
離線恢復:翻譯失敗的行(與同批未試的行)下一輪重新嘗試,橫幅只在還在離線時顯示,
只有真的翻譯成功過才清除橫幅、重置退避;找不到遊戲時顯示對應橫幅並重試。"""
import queue
import threading

import httpx

import src.main as main_module
from src.main import reader_loop
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
    """跑 reader_loop 到第 stop_after_cycle 輪就停;每輪 read_chat_lines 回傳 lines。
    跑完把 ui_queue 回呼全部執行掉(模擬 pump())。"""
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    count = {"n": 0}

    def fake_read(process_name="WizardGraphicalClient.exe"):
        count["n"] += 1
        if count["n"] >= stop_after_cycle:
            stop.set()
        return list(lines)

    monkeypatch.setattr(main_module, "read_chat_lines", fake_read)
    reader_loop(cfg, translator, overlay, ui_queue, stop)

    while True:
        try:
            ui_queue.get_nowait()()
        except queue.Empty:
            break
    return overlay


class FlakyTranslator:
    """第一次呼叫拋 HTTPError(模擬伺服器離線),之後成功。"""

    def __init__(self):
        self.calls = 0

    def to_zh(self, text):
        self.calls += 1
        if self.calls == 1:
            raise httpx.HTTPError("offline")
        return f"譯:{text}"


def test_failed_line_is_retranslated_after_recovery(monkeypatch):
    monkeypatch.setattr(main_module, "BACKOFF_STEPS", [0.01, 0.01, 0.01])
    cfg = {"poll_interval": 0.01, "startup_tail": 100}  # 首輪照翻(非測啟動抑制)
    translator = FlakyTranslator()
    overlay = FakeOverlay()

    run_cycles(cfg, translator, overlay, ["hello there"], stop_after_cycle=2, monkeypatch=monkeypatch)

    assert translator.calls == 2
    assert overlay.messages == [("hello there", "譯:hello there")]


def test_banner_set_while_offline_and_cleared_only_after_success(monkeypatch):
    monkeypatch.setattr(main_module, "BACKOFF_STEPS", [0.01, 0.01, 0.01])
    cfg = {"poll_interval": 0.01, "startup_tail": 100}  # 首輪照翻(非測啟動抑制)
    translator = FlakyTranslator()
    overlay = FakeOverlay()

    run_cycles(cfg, translator, overlay, ["hello there"], stop_after_cycle=2, monkeypatch=monkeypatch)

    assert overlay.errors == ["⚠ 翻譯伺服器離線,重試中…"]
    assert overlay.clears == 1


class BatchFlakyTranslator:
    """整批第一行失敗一次(模擬離線),之後全部成功。驗證同批未試的行也一起重試。"""

    def __init__(self):
        self.calls: list[str] = []
        self._failed_once = False

    def to_zh(self, text):
        self.calls.append(text)
        if not self._failed_once:
            self._failed_once = True
            raise httpx.HTTPError("offline")
        return f"譯:{text}"


def test_batch_remainder_is_forgotten_and_retried_after_recovery(monkeypatch):
    monkeypatch.setattr(main_module, "BACKOFF_STEPS", [0.01, 0.01, 0.01])
    cfg = {"poll_interval": 0.01, "startup_tail": 100}  # 首輪照翻(非測啟動抑制)
    translator = BatchFlakyTranslator()
    overlay = FakeOverlay()

    run_cycles(cfg, translator, overlay, ["line one", "line two", "line three"],
               stop_after_cycle=2, monkeypatch=monkeypatch)

    # 第一輪只試了 "line one" 就失敗,整批(含未試的 line two/three)下一輪重新嘗試,
    # 三行最後都成功翻譯,沒有任何一行被永久漏掉。
    assert translator.calls == ["line one", "line one", "line two", "line three"]
    assert overlay.messages == [
        ("line one", "譯:line one"),
        ("line two", "譯:line two"),
        ("line three", "譯:line three"),
    ]
    assert overlay.errors == ["⚠ 翻譯伺服器離線,重試中…"]
    assert overlay.clears == 1


class OkTranslator:
    def __init__(self):
        self.calls: list[str] = []

    def to_zh(self, text):
        self.calls.append(text)
        return f"譯:{text}"


def run_scripted(cfg, translator, overlay, reads, monkeypatch):
    """每輪 read_chat_lines 依序回傳 reads[i];跑完 len(reads) 輪後停止。"""
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    count = {"n": 0}

    def fake_read(process_name="WizardGraphicalClient.exe"):
        i = count["n"]
        count["n"] += 1
        if count["n"] >= len(reads):
            stop.set()
        return list(reads[i])

    monkeypatch.setattr(main_module, "read_chat_lines", fake_read)
    reader_loop(cfg, translator, overlay, ui_queue, stop)
    while True:
        try:
            ui_queue.get_nowait()()
        except queue.Empty:
            break


def test_startup_translates_only_tail_of_backlog(monkeypatch):
    backlog = [f"[P] msg{i:02d}" for i in range(25)]
    cfg = {"poll_interval": 0.01, "startup_tail": 10}
    translator = OkTranslator()
    overlay = FakeOverlay()

    # 掃1:整段 backlog(啟動翻最後 10 句);掃2:兩句新訊息首次出現(未穩定);
    # 掃3:兩句新訊息連續第二次 → 穩定 → 翻譯
    new = ["[P] newA", "[P] newB"]
    reads = [backlog, backlog + new, backlog + new]
    run_scripted(cfg, translator, overlay, reads, monkeypatch)

    # 啟動翻 backlog 最後 10 句;之後翻穩定出現的兩句新訊息
    assert translator.calls == backlog[-10:] + new


def test_transient_lines_not_translated_only_stable_ones(monkeypatch):
    cfg = {"poll_interval": 0.01, "startup_tail": 0}
    translator = OkTranslator()
    overlay = FakeOverlay()

    # 掃1:啟動基準([A] 標記看過,不翻)
    # 掃2:[B] 首次出現(尚未穩定,不翻)
    # 掃3:[B] 連續第二次出現 → 穩定 → 翻譯
    # 掃4:[X] 暫時垃圾出現一次(不穩定,不翻)
    # 掃5:[X] 消失 → 永遠不翻
    reads = [
        ["[A] one"],
        ["[A] one", "[B] two"],
        ["[A] one", "[B] two"],
        ["[A] one", "[B] two", "[X] junk"],
        ["[A] one", "[B] two"],
    ]
    run_scripted(cfg, translator, overlay, reads, monkeypatch)

    assert translator.calls == ["[B] two"]  # 只翻穩定的 B;A 為啟動歷史、X 為暫時垃圾


def test_startup_tail_smaller_than_backlog_translates_all(monkeypatch):
    backlog = ["[P] a", "[P] b", "[P] c"]
    cfg = {"poll_interval": 0.01, "startup_tail": 10}
    translator = OkTranslator()
    overlay = FakeOverlay()

    run_scripted(cfg, translator, overlay, [backlog], monkeypatch)

    assert translator.calls == backlog  # backlog 不足 10 句時全翻


class NeverTranslator:
    def to_zh(self, text):
        raise AssertionError("找不到遊戲時不應嘗試翻譯")


def test_game_not_running_shows_banner_once_and_retries(monkeypatch):
    monkeypatch.setattr(main_module, "GAME_MISSING_INTERVAL", 0.01)
    cfg = {"poll_interval": 0.01, "startup_tail": 100}  # 首輪照翻(非測啟動抑制)
    overlay = FakeOverlay()
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    count = {"n": 0}

    def fake_read(process_name="WizardGraphicalClient.exe"):
        count["n"] += 1
        if count["n"] >= 2:
            stop.set()
        raise GameNotRunning("no game")

    monkeypatch.setattr(main_module, "read_chat_lines", fake_read)
    reader_loop(cfg, NeverTranslator(), overlay, ui_queue, stop)

    while True:
        try:
            ui_queue.get_nowait()()
        except queue.Empty:
            break

    assert overlay.errors == ["⚠ 找不到遊戲程序,等待中…"]  # 只顯示一次
    assert overlay.messages == []
