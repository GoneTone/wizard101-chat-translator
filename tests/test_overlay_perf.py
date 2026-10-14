"""分頁切換與新增訊息的效能門檻：走正式的 `OverlayWindow` 路徑（停在螢幕外），不 mock 畫布。"""
import statistics
import time

import pytest

from src.reader.chat_tabs import TabState
from src.ui.outlined_text import line_height
from src.ui.overlay import OverlayWindow

pytestmark = pytest.mark.xdist_group("overlay-perf")

COUNT = 1000
TABS = TabState(0, ((0, "A"), (1, "B")))
LONG = "這是一段普通長度的聊天測試訊息，寬畫布一行放得下"


def _overlay(root, max_messages=COUNT):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=max_messages, fade_seconds=0)
    ov.set_tabs(1, TABS)
    return ov


def _fill(ov, text="hello"):
    for i in range(COUNT):
        ov.add_message(f"{text} {i}", f"{text} 譯文 {i}", slot=1, tabs=(0, 1))
    ov._win.update_idletasks()


def _switch(ov, tab):
    start = time.perf_counter()
    ov.set_tabs(1, TabState(tab, TABS.tabs))
    ov._win.update_idletasks()
    return time.perf_counter() - start


def test_switching_tabs_is_independent_of_message_count(root):
    """兩分頁各 1000 則，來回切換的中位數 ≤ 0.1 秒。"""
    ov = _overlay(root)
    _fill(ov)
    times = [_switch(ov, (i + 1) % 2) for i in range(5)]
    assert statistics.median(times) <= 0.100, times


def test_adding_a_message_stays_cheap(root):
    """顯示中分頁連續新增 1000 則，平均每則 ≤ 3 ms。"""
    ov = _overlay(root)
    start = time.perf_counter()
    for i in range(COUNT):
        ov.add_message(f"hello {i}", f"譯文 {i}", slot=1, tabs=(0,))
    ov._win.update_idletasks()
    mean = (time.perf_counter() - start) / COUNT
    assert mean <= 0.003, mean


def test_rewrapping_a_stale_tab_is_bounded(root):
    """隱藏分頁的換行寬度過期後切過去，1000 則的重排 ≤ 2.5 秒。"""
    ov = _overlay(root)
    _fill(ov, LONG)
    records = ov._list._views[(1, 1)].records
    before = [line_height(ln) for m in records for ln in m.lines]

    ov._apply_geometry(10000, 10000, 240, 300)
    ov._win.update_idletasks()
    elapsed = _switch(ov, 1)
    assert [line_height(ln) for m in records for ln in m.lines] != before, "行高沒變就量不到重排"
    assert elapsed <= 2.5, elapsed
