"""框選框：框選後留在畫面上的可調整矩形 —— 拖角落改兩軸、拖邊線只拉該邊、拖框內移動、
點一下不算、放開才回報新矩形；框線視窗只有線與把手是實體（透明色鍵），框內另有一片
幾乎全透明的視窗接移動的拖曳。"""
import pytest

from src.ui.palette import FG_UPDATE
from src.ui.region_box import BAND, HANDLE, MARGIN, MIN_SIZE, RegionBox, hit_at

_RECT = (100, 100, 300, 200)


@pytest.fixture
def box(root):
    changes = []
    b = RegionBox(root)
    b.on_change = changes.append
    b.changes = changes
    yield b
    b.hide()


def _drag(box, root, x, y, dx, dy):
    """在 canvas 座標 (x, y) 按下，拖 (dx, dy) 後放開；螢幕座標另給，實作靠它算位移。"""
    canvas = box._canvas
    canvas.event_generate("<ButtonPress-1>", x=x, y=y, rootx=1000 + x, rooty=1000 + y)
    canvas.event_generate("<B1-Motion>", x=x + dx, y=y + dy,
                          rootx=1000 + x + dx, rooty=1000 + y + dy)
    root.update()
    canvas.event_generate("<ButtonRelease-1>", x=x + dx, y=y + dy,
                          rootx=1000 + x + dx, rooty=1000 + y + dy)
    root.update()


def test_show_opens_a_window_around_the_rect_with_room_for_the_handles(box, root):
    assert not box.is_open
    box.show(_RECT)
    root.update()
    assert box.is_open and box.rect() == _RECT
    assert box._win.geometry().split("+")[0] == f"{300 + 2 * MARGIN}x{200 + 2 * MARGIN}"


def test_dragging_a_corner_handle_resizes_and_reports_the_new_rect(box, root):
    box.show(_RECT)
    root.update()
    _drag(box, root, MARGIN + 300, MARGIN + 200, 40, 30)   # 右下角把手
    assert box.rect() == (100, 100, 340, 230)
    assert box.changes == [(100, 100, 340, 230)]


def test_dragging_an_edge_line_moves_only_that_edge(box, root):
    box.show(_RECT)
    root.update()
    _drag(box, root, MARGIN + 60, MARGIN, 20, 10)   # 上緣框線：只有上緣往下 10，左右不動
    assert box.rect() == (100, 110, 300, 190)
    assert box.changes == [(100, 110, 300, 190)]


def test_dragging_inside_the_box_moves_it(box, root):
    box.show(_RECT)
    root.update()
    grip = box._grip
    grip.event_generate("<ButtonPress-1>", x=50, y=50, rootx=1150, rooty=1150)
    grip.event_generate("<B1-Motion>", x=70, y=60, rootx=1170, rooty=1160)
    root.update()
    grip.event_generate("<ButtonRelease-1>", x=70, y=60, rootx=1170, rooty=1160)
    root.update()
    assert box.rect() == (120, 110, 300, 200)
    assert box.changes == [(120, 110, 300, 200)]


def test_the_grip_is_nearly_invisible_and_fills_the_inside_of_the_line(box, root):
    box.show(_RECT)
    root.update()
    assert 0 < float(box._grip.attributes("-alpha")) <= 0.02
    assert box._grip.geometry().split("+")[0] == f"{300 - 2 * BAND}x{200 - 2 * BAND}"


def test_a_click_on_the_band_changes_nothing(box, root):
    box.show(_RECT)
    root.update()
    _drag(box, root, MARGIN + 60, MARGIN, 2, 1)
    assert box.rect() == _RECT and box.changes == []


def test_the_opposite_corner_stays_put_and_the_size_never_drops_below_the_minimum(box, root):
    box.show(_RECT)
    root.update()
    _drag(box, root, MARGIN, MARGIN, 1000, 1000)   # 左上角把手往右下拖過頭
    assert box.rect() == (100 + 300 - MIN_SIZE, 100 + 200 - MIN_SIZE, MIN_SIZE, MIN_SIZE)


def test_the_window_follows_the_box_while_dragging(box, root):
    box.show(_RECT)
    root.update()
    canvas = box._canvas
    canvas.event_generate("<ButtonPress-1>", x=MARGIN + 300, y=MARGIN + 200,
                          rootx=1305, rooty=1205)
    canvas.event_generate("<B1-Motion>", x=MARGIN + 340, y=MARGIN + 230,
                          rootx=1345, rooty=1235)
    root.update()
    assert box._win.geometry().split("+")[0] == f"{340 + 2 * MARGIN}x{230 + 2 * MARGIN}"


def test_only_the_line_and_the_handles_are_painted(box, root):
    box.show(_RECT)
    root.update()
    key = str(box._win.attributes("-transparentcolor"))
    assert box._canvas.cget("bg") == key
    painted = {box._canvas.itemcget(item, "fill") for item in box._canvas.find_all()}
    painted |= {box._canvas.itemcget(item, "outline") for item in box._canvas.find_all()}
    assert painted - {"", key} == {FG_UPDATE}   # 沒有暗色外圈、把手也沒有暗色邊


def test_show_while_open_moves_the_existing_box(box, root):
    box.show(_RECT)
    first = box._win
    box.show((10, 20, 300, 200))
    root.update()
    assert box._win is first and box.rect() == (10, 20, 300, 200)


def test_hide_is_idempotent(box, root):
    box.show(_RECT)
    box.hide()
    box.hide()
    assert not box.is_open


@pytest.mark.parametrize("point, expected", [
    ((MARGIN + 300, MARGIN + 200), "se"),               # 右下角把手正中
    ((MARGIN, MARGIN), "nw"),
    ((MARGIN + 300, MARGIN + 100), "e"),                # 右緣中點把手
    ((MARGIN + 150, MARGIN + 200), "s"),                # 下緣中點把手
    ((MARGIN + 60, MARGIN), "n"),                       # 上緣框線：只拉上緣
    ((MARGIN + 300 + 1, MARGIN + 60), "e"),             # 右緣框線外側那 1px
    ((MARGIN + 150, MARGIN + 100), ""),                 # 框內：透明，實際上收不到點擊
    ((MARGIN + 150 - HANDLE, MARGIN + 100), ""),
])
def test_hit_at_tells_corners_from_edges_from_the_inside(point, expected):
    assert hit_at(*point, 300, 200) == expected
