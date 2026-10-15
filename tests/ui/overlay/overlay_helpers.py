"""覆疊視窗測試共用的建立與擷取輔助函式。"""
from src.reader.chat_tabs import TabState
from src.ui.overlay.message_list import _row_bottom, should_stick_to_bottom
from src.ui.overlay.outlined_text import line_height, line_origin
from src.ui.overlay.selection import TEXT_ORIGIN, Selection
from src.ui.overlay.window import (
    OverlayWindow,
)


def _filled_overlay(root, width=739, height=350, count=40,
                    max_messages=200, fade_seconds=0):
    """裝滿到需要捲動、且視圖停在最底的 overlay，供自動跟隨的測試當起點。"""
    ov = OverlayWindow(root, x=100, y=100, width=width, height=height,
                       max_messages=max_messages, fade_seconds=fade_seconds)
    root.update()
    for i in range(count):
        ov.add_message(f"message number {i}: a chat line long enough that a narrower "
                       f"window forces it onto a second line",
                       f"第 {i} 則譯文，內容夠長，窄視窗下一定會換行成兩行以上，"
                       f"這樣才測得到重新排版導致的高度變化", msg_id=i)
    root.update()
    assert ov._list._canvas.yview()[1] == 1.0, "前置條件：視圖應停在最底"
    return ov


def _at_bottom(ov) -> bool:
    return should_stick_to_bottom(ov._list._canvas.yview()[1])


def _line_box(line):
    """某行在螢幕上的（左緣 x, 頂緣 y, 行高）。"""
    return line.canvas.winfo_rootx(), Selection._screen_top(line), line_height(line)


def _select_whole_message(ov, index=0):
    """把第 index 則訊息整則選起來，回傳它的兩行。"""
    # update()（非 update_idletasks）：命中判定要量訊息畫布的實際高度，理由同
    # test_press_on_a_message_starts_a_selection
    ov._win.update()
    first, second = ov._list._messages[index].lines
    x, top, height = _line_box(first)
    ov._selection.begin(x + TEXT_ORIGIN, top + height // 2)
    x, top, height = _line_box(second)
    ov._selection.extend(x + 1000, top + height // 2)
    return first, second


class _Press:
    """假的滑鼠事件（只用到螢幕座標）。"""

    def __init__(self, x_root, y_root):
        self.x_root = x_root
        self.y_root = y_root
        self.widget = None


def _bar_state(current):
    return TabState(current, ((0, "主要的"), (1, "队伍"), (2, "房间(1)")))


def _configure_width(ov, width):
    """送一個指定寬度的假 <Configure> 給顯示中的畫布。"""
    class FakeEvent:
        pass

    FakeEvent.width = width
    ov._list._on_canvas_configure(FakeEvent())


def _long_overlay(root, n, fade_seconds=0):
    """n 則長度不一、會換行的中英文訊息（第 i 則的時刻為 i），視圖停在最底。"""
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, max_messages=n,
                       fade_seconds=fade_seconds)
    for i in range(n):
        ov.add_message(f"message {i}: " + "wizard chat line " * (1 + i % 6),
                       f"第 {i} 則譯文" + "會換行的長譯文內容" * (1 + i % 4), now=float(i), msg_id=i)
    ov._win.update()
    return ov


def _covered(ov) -> bool:
    """視口範圍內沒有未畫的區域。"""
    ml, view = ov._list, ov._list._views[ov._list.shown_key]
    top, bottom = ml._canvas.canvasy(0), ml._canvas.canvasy(ml._canvas.winfo_height())
    first, last = view.records[view.lo], view.records[view.hi - 1]
    return ((view.lo == 0 or line_origin(first.lines[0])[1] <= top)
            and (view.hi == len(view.records) or _row_bottom(last) >= bottom))


def _view(ov):
    """顯示中的分頁。"""
    return ov._list._views[ov._list.shown_key]
