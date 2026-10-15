"""drain_ui_queue：單一回呼拋錯不能讓佇列裡其餘回呼被跳過，也不能讓例外往外傳。"""
import queue

from src.main import drain_ui_queue


def test_raising_callback_does_not_propagate():
    q: queue.Queue = queue.Queue()

    def boom():
        raise ValueError("bad callback")

    q.put(boom)
    drain_ui_queue(q)  # 不應該拋出例外
    assert q.empty()


def test_later_callbacks_still_run_after_one_raises():
    q: queue.Queue = queue.Queue()
    calls = []

    def boom():
        raise RuntimeError("bad")

    def ok_one():
        calls.append("one")

    def ok_two():
        calls.append("two")

    q.put(boom)
    q.put(ok_one)
    q.put(ok_two)

    drain_ui_queue(q)

    assert calls == ["one", "two"]
    assert q.empty()


def test_empty_queue_is_a_noop():
    q: queue.Queue = queue.Queue()
    drain_ui_queue(q)  # 不應該拋出例外
    assert q.empty()
