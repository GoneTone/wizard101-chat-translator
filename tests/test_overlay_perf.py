"""分頁切換與新增訊息的效能門檻：走正式的 `OverlayWindow` 路徑（停在螢幕外），不 mock 畫布。"""
import statistics
import time

import pytest

from src.reader.chat_tabs import TabState
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


def _settle(ov):
    ov._win.update_idletasks()


def _resize(ov, width):
    ov._apply_geometry(10000, 10000, width, 300)
    # update()（非 update_idletasks）：頂層視窗改尺寸後的 <Configure> 是事件，idle 佇列送不到畫布
    ov._win.update()


def _fill(ov, text="hello", count=COUNT, tabs=(0, 1)):
    for i in range(count):
        ov.add_message(f"{text} {i}", f"{text} 譯文 {i}", slot=1, tabs=tabs)
    _settle(ov)


def _switch(ov, tab):
    start = time.perf_counter()
    ov.set_tabs(1, TabState(tab, TABS.tabs))
    _settle(ov)
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
    _settle(ov)
    mean = (time.perf_counter() - start) / COUNT
    assert mean <= 0.003, mean


def _stale_overlay(root, count):
    """只有分頁 1 有 count 則長訊息的 overlay（分頁 0 空，換寬度時只有分頁 1 要重排）。"""
    ov = _overlay(root, max_messages=count)
    _fill(ov, LONG, count, tabs=(1,))
    return ov


def test_the_synchronous_part_of_a_resize_is_independent_of_message_count(root):
    """縮放的同步部分：1000 則中位數 ≤ 0.2 秒（CPU），1000／250 ≤ 2。"""
    small, large = _stale_overlay(root, 250), _stale_overlay(root, 1000)
    _switch(small, 1)
    _switch(large, 1)

    def cpu(ov, width):
        start = time.process_time()
        _resize(ov, width)
        return time.process_time() - start

    pairs = [(cpu(small, w), cpu(large, w)) for w in (240, 460, 240, 460, 240)]
    t1000 = statistics.median(p[1] for p in pairs)
    assert t1000 <= 0.200, pairs
    assert t1000 / max(statistics.median(p[0] for p in pairs), 1e-3) <= 2, pairs
