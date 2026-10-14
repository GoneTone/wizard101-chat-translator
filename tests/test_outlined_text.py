import tkinter as tk

import pytest

from src.ui.fonts import ui_font
from src.ui.outlined_text import TEXT_ORIGIN, draw_outlined, line_height, line_origin
from src.ui.palette import OUTLINE

LONG = "Hello wizard friends 這是一段中英混排的訊息內容用來測試換行"


@pytest.fixture
def canvas(root):
    win = tk.Toplevel(root)
    c = tk.Canvas(win, width=300, height=300, highlightthickness=0)
    c.pack()
    yield c
    win.destroy()


def test_draw_outlined_matches_the_old_per_line_canvas(canvas):
    font = ui_font(11)
    reference = canvas.create_text(0, 0, font=font)
    line = draw_outlined(canvas, 0, 0, "hello", fill="#f2f2f7", font=font, wrap=250, tag="txt")
    items = canvas.find_withtag("txt")
    assert len(items) == 9
    assert reference not in items
    expected = [(1, 1), (1, 2), (1, 3), (2, 1), (2, 3), (3, 1), (3, 2), (3, 3)]
    for item, pos in zip(items[:8], expected, strict=True):
        assert canvas.coords(item) == list(pos)
        assert canvas.itemcget(item, "fill") == OUTLINE
    assert items[8] == line.fg
    assert canvas.coords(line.fg) == [2, 2]
    assert canvas.itemcget(line.fg, "fill") == "#f2f2f7"
    assert canvas.find_withtag("fg") == (line.fg,)
    for item in items:
        assert canvas.itemcget(item, "anchor") == "nw"
        assert canvas.itemcget(item, "width") == "250"
        assert canvas.itemcget(item, "text") == "hello"
        assert canvas.itemcget(item, "font") == canvas.itemcget(reference, "font")


def test_line_height_matches_the_old_canvas_height(canvas):
    font = ui_font(11)
    reference = tk.Canvas(canvas.master)
    for dx, dy in ((-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)):
        reference.create_text(TEXT_ORIGIN + dx, TEXT_ORIGIN + dy, text=LONG, font=font,
                              anchor="nw", width=120, tags="txt")
    reference.create_text(TEXT_ORIGIN, TEXT_ORIGIN, text=LONG, font=font, anchor="nw",
                          width=120, tags="txt")
    expected = reference.bbox("txt")[3] + 2
    line = draw_outlined(canvas, 0, 0, LONG, fill="#fff", font=font, wrap=120, tag="a")
    assert line_height(line) == expected
    assert line_height(draw_outlined(canvas, 0, 90, LONG, fill="#fff", font=font,
                                     wrap=120, tag="b")) == expected


def test_draw_outlined_at_an_offset_moves_every_item(canvas):
    line = draw_outlined(canvas, 0, 50, "hi", fill="#fff", font=ui_font(9), wrap=100, tag="a")
    assert line_origin(line) == (0, 50)
    assert canvas.bbox(line.tag)[1] >= 50
