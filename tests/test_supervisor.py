"""supervisor：遊戲視窗列舉 → 編號配置 → 起／收 reader 執行緒 → 多客戶端模式一次性觸發。
以假列舉與假執行緒驗證，不需遊戲。"""
import threading
import time

from src.reader import supervisor
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
            for ev in self.gone.values():   # 跑完前先放行，shutdown join 才不用等 JOIN_TIMEOUT
                ev.set()
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


def test_a_raising_on_multi_client_retries_next_scan_instead_of_latching():
    # 先呼叫再鎖存：第一次 raise 不可讓通知永久跳過，第三輪已鎖存就不該再呼叫
    h = Harness([[0xA], [0xA, 0xB], [0xA, 0xB], [0xA, 0xB]])
    calls = {"n": 0}

    def on_multi_client():
        calls["n"] += 1
        if calls["n"] == 1:
            raise RuntimeError("boom")

    supervise(h.stop, h.spawn, h.enumerate, on_multi_client, FakeBoard(), interval=0.005)
    for ev in h.gone.values():
        ev.set()
    assert calls["n"] == 2


def test_shutdown_join_budget_is_shared_not_per_thread(monkeypatch):
    monkeypatch.setattr(supervisor, "JOIN_TIMEOUT", 0.2)
    stop = threading.Event()
    calls = {"n": 0}

    def enumerate():
        calls["n"] += 1
        if calls["n"] >= 2:
            stop.set()
        return [0xA, 0xB]

    def spawn(hwnd, slot):
        th = threading.Thread(target=lambda: threading.Event().wait(), daemon=True)
        th.start()
        return th

    start = time.monotonic()
    supervise(stop, spawn, enumerate, lambda: None, FakeBoard(), interval=0.005)
    elapsed = time.monotonic() - start
    assert elapsed < 0.35   # 兩條卡死的 thread 共用一個 0.2s 預算，不是各等 0.2s（0.4s）
