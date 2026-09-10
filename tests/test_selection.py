import tkinter as tk

import pytest

from src.ui.fonts import ui_font
from src.ui.message_list import _outlined_line
from src.ui.selection import (
    TEXT_ORIGIN,
    Caret,
    Selection,
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
    assert selected_text([["abcdef", "xyz"]], Caret(0, 0, 1), Caret(0, 0, 4)) == "bcd"


def test_selected_text_across_both_lines():
    assert selected_text([["abcdef", "xyz"]], Caret(0, 0, 4), Caret(0, 1, 2)) == "ef\nxy"


def test_selected_text_of_an_empty_span():
    assert selected_text([["abcdef", "xyz"]], Caret(0, 0, 2), Caret(0, 0, 2)) == ""


def test_selected_text_includes_whole_lines_in_between():
    assert selected_text([["abc", "mid", "xyz"]],
                         Caret(0, 0, 1), Caret(0, 2, 2)) == "bc\nmid\nxy"


def test_selected_text_separates_messages_with_a_blank_line():
    messages = [["a1", "a2"], ["b1", "b2"], ["c1", "c2"]]
    assert selected_text(messages, Caret(0, 1, 1), Caret(2, 0, 1)) == "2\n\nb1\nb2\n\nc"


def test_selected_text_of_two_whole_messages():
    messages = [["原文一", "譯文一"], ["原文二", "譯文二"]]
    assert selected_text(messages, Caret(0, 0, 0), Caret(1, 1, 3)) == (
        "原文一\n譯文一\n\n原文二\n譯文二")


@pytest.fixture
def message(root):
    """兩行一組的訊息列（原文行 ＋ 譯文行），已註冊進一個 Selection。"""
    win = tk.Toplevel(root)
    frame = tk.Frame(win)
    frame.pack()
    first = _outlined_line(frame, "original text", "#c0c0cd", ui_font(9), 400)
    first.pack(fill="x")
    second = _outlined_line(frame, "translated text", "#f2f2f7", ui_font(11), 400)
    second.pack(fill="x")
    win.update()
    sel = Selection()
    sel.register(frame, (first, second))
    yield sel, frame, first, second
    win.destroy()


def _at(canvas, dx=TEXT_ORIGIN, dy=None):
    """canvas 內某點的螢幕座標（dy 預設取該 canvas 的垂直中線）。"""
    if dy is None:
        dy = canvas.winfo_height() // 2
    return canvas.winfo_rootx() + dx, canvas.winfo_rooty() + dy


def test_begin_on_a_line_reports_a_hit(message):
    sel, _, first, _ = message
    assert sel.begin(*_at(first)) is True
    assert sel.dragging is True


def test_begin_outside_every_message_reports_no_hit(message):
    sel, frame, first, _ = message
    x, y = _at(first)
    assert sel.begin(x, y - frame.winfo_height() - 200) is False


def test_begin_far_to_the_right_of_a_line_reports_no_hit(message):
    # 命中要兩軸都算：只看垂直的話，視窗外任何同高度的點都會誤判成選到訊息
    sel, _, first, _ = message
    x, y = _at(first)
    assert sel.begin(x + first.winfo_width() + 200, y) is False


def test_a_bare_press_is_not_a_selection(message):
    sel, _, first, _ = message
    sel.begin(*_at(first))
    sel.finish()
    assert sel.active is False
    assert sel.text() == ""


def test_dragging_across_both_lines_selects_both(message):
    sel, _, first, second = message
    sel.begin(*_at(first))
    sel.extend(*_at(second, dx=1000))
    assert sel.text() == "original text\ntranslated text"


def test_reverse_drag_selects_the_same_text(message):
    sel, _, first, second = message
    sel.begin(*_at(second, dx=second.winfo_width() - 1))
    sel.extend(*_at(first))
    assert sel.text() == "original text\ntranslated text"


def test_dragging_below_the_message_clamps_to_its_end(message):
    sel, _, first, second = message
    sel.begin(*_at(first))
    x, y = _at(second)
    sel.extend(x, y + 500)
    assert sel.text() == "original text\ntranslated text"


def test_dragging_above_the_message_clamps_to_its_start(message):
    sel, _, first, second = message
    sel.begin(*_at(second, dx=second.winfo_width() - 1))
    x, y = _at(first)
    sel.extend(x, y - 500)
    assert sel.text() == "original text\ntranslated text"


@pytest.fixture
def three_messages(root):
    """三則訊息（每則原文行＋譯文行）疊在同一個視窗裡，已註冊進一個 Selection。"""
    win = tk.Toplevel(root)
    rows = []
    sel = Selection()
    for n in ("one", "two", "three"):
        frame = tk.Frame(win)
        frame.pack()
        top = _outlined_line(frame, f"orig {n}", "#c0c0cd", ui_font(9), 400)
        top.pack(fill="x")
        bottom = _outlined_line(frame, f"trans {n}", "#f2f2f7", ui_font(11), 400)
        bottom.pack(fill="x")
        sel.register(frame, (top, bottom))
        rows.append((frame, top, bottom))
    win.update()
    yield sel, rows
    win.destroy()


def test_selection_spans_messages_with_a_blank_line_between(three_messages):
    sel, rows = three_messages

    sel.begin(*_at(rows[0][1]))
    sel.extend(*_at(rows[1][2], dx=rows[1][2].winfo_width() - 1))

    assert sel.text() == "orig one\ntrans one\n\norig two\ntrans two"


def test_selection_spans_three_messages(three_messages):
    sel, rows = three_messages

    sel.begin(*_at(rows[0][1]))
    sel.extend(*_at(rows[2][2], dx=rows[2][2].winfo_width() - 1))

    assert sel.text().count("\n\n") == 2, "三則之間要有兩個空行"
    assert sel.text().startswith("orig one")
    assert sel.text().endswith("trans three")


def test_dragging_below_everything_clamps_to_the_last_message(three_messages):
    sel, rows = three_messages
    x, y = _at(rows[2][2])

    sel.begin(*_at(rows[0][1]))
    sel.extend(x, y + 500)

    assert sel.text().endswith("trans three")


def test_dragging_above_everything_clamps_to_the_first_message(three_messages):
    sel, rows = three_messages
    x, y = _at(rows[0][1])

    sel.begin(*_at(rows[2][2], dx=rows[2][2].winfo_width() - 1))
    sel.extend(x, y - 500)

    assert sel.text().startswith("orig one")


def test_highlight_is_drawn_on_every_spanned_message(three_messages):
    sel, rows = three_messages

    sel.begin(*_at(rows[0][1]))
    sel.extend(*_at(rows[2][2], dx=rows[2][2].winfo_width() - 1))

    for _, top, bottom in rows:
        assert top.find_withtag("sel"), "跨越的每一列都要有反白"
        assert bottom.find_withtag("sel")


def test_forgetting_a_row_before_the_selection_keeps_the_text(three_messages):
    # 訊息以序號定位，prune 砍掉最舊的一則時序號要整體前移，否則選取會錯位
    sel, rows = three_messages
    sel.begin(*_at(rows[1][1]))
    sel.extend(*_at(rows[2][2], dx=rows[2][2].winfo_width() - 1))
    before = sel.text()

    sel.forget(rows[0][0])

    assert sel.text() == before


def test_forgetting_a_row_inside_the_selection_clears_it(three_messages):
    sel, rows = three_messages
    sel.begin(*_at(rows[0][1]))
    sel.extend(*_at(rows[2][2], dx=rows[2][2].winfo_width() - 1))

    sel.forget(rows[1][0])

    assert sel.active is False


def test_highlight_is_drawn_below_the_text(message):
    sel, _, first, second = message
    sel.begin(*_at(first))
    sel.extend(*_at(second, dx=1000))
    order = first.find_all()
    highlight = first.find_withtag("sel")
    glyphs = first.find_withtag("txt")
    assert highlight, "原文行應該畫出反白矩形"
    # find_withtag 給的是 item ID（建立順序），要判斷誰壓在誰底下得看 find_all 的堆疊順序
    assert max(order.index(i) for i in highlight) < min(order.index(i) for i in glyphs), \
        "反白必須壓在文字之下，否則會蓋掉字"


def test_clear_removes_the_highlight_and_the_selection(message):
    sel, _, first, second = message
    sel.begin(*_at(first))
    sel.extend(*_at(second, dx=1000))
    sel.clear("test")
    assert sel.active is False
    assert sel.text() == ""
    assert first.find_withtag("sel") == ()


def test_forget_clears_a_selection_in_that_row(message):
    sel, frame, first, second = message
    sel.begin(*_at(first))
    sel.extend(*_at(second, dx=1000))
    sel.forget(frame)
    assert sel.active is False
    assert sel.holds(frame) is False


def test_redraw_keeps_the_selected_text(message):
    sel, _, first, second = message
    sel.begin(*_at(first))
    sel.extend(*_at(second, dx=1000))
    sel.redraw()
    assert sel.text() == "original text\ntranslated text"
    assert first.find_withtag("sel")
