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


def _settle(ov):
    ov._win.update_idletasks()


def _resize(ov, width):
    ov._apply_geometry(10000, 10000, width, 300)
    _settle(ov)


def _fill(ov, text="hello", count=COUNT, tabs=(0, 1)):
    for i in range(count):
        ov.add_message(f"{text} {i}", f"{text} 譯文 {i}", slot=1, tabs=tabs)
    _settle(ov)


def _switch(ov, tab, clock=time.perf_counter):
    start = clock()
    ov.set_tabs(1, TabState(tab, TABS.tabs))
    _settle(ov)
    return clock() - start


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


def _stale_rewrap_seconds(ov, width):
    """把窗寬改成 width 再切到分頁 1 的重排 CPU 時間（本程序的，不受其他 worker 搶佔灌水）。"""
    records = ov._list._views[(1, 1)].records
    before = [line_height(ln) for m in records for ln in m.lines]
    _resize(ov, width)
    seconds = _switch(ov, 1, clock=time.process_time)
    assert [line_height(ln) for m in records for ln in m.lines] != before, "行高沒變就量不到重排"
    _switch(ov, 0)
    return seconds


def test_rewrapping_a_stale_tab_scales_linearly(root):
    """重排耗時與訊息數成線性：1000 則 / 250 則 ≤ 7（線性約 4、平方約 16），且 ≤ 10 秒。"""
    small, large = _stale_overlay(root, 250), _stale_overlay(root, 1000)
    pairs = [(_stale_rewrap_seconds(small, w), _stale_rewrap_seconds(large, w))
             for w in (240, 460, 240)]   # 兩者緊接著量，負載變動對兩邊一致；取比值最小的一輪
    t250, t1000 = min(pairs, key=lambda p: p[1] / p[0])
    print(f"REWRAP t250={t250:.3f} t1000={t1000:.3f} ratio={t1000 / t250:.2f}")
    assert t1000 / t250 <= 7, (t250, t1000)
    assert t1000 <= 10, t1000
