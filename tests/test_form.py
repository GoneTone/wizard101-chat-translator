"""form.BackgroundButton：按鈕觸發的背景工作與過期結果防護。"""
import threading
import time
from tkinter import ttk

from src.ui.form import BackgroundButton, collapsible


def _pump_until(root, predicate, seconds: float = 30) -> None:
    deadline = time.monotonic() + seconds
    while not predicate() and time.monotonic() < deadline:
        root.update()
        time.sleep(0.01)


def test_start_disables_the_button_and_delivers_the_result_on_the_main_thread(root):
    btn = ttk.Button(root, text="idle")
    task = BackgroundButton(btn, "probe")
    release = threading.Event()
    seen: list = []

    def work():
        release.wait()   # 不設上限：語意就是「測試放行前絕不回應」
        return "done"

    task.start(work, seen.append, "busy")
    root.update()
    assert str(btn.cget("state")) == "disabled"
    assert btn.cget("text") == "busy"

    release.set()
    _pump_until(root, lambda: seen)
    assert seen == ["done"]
    assert str(btn.cget("state")) == "normal"
    assert btn.cget("text") == "idle"


def test_an_exception_in_work_is_handed_to_on_done(root):
    btn = ttk.Button(root, text="idle")
    task = BackgroundButton(btn, "probe")
    seen: list = []

    def boom():
        raise RuntimeError("HTTP 403")

    task.start(boom, seen.append, "busy")
    _pump_until(root, lambda: seen)
    assert isinstance(seen[0], RuntimeError)
    assert str(btn.cget("state")) == "normal"   # 失敗也要把按鈕還原，不能卡在「進行中」


def test_a_result_that_arrives_after_invalidate_is_discarded_but_the_button_recovers(root):
    btn = ttk.Button(root, text="idle")
    task = BackgroundButton(btn, "probe")
    seen: list = []
    session = task.session
    btn.configure(state="disabled", text="busy")
    task.invalidate()
    task._finish("stale", session)
    assert seen == []
    assert str(btn.cget("state")) == "normal"


def test_a_slow_round_cannot_land_in_a_later_round(root):
    """兩輪重疊：第一輪還沒回來就再啟動一輪，舊結果回來時只還原按鈕、不交給 on_done；
    畫面只看到新一輪的結果。"""
    btn = ttk.Button(root, text="idle")
    task = BackgroundButton(btn, "probe")
    release_stale, release_current = threading.Event(), threading.Event()
    seen: list = []

    def stale():
        release_stale.wait()
        return "stale"

    def current():
        release_current.wait()
        return "current"

    task.start(stale, seen.append, "busy")
    task.start(current, seen.append, "busy")
    release_stale.set()
    _pump_until(root, lambda: str(btn.cget("state")) == "normal", seconds=5)
    assert seen == []   # 舊結果被丟掉

    release_current.set()
    _pump_until(root, lambda: seen)
    assert seen == ["current"]


def test_collapsible_starts_collapsed(root):
    body = collapsible(root, "進階")
    assert body.is_expanded() is False
    assert body.winfo_manager() == ""   # 真的沒被 pack —— withdrawn 的 root 下 ismapped 恆為 0，驗不出東西


def test_collapsible_can_start_expanded(root):
    body = collapsible(root, "進階", expanded=True)
    assert body.is_expanded()
    assert body.winfo_manager() == "pack"


def test_collapsible_toggles(root):
    body = collapsible(root, "進階")
    # holder 是直接 pack 在 root 的，body 也是 holder 的子元件；header 是 holder 底下的第一個子元件
    header = body.master.winfo_children()[0]
    assert header.cget("text") == "▸ 進階"

    body.toggle()
    assert body.is_expanded()
    assert body.winfo_manager() == "pack"
    assert header.cget("text") == "▾ 進階"

    body.toggle()
    assert not body.is_expanded()
    assert body.winfo_manager() == ""
    assert header.cget("text") == "▸ 進階"
