"""reader_loop 離線恢復行為:翻譯失敗的行(與同批未試的行)下一輪要重新嘗試,
橫幅只在還在離線時顯示,只有真的翻譯成功過才清除橫幅、重置退避。"""
import queue
import threading

import httpx

import src.main as main_module
from src.main import reader_loop


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


def run_cycles(cfg, translator, overlay, stop_after_cycle: int):
    """跑 reader_loop 到第 stop_after_cycle 輪結束就停止,並把 ui_queue 的回呼全部執行掉
    (模擬 pump()),回傳 ui_queue(已清空)。"""
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    count = {"n": 0}

    def fake_hidden():
        count["n"] += 1
        if count["n"] >= stop_after_cycle:
            stop.set()
        return False

    orig_hidden = main_module.game_window_hidden
    orig_grab = main_module.grab_region
    orig_recognize = main_module.recognize_lines
    try:
        main_module.game_window_hidden = fake_hidden
        main_module.grab_region = lambda region: object()
        main_module.recognize_lines = lambda img: ["hello there"]
        reader_loop(cfg, translator, overlay, ui_queue, stop)
    finally:
        main_module.game_window_hidden = orig_hidden
        main_module.grab_region = orig_grab
        main_module.recognize_lines = orig_recognize

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
    cfg = {"poll_interval": 0.01, "chat_region": (0, 0, 10, 10)}
    translator = FlakyTranslator()
    overlay = FakeOverlay()

    run_cycles(cfg, translator, overlay, stop_after_cycle=2)

    assert translator.calls == 2
    assert overlay.messages == [("hello there", "譯:hello there")]


def test_banner_set_while_offline_and_cleared_only_after_success(monkeypatch):
    monkeypatch.setattr(main_module, "BACKOFF_STEPS", [0.01, 0.01, 0.01])
    cfg = {"poll_interval": 0.01, "chat_region": (0, 0, 10, 10)}
    translator = FlakyTranslator()
    overlay = FakeOverlay()

    run_cycles(cfg, translator, overlay, stop_after_cycle=2)

    assert overlay.errors == ["⚠ 翻譯伺服器離線,重試中…"]
    assert overlay.clears == 1


class BatchFlakyTranslator:
    """整批第一行失敗一次(模擬離線),之後全部成功。用來驗證同批未試的行也一起重試。"""

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
    cfg = {"poll_interval": 0.01, "chat_region": (0, 0, 10, 10)}
    translator = BatchFlakyTranslator()
    overlay = FakeOverlay()

    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    count = {"n": 0}

    def fake_hidden():
        count["n"] += 1
        if count["n"] >= 2:
            stop.set()
        return False

    monkeypatch.setattr(main_module, "game_window_hidden", fake_hidden)
    monkeypatch.setattr(main_module, "grab_region", lambda region: object())
    monkeypatch.setattr(
        main_module, "recognize_lines", lambda img: ["line one", "line two", "line three"]
    )

    reader_loop(cfg, translator, overlay, ui_queue, stop)

    while True:
        try:
            ui_queue.get_nowait()()
        except queue.Empty:
            break

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
