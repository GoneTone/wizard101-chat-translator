"""reader_loop 行為:讀可視聊天視窗 → 結尾新增偵測(含重複、依序)→ 翻譯 → overlay。
啟動只記錄視窗、不翻;離線時失敗那行下輪重試;找不到遊戲顯示橫幅。"""
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


def run_scripted(cfg, translator, overlay, reads, monkeypatch):
    """每輪 read_visible_chat 依序回傳 reads[i](各為一個視窗行清單);跑完停止。"""
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    count = {"n": 0}

    def fake_win(process_name="WizardGraphicalClient.exe"):
        i = count["n"]
        count["n"] += 1
        if count["n"] >= len(reads):
            stop.set()
        return list(reads[i])

    monkeypatch.setattr(main_module, "read_visible_chat", fake_win)
    reader_loop(cfg, translator, overlay, ui_queue, stop)
    _drain(ui_queue)


def _drain(ui_queue):
    while True:
        try:
            ui_queue.get_nowait()()
        except queue.Empty:
            break


# --- appended_lines 純函式 ---
def test_appended_after_scroll():
    assert appended_lines(["a", "b", "c"], ["b", "c", "d"]) == ["d"]


def test_appended_none_when_unchanged():
    assert appended_lines(["a", "b", "c"], ["a", "b", "c"]) == []


def test_appended_handles_repeats():
    # 視窗尾端本來是 np,又送一次 np → 應偵測到新的那句 np
    assert appended_lines(["x", "y", "np"], ["y", "np", "np"]) == ["np"]


def test_appended_no_overlap_returns_empty():
    # 對不齊(視窗劇烈變動)→ 不翻,只重新對齊,避免整窗爆量重譯
    assert appended_lines(["a", "b"], ["x", "y", "z"]) == []


def test_appended_empty_prev_returns_empty():
    assert appended_lines([], ["a", "b"]) == []


# --- reader_loop ---
class OkTranslator:
    def __init__(self):
        self.calls: list[str] = []

    def to_zh(self, text):
        self.calls.append(text)
        return f"譯:{text}"


def test_startup_records_window_without_translating(monkeypatch):
    cfg = {"poll_interval": 0.01}
    tr = OkTranslator()
    ov = FakeOverlay()
    run_scripted(cfg, tr, ov, [["[A] a", "[B] b"]], monkeypatch)
    assert tr.calls == []


def test_appended_messages_translated_in_order(monkeypatch):
    cfg = {"poll_interval": 0.01}
    tr = OkTranslator()
    ov = FakeOverlay()
    reads = [["[A] a", "[B] b"], ["[A] a", "[B] b", "[C] c"], ["[B] b", "[C] c", "[D] d"]]
    run_scripted(cfg, tr, ov, reads, monkeypatch)
    assert tr.calls == ["[C] c", "[D] d"]


def test_repeated_message_is_translated_again(monkeypatch):
    # 遊戲聊天室出現重複的同一句 → 也要翻(不去重)
    cfg = {"poll_interval": 0.01}
    tr = OkTranslator()
    ov = FakeOverlay()
    reads = [["[A] hi"], ["[A] hi", "[A] hi"], ["[A] hi", "[A] hi", "[A] hi"]]
    run_scripted(cfg, tr, ov, reads, monkeypatch)
    assert tr.calls == ["[A] hi", "[A] hi"]  # 第二、三次的重複都翻


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
    reads = [["[Z] base"], ["[Z] base", "[A] a", "[B] bad", "[C] c"]]
    run_scripted(cfg, tr, ov, reads, monkeypatch)
    assert tr.calls == ["[A] a", "[B] bad", "[C] c"]
    assert ov.messages == [("[A] a", "譯:[A] a"), ("[C] c", "譯:[C] c")]


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
    reads = [["[Z] base"], ["[Z] base", "[X] x"], ["[Z] base", "[X] x"]]
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

    def fake_win(process_name="WizardGraphicalClient.exe"):
        count["n"] += 1
        if count["n"] >= 2:
            stop.set()
        raise GameNotRunning("no game")

    monkeypatch.setattr(main_module, "read_visible_chat", fake_win)
    reader_loop(cfg, NeverTranslator(), ov, ui_queue, stop)
    _drain(ui_queue)
    assert ov.errors == ["⚠ 找不到遊戲程序,等待中…"]
    assert ov.messages == []
