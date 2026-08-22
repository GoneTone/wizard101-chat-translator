"""reader_loop 行為:WizChatReader.read_new() → pending 佇列 → 翻譯 → overlay。
離線時失敗行留在 pending 下輪續翻;找不到遊戲顯示橫幅;非 HTTP 錯誤跳過該行。"""
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
        self.statuses: list[str] = []

    def add_message(self, original, translated):
        self.messages.append((original, translated))

    def set_error(self, text):
        self.errors.append(text)

    def clear_error(self):
        self.clears += 1

    def set_status(self, text, color=None):
        self.statuses.append(text)


class FakeReader:
    """依序回傳 reads[i](每輪新增的行清單);跑完設 stop。GameNotRunning 以例外物件表示。"""

    def __init__(self, reads, stop, anchored=True):
        self.reads = reads
        self.stop = stop
        self.anchored = anchored
        self.n = 0

    def read_new(self):
        i = self.n
        self.n += 1
        if self.n >= len(self.reads):
            self.stop.set()
        r = self.reads[i]
        if isinstance(r, Exception):
            raise r
        return list(r)

    def close(self):
        pass


def run_scripted(cfg, translator, overlay, reads, monkeypatch):
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    monkeypatch.setattr(main_module, "WizChatReader",
                        lambda **kw: FakeReader(reads, stop))
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

    def translate_incoming(self, text):
        self.calls.append(text)
        return f"譯:{text}"


def test_new_lines_translated_in_order_including_repeats(monkeypatch):
    cfg = {"poll_interval": 0.01}
    tr = OkTranslator()
    ov = FakeOverlay()
    reads = [[], ["[A] a"], ["[B] hi", "[B] hi"], []]
    run_scripted(cfg, tr, ov, reads, monkeypatch)
    assert tr.calls == ["[A] a", "[B] hi", "[B] hi"]
    assert ov.messages == [("[A] a", "譯:[A] a"), ("[B] hi", "譯:[B] hi"),
                           ("[B] hi", "譯:[B] hi")]


class OneBadTranslator:
    def __init__(self):
        self.calls: list[str] = []

    def translate_incoming(self, text):
        self.calls.append(text)
        if text == "[B] bad":
            raise ValueError("模型回傳非預期格式")
        return f"譯:{text}"


def test_non_http_error_skips_line_and_keeps_going(monkeypatch):
    cfg = {"poll_interval": 0.01}
    tr = OneBadTranslator()
    ov = FakeOverlay()
    reads = [["[A] a", "[B] bad", "[C] c"], []]
    run_scripted(cfg, tr, ov, reads, monkeypatch)
    assert tr.calls == ["[A] a", "[B] bad", "[C] c"]
    assert ov.messages == [("[A] a", "譯:[A] a"), ("[C] c", "譯:[C] c")]


class FlakyTranslator:
    def __init__(self):
        self.calls = 0

    def translate_incoming(self, text):
        self.calls += 1
        if self.calls == 1:
            raise httpx.HTTPError("offline")
        return f"譯:{text}"


def test_failed_line_stays_pending_and_retried(monkeypatch):
    monkeypatch.setattr(main_module, "BACKOFF_STEPS", [0.01, 0.01, 0.01])
    cfg = {"poll_interval": 0.01}
    tr = FlakyTranslator()
    ov = FakeOverlay()
    reads = [["[X] x"], [], []]
    run_scripted(cfg, tr, ov, reads, monkeypatch)
    assert tr.calls == 2                      # 第一次離線,第二輪重試同一行
    assert ov.messages == [("[X] x", "譯:[X] x")]
    assert ov.errors == ["⚠  翻譯伺服器離線，重試中…"]
    assert ov.clears == 1


def test_status_transitions(monkeypatch):
    # 監聽 →(有新訊息)翻譯中 → 監聽
    cfg = {"poll_interval": 0.01}
    tr = OkTranslator()
    ov = FakeOverlay()
    reads = [[], ["[A] a"], []]
    run_scripted(cfg, tr, ov, reads, monkeypatch)
    assert ov.statuses == ["●  監聽中", "●  翻譯中…", "●  監聽中"]


def test_status_locating_when_not_anchored(monkeypatch):
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    monkeypatch.setattr(main_module, "WizChatReader",
                        lambda **kw: FakeReader([[], []], stop, anchored=False))
    reader_loop(cfg, OkTranslator(), ov, ui_queue, stop)
    _drain(ui_queue)
    assert ov.statuses == ["●  連線遊戲中…"]  # 狀態未變不重複發


class NeverTranslator:
    def translate_incoming(self, text):
        raise AssertionError("找不到遊戲時不應嘗試翻譯")


def test_game_not_running_shows_banner_once(monkeypatch):
    monkeypatch.setattr(main_module, "GAME_MISSING_INTERVAL", 0.01)
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    reads = [GameNotRunning("no game"), GameNotRunning("no game")]
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    monkeypatch.setattr(main_module, "WizChatReader",
                        lambda **kw: FakeReader(reads, stop))
    reader_loop(cfg, NeverTranslator(), ov, ui_queue, stop)
    _drain(ui_queue)
    assert ov.errors == ["⚠  遊戲未就緒／連線中斷，等待中…"]
    assert ov.messages == []
    assert "●  等待遊戲中…" in ov.statuses
