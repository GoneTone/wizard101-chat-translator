import tkinter as tk

import pytest

from src.ui.fonts import ui_font
from src.ui.overlay import _outlined_line
from src.ui.selection import (
    TEXT_ORIGIN,
    Caret,
    caret_at,
    highlight_rects,
    line_font,
    line_text,
    selected_text,
    visual_lines,
)

WRAPPED = "Hello wizard friends 這是一段中英混排的訊息內容用來測試換行"


@pytest.fixture
def line(root):
    """一行描邊文字，寬度窄到一定會換行（建法與 overlay 正式路徑相同）。"""
    frame = tk.Frame(root)
    frame.pack()
    canvas = _outlined_line(frame, WRAPPED, "#f2f2f7", ui_font(11), 120)
    canvas.pack()
    root.update_idletasks()
    yield canvas
    frame.destroy()


def test_line_text_reads_the_foreground_item(line):
    assert line_text(line) == WRAPPED


def test_visual_lines_cover_the_whole_text(line):
    lines = visual_lines(line, line_font(line))
    assert len(lines) > 1, "測試字串應該要換行，否則這個測試沒有守到東西"
    assert lines[0][0] == 0
    assert lines[-1][1] == len(WRAPPED)
    for (_, end, _), (start, _, _) in zip(lines, lines[1:], strict=False):
        assert end == start


def test_visual_line_starts_agree_with_tk(line):
    # 行首索引是向 Tk 問出來的，這裡再問一次確認我們挑的取樣點落在正確的視覺行上
    font = line_font(line)
    spacing = font.metrics("linespace")
    for start, _, top in visual_lines(line, font):
        assert caret_at(line, TEXT_ORIGIN, top + spacing // 2) == start


def test_highlight_x_agrees_with_tk_hit_test(line):
    # 哨兵測試：反白的 x 是 font.measure 算的，Tk 的換行排版必須認同同一個位置。
    # 這條轉紅就代表 font.measure 與 Tk 的排版脫鉤了，反白會整片偏掉。
    font = line_font(line)
    spacing = font.metrics("linespace")
    first_start, first_end, top = visual_lines(line, font)[0]
    middle = (first_start + first_end) // 2
    (x0, y0, _, _) = highlight_rects(line, font, middle, first_end)[0]
    assert y0 == top
    assert caret_at(line, x0, top + spacing // 2) == middle


def test_highlight_spans_every_visual_line(line):
    font = line_font(line)
    rects = highlight_rects(line, font, 0, len(WRAPPED))
    assert len(rects) == len(visual_lines(line, font))


def test_empty_selection_has_no_rects(line):
    assert highlight_rects(line, line_font(line), 5, 5) == []


def test_selected_text_within_one_line():
    assert selected_text(["abcdef", "xyz"], Caret(0, 1), Caret(0, 4)) == "bcd"


def test_selected_text_across_both_lines():
    assert selected_text(["abcdef", "xyz"], Caret(0, 4), Caret(1, 2)) == "ef\nxy"


def test_selected_text_of_an_empty_span():
    assert selected_text(["abcdef", "xyz"], Caret(0, 2), Caret(0, 2)) == ""
