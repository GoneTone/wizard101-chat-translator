"""ChatContext：跨執行緒共用的近期原文行緩衝。"""
import threading

from src.translation.context import CONTEXT_LINES, ChatContext


def test_snapshot_returns_pushed_lines_in_order():
    ctx = ChatContext()
    ctx.push("[A] one")
    ctx.push("[B] two")
    assert ctx.snapshot() == ["[A] one", "[B] two"]


def test_snapshot_is_a_copy():
    # 呼叫端拿到的 snapshot 之後不得被新 push 影響 —— worker 帶著它重試時上下文不能漂移
    ctx = ChatContext()
    ctx.push("[A] one")
    snap = ctx.snapshot()
    ctx.push("[B] two")
    assert snap == ["[A] one"]


def test_oldest_lines_drop_beyond_capacity():
    ctx = ChatContext()
    for i in range(CONTEXT_LINES + 3):
        ctx.push(f"[A] m{i}")
    lines = ctx.snapshot()
    assert len(lines) == CONTEXT_LINES
    assert "[A] m0" not in lines
    assert f"[A] m{CONTEXT_LINES + 2}" in lines


def test_concurrent_pushes_do_not_lose_lines():
    # reader 執行緒、翻譯 worker、發話執行緒共用同一份，併發 push 不得掉行
    ctx = ChatContext(max_lines=1000)
    start = threading.Event()

    def worker(tag):
        start.wait()
        for i in range(100):
            ctx.push(f"[{tag}] {i}")

    threads = [threading.Thread(target=worker, args=(t,)) for t in "ABCD"]
    for t in threads:
        t.start()
    start.set()
    for t in threads:
        t.join()
    assert len(ctx.snapshot()) == 400
