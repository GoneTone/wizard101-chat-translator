"""選取層：拖曳→矩形（螢幕座標）、點一下→取消、Esc→取消。"""
import pytest

from src.ui import region_select as select_module
from src.ui.region_select import RegionSelector

_MONITOR = (1920, 0, 1600, 900)   # 第二顆螢幕：矩形要加上螢幕原點


@pytest.fixture
def selector(root, monkeypatch):
    monkeypatch.setattr(select_module, "force_foreground", lambda hwnd: None)
    monkeypatch.setattr(RegionSelector, "_take_focus", lambda self: None)
    s = RegionSelector(root)
    yield s
    s.cancel()


def _drag(selector, root, x0, y0, x1, y1):
    canvas = selector._canvas
    canvas.event_generate("<ButtonPress-1>", x=x0, y=y0)
    canvas.event_generate("<B1-Motion>", x=x1, y=y1)
    root.update()
    canvas.event_generate("<ButtonRelease-1>", x=x1, y=y1)
    root.update()


def test_drag_reports_a_normalized_rect_in_screen_coordinates(selector, root):
    picked, cancelled = [], []
    selector.show(_MONITOR, on_select=picked.append, on_cancel=lambda: cancelled.append(1))
    root.update()
    assert selector.is_open
    _drag(selector, root, 400, 300, 100, 150)   # 從右下拖到左上也要正規化
    assert picked == [(1920 + 100, 150, 300, 150)]
    assert cancelled == [] and not selector.is_open


def test_a_click_without_drag_cancels(selector, root):
    picked, cancelled = [], []
    selector.show(_MONITOR, on_select=picked.append, on_cancel=lambda: cancelled.append(1))
    root.update()
    _drag(selector, root, 100, 100, 102, 101)
    assert picked == [] and cancelled == [1] and not selector.is_open


def test_escape_cancels(selector, root):
    picked, cancelled = [], []
    selector.show(_MONITOR, on_select=picked.append, on_cancel=lambda: cancelled.append(1))
    root.update()
    selector._win.event_generate("<Escape>")
    root.update()
    assert picked == [] and cancelled == [1] and not selector.is_open


def test_show_while_open_replaces_the_previous_layer(selector, root):
    selector.show(_MONITOR, on_select=lambda r: None)
    first = selector._win
    selector.show(_MONITOR, on_select=lambda r: None)
    root.update()
    assert selector._win is not first and selector.is_open


def test_rubber_band_is_drawn_while_dragging(selector, root):
    selector.show(_MONITOR, on_select=lambda r: None)
    root.update()
    canvas = selector._canvas
    canvas.event_generate("<ButtonPress-1>", x=10, y=20)
    canvas.event_generate("<B1-Motion>", x=110, y=70)
    root.update()
    assert [int(v) for v in canvas.coords(selector._band)] == [10, 20, 110, 70]
