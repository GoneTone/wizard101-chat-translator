"""選取層：拖曳→矩形（螢幕座標）、點一下→取消、Esc→取消、凍結畫面顯示與聚光燈、
backdrop 暗化背景、提示文字自己一層視窗。"""
import pytest
from PIL import Image

from src.i18n import t
from src.region.capture import Frame
from src.ui import region_select as select_module
from src.ui.region_select import RegionSelector

_MONITOR = (1920, 0, 1600, 900)   # 第二顆螢幕：矩形要加上螢幕原點
# 遊戲 client 區左上角落在螢幕 (1920 + 100, 50)，換算成這顆螢幕的 canvas 座標是 (100, 50)
_FRAME = Frame(Image.new("RGB", (400, 300), "white"),
              client_origin=(1920 + 100, 50), client_size=(400, 300))


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
    selector.show(_MONITOR, _FRAME, on_select=picked.append, on_cancel=lambda: cancelled.append(1))
    root.update()
    assert selector.is_open
    _drag(selector, root, 400, 300, 100, 150)   # 從右下拖到左上也要正規化
    assert picked == [(1920 + 100, 150, 300, 150)]
    assert cancelled == [] and not selector.is_open


def test_a_click_without_drag_cancels(selector, root):
    picked, cancelled = [], []
    selector.show(_MONITOR, _FRAME, on_select=picked.append, on_cancel=lambda: cancelled.append(1))
    root.update()
    _drag(selector, root, 100, 100, 102, 101)
    assert picked == [] and cancelled == [1] and not selector.is_open


def test_escape_cancels(selector, root):
    picked, cancelled = [], []
    selector.show(_MONITOR, _FRAME, on_select=picked.append, on_cancel=lambda: cancelled.append(1))
    root.update()
    selector._win.event_generate("<Escape>")
    root.update()
    assert picked == [] and cancelled == [1] and not selector.is_open


def test_show_while_open_replaces_the_previous_layer(selector, root):
    selector.show(_MONITOR, _FRAME, on_select=lambda r: None)
    first = selector._win
    selector.show(_MONITOR, _FRAME, on_select=lambda r: None)
    root.update()
    assert selector._win is not first and selector.is_open


def test_rubber_band_is_drawn_while_dragging(selector, root):
    selector.show(_MONITOR, _FRAME, on_select=lambda r: None)
    root.update()
    canvas = selector._canvas
    canvas.event_generate("<ButtonPress-1>", x=10, y=20)
    canvas.event_generate("<B1-Motion>", x=110, y=70)
    root.update()
    assert [int(v) for v in canvas.coords(selector._band)] == [10, 20, 110, 70]


def test_show_displays_the_frozen_frame_at_full_opacity(selector, root):
    selector.show(_MONITOR, _FRAME, on_select=lambda r: None)
    root.update()
    assert float(selector._win.attributes("-alpha")) == 1.0


def test_dim_image_is_placed_at_the_games_client_origin(selector, root):
    selector.show(_MONITOR, _FRAME, on_select=lambda r: None)
    root.update()
    dim_item = selector._canvas.find_all()[0]
    assert [int(v) for v in selector._canvas.coords(dim_item)] == [100, 50]


def test_spotlight_shows_the_dragged_area_and_hides_on_the_next_press(selector, root):
    selector.show(_MONITOR, _FRAME, on_select=lambda r: None)
    root.update()
    canvas = selector._canvas
    canvas.event_generate("<ButtonPress-1>", x=120, y=80)
    canvas.event_generate("<B1-Motion>", x=220, y=150)
    root.update()
    assert canvas.itemcget(selector._spot, "state") == "normal"
    assert [int(v) for v in canvas.coords(selector._spot)] == [
        int(v) for v in canvas.coords(selector._band)[:2]]
    canvas.event_generate("<ButtonPress-1>", x=130, y=90)
    root.update()
    assert canvas.itemcget(selector._spot, "state") == "hidden"


def test_backdrop_is_placed_below_the_game_frame_when_given(selector, root):
    backdrop = Image.new("RGB", (1600, 900), "white")
    selector.show(_MONITOR, _FRAME, on_select=lambda r: None, backdrop=backdrop)
    root.update()
    items = selector._canvas.find_all()
    assert [int(v) for v in selector._canvas.coords(items[0])] == [0, 0]
    assert [int(v) for v in selector._canvas.coords(items[1])] == [100, 50]


def test_hint_shows_in_its_own_opaque_window(selector, root):
    # 提示文字不能畫在明暗變化的底圖上（見 region_select.py 的說明），要另開一層不透明視窗
    selector.show(_MONITOR, _FRAME, on_select=lambda r: None)
    root.update()
    assert selector._hint is not None and selector._hint.winfo_exists()
    assert selector._hint_label.cget("text") == t("region.hint")


def test_hint_window_closes_with_the_layer(selector, root):
    selector.show(_MONITOR, _FRAME, on_select=lambda r: None)
    root.update()
    selector.cancel()
    assert selector._hint is None


def test_hint_window_closes_after_a_completed_drag(selector, root):
    picked = []
    selector.show(_MONITOR, _FRAME, on_select=picked.append)
    root.update()
    _drag(selector, root, 400, 300, 100, 150)
    assert picked and selector._hint is None


def test_show_while_open_leaves_exactly_one_hint_window(selector, root):
    selector.show(_MONITOR, _FRAME, on_select=lambda r: None)
    first_hint = selector._hint
    selector.show(_MONITOR, _FRAME, on_select=lambda r: None)
    root.update()
    assert not first_hint.winfo_exists()
    assert selector._hint is not None and selector._hint.winfo_exists()
