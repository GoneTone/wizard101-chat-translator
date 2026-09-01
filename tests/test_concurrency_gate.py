"""ConcurrencyGate：翻譯請求的總量上限（兩個 pool 共用一個閘）。"""
import threading

from src.concurrency_gate import ConcurrencyGate


def test_allows_up_to_the_limit():
    gate = ConcurrencyGate(2)
    stop = threading.Event()
    assert gate.acquire(stop) is True
    assert gate.acquire(stop) is True
    assert gate.in_use == 2


def test_blocks_beyond_the_limit_until_released():
    gate = ConcurrencyGate(1)
    stop = threading.Event()
    assert gate.acquire(stop) is True
    got = threading.Event()

    def waiter():
        if gate.acquire(stop):
            got.set()

    t = threading.Thread(target=waiter, daemon=True)
    t.start()
    assert not got.wait(0.2), "額度已滿時不該放行"
    gate.release()
    assert got.wait(2.0), "額度釋出後等待者要被喚醒"
    t.join(timeout=2)


def test_stop_event_aborts_a_waiter():
    gate = ConcurrencyGate(1)
    stop = threading.Event()
    assert gate.acquire(stop) is True
    result = []

    def waiter():
        result.append(gate.acquire(stop))

    t = threading.Thread(target=waiter, daemon=True)
    t.start()
    stop.set()
    t.join(timeout=2)
    assert result == [False], "關閉中應放棄等待、回傳 False"


def test_raising_the_limit_wakes_waiters():
    gate = ConcurrencyGate(1)
    stop = threading.Event()
    gate.acquire(stop)
    got = threading.Event()

    def waiter():
        if gate.acquire(stop):
            got.set()

    t = threading.Thread(target=waiter, daemon=True)
    t.start()
    assert not got.wait(0.2)
    gate.set_limit(2)
    assert got.wait(2.0), "上限調大後等待者要被喚醒"
    t.join(timeout=2)


def test_lowering_the_limit_does_not_overcommit():
    gate = ConcurrencyGate(4)
    stop = threading.Event()
    for _ in range(4):
        gate.acquire(stop)
    gate.set_limit(2)
    assert gate.in_use == 4, "已發出的額度不會被收回"
    gate.release()
    gate.release()
    gate.release()
    assert gate.in_use == 1
    assert gate.acquire(stop) is True   # 降到 2 之後，1 → 2 仍可再取一個
    assert gate.in_use == 2
