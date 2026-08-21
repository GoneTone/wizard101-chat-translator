"""reader_loop 行為:逐輪比對(這輪有、上輪沒有 = 新訊息)→ 翻譯 → overlay。
啟動當輪只記錄既有、不翻;離線時未試的行不併入 prev,下輪重試;找不到遊戲顯示橫幅。"""
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
    _drain(ui_queue)


def _drain(ui_queue):
    while True:
        try:
            ui_queue.get_nowait()()
        except queue.Empty:
            break


class OkTranslator:
    def __init__(self):
        self.calls: list[str] = []

    def to_zh(self, text):
        self.calls.append(text)
        return f"譯:{text}"


def test_startup_does_not_translate_existing(monkeypatch):
    cfg = {"poll_interval": 0.01}
    tr = OkTranslator()
    ov = FakeOverlay()
    run_scripted(cfg, tr, ov, [["[A] a", "[B] b"]], monkeypatch)
    assert tr.calls == []  # 啟動當輪不翻既有


def test_new_lines_translated_each_scan(monkeypatch):
    cfg = {"poll_interval": 0.01}
    tr = OkTranslator()
    ov = FakeOverlay()
    # 掃1 基準;掃2 [C] 新出現 → 翻;掃3 [D] 新出現 → 翻;[A][B][C] 不重翻
    reads = [["[A] a", "[B] b"], ["[A] a", "[B] b", "[C] c"], ["[A] a", "[B] b", "[C] c", "[D] d"]]
    run_scripted(cfg, tr, ov, reads, monkeypatch)
    assert tr.calls == ["[C] c", "[D] d"]
    assert ov.messages == [("[C] c", "譯:[C] c"), ("[D] d", "譯:[D] d")]


def test_line_that_disappears_then_returns_is_retranslated(monkeypatch):
    # 逐輪比對:一行消失後又出現會被視為新行(這是「偶爾冒舊訊息」的取捨)
    cfg = {"poll_interval": 0.01}
    tr = OkTranslator()
    ov = FakeOverlay()
    reads = [["[A] a"], ["[A] a", "[B] b"], ["[A] a"], ["[A] a", "[B] b"]]
    run_scripted(cfg, tr, ov, reads, monkeypatch)
    assert tr.calls == ["[B] b", "[B] b"]  # 出現、消失、再出現 → 翻兩次


class OneBadTranslator:
    def __init__(self):
        self.calls: list[str] = []

    def to_zh(self, text):
        self.calls.append(text)
        if text == "[B] bad":
            raise ValueError("模型回傳非預期格式")
        return f"譯:{text}"


def test_non_http_error_skips_line_and_keeps_going(monkeypatch):
    cfg = {"poll_interval": 0.01}
    tr = OneBadTranslator()
    ov = FakeOverlay()
    reads = [[], ["[A] a", "[B] bad", "[C] c"]]  # 掃1 空基準;掃2 三行新出現
    run_scripted(cfg, tr, ov, reads, monkeypatch)
    assert tr.calls == ["[A] a", "[B] bad", "[C] c"]  # 三行都嘗試
    assert ov.messages == [("[A] a", "譯:[A] a"), ("[C] c", "譯:[C] c")]  # bad 被跳過


class FlakyTranslator:
    def __init__(self):
        self.calls = 0

    def to_zh(self, text):
        self.calls += 1
        if self.calls == 1:
            raise httpx.HTTPError("offline")
        return f"譯:{text}"


def test_failed_line_retried_after_recovery(monkeypatch):
    monkeypatch.setattr(main_module, "BACKOFF_STEPS", [0.01, 0.01, 0.01])
    cfg = {"poll_interval": 0.01}
    tr = FlakyTranslator()
    ov = FakeOverlay()
    # 掃1 空基準;掃2 [X] 新出現→翻譯失敗(離線);掃3 [X] 仍在→重試成功
    reads = [[], ["[X] x"], ["[X] x"]]
    run_scripted(cfg, tr, ov, reads, monkeypatch)
    assert tr.calls == 2
    assert ov.messages == [("[X] x", "譯:[X] x")]
    assert ov.errors == ["⚠ 翻譯伺服器離線,重試中…"]
    assert ov.clears == 1


class NeverTranslator:
    def to_zh(self, text):
        raise AssertionError("找不到遊戲時不應嘗試翻譯")


def test_game_not_running_shows_banner_once(monkeypatch):
    monkeypatch.setattr(main_module, "GAME_MISSING_INTERVAL", 0.01)
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    count = {"n": 0}

    def fake_read(process_name="WizardGraphicalClient.exe"):
        count["n"] += 1
        if count["n"] >= 2:
            stop.set()
        raise GameNotRunning("no game")

    monkeypatch.setattr(main_module, "read_chat_lines", fake_read)
    reader_loop(cfg, NeverTranslator(), ov, ui_queue, stop)
    _drain(ui_queue)
    assert ov.errors == ["⚠ 找不到遊戲程序,等待中…"]
    assert ov.messages == []
