"""疊加視窗訊息的框選：把 Tk 的排版問出來，換算成反白矩形與選取字串。

一則訊息由兩行描邊文字組成（原文行、譯文行，見 `outlined_text.Line`）。Tk 的 canvas
原生選取全 app 只有一份、跨不了多個文字 item，所以反白自繪；但換行規則不自行實作 ——
逐行問 Tk「這個座標是第幾個字元」即可還原排版。行內座標一律以行原點為 (0, 0)。
"""
import tkinter.font as tkfont
from typing import NamedTuple

from src.logs import log
from src.ui.geometry import point_in_rect
from src.ui.overlay.outlined_text import TEXT_ORIGIN, Line, line_height, line_origin
from src.ui.palette import SELECT_BG


class Caret(NamedTuple):
    """選取端點。message＝第幾則訊息；line＝該則內第幾行（0＝原文行，
    1＝譯文行）；index＝該行文字內的字元索引。欄位順序即閱讀順序，NamedTuple
    的字典序比較因此可以直接拿來排先後。"""
    message: int
    line: int
    index: int


class Row(NamedTuple):
    """一則訊息的登記內容。texts＝原文、譯文；lines＝畫布上的行，空代表尚未畫出。"""
    texts: tuple[str, ...]
    lines: tuple[Line, ...]


def line_text(line: Line) -> str:
    """該行目前顯示的文字。本色 item 是唯一真實來源，不在別處另記一份。"""
    return line.canvas.itemcget(line.fg, "text")


def line_font(line: Line) -> tkfont.Font:
    """該行的字型，包成可量測的 Font 物件。"""
    return tkfont.Font(line.canvas, font=line.canvas.itemcget(line.fg, "font"))


def caret_at(line: Line, x: int, y: int) -> int:
    """行內座標對應的字元索引。座標超出文字範圍時 Tk 自己會夾到頭／尾。"""
    ox, oy = line_origin(line)
    return line.canvas.index(line.fg, f"@{x + ox},{y + oy}")


def visual_lines(line: Line, font) -> list[tuple[int, int, int]]:
    """換行後每個視覺行的 `(起始索引, 結束索引, y_top)`（行內座標）。

    bbox 只取本色 item：描邊副本往外多 1px，用整行的 bbox 會把視覺行數多算出來。"""
    box = line.canvas.bbox(line.fg)
    text = line_text(line)
    if not box or not text:
        return []
    spacing = font.metrics("linespace")
    count = max(1, round((box[3] - box[1]) / spacing))
    starts = [caret_at(line, TEXT_ORIGIN, TEXT_ORIGIN + n * spacing + spacing // 2)
              for n in range(count)]
    return [(start, starts[n + 1] if n + 1 < count else len(text),
             TEXT_ORIGIN + n * spacing)
            for n, start in enumerate(starts)]


def highlight_rects(line: Line, font, start: int, end: int) -> list[tuple[int, int, int, int]]:
    """`[start, end)` 在該行要反白的矩形（行內座標，每個視覺行一個）。"""
    if start >= end:
        return []
    text = line_text(line)
    spacing = font.metrics("linespace")
    rects = []
    for first, last, top in visual_lines(line, font):
        lo, hi = max(start, first), min(end, last)
        if lo >= hi:
            continue
        rects.append((TEXT_ORIGIN + font.measure(text[first:lo]), top,
                      TEXT_ORIGIN + font.measure(text[first:hi]), top + spacing))
    return rects


def selected_text(messages: list[list[str]], start: Caret, end: Caret) -> str:
    """選取範圍內的文字。同一則訊息的行以換行相接，訊息與訊息之間空一行 ——
    貼出去才看得出哪幾句原本是同一則。純字串運算，不碰 Tk。"""
    if start >= end:
        return ""
    chunks = []
    for message in range(start.message, end.message + 1):
        lines = messages[message]
        first = start.line if message == start.message else 0
        last = end.line if message == end.message else len(lines) - 1
        picked = []
        for line in range(first, last + 1):
            text = lines[line]
            lo = start.index if (message, line) == (start.message, start.line) else 0
            hi = end.index if (message, line) == (end.message, end.line) else len(text)
            picked.append(text[lo:hi])
        chunks.append("\n".join(picked))
    return "\n\n".join(chunks)


class Selection:
    """疊加視窗訊息的選取狀態：命中、夾取、反白繪製與取字。

    只認螢幕座標 —— 事件從訊息畫布（文字墨跡像素）或 backdrop（其餘區域因透明色鍵
    而穿透過去）進來都一樣，呼叫端負責換算。選取可跨訊息；拖出全部訊息的上下界
    就夾到最前／最後。

    caret 以「第幾則」定位而非物件參照，所以列被移除時序號要跟著維護（見 `forget`）。
    """

    def __init__(self):
        self._rows: dict[object, Row] = {}   # 訊息列 → 該列的 Row（插入順序＝顯示順序）
        self._fonts: dict = {}         # 字型描述 → Font；拖曳時每個事件都要量測，不能重建
        self._drawn: list = []         # 畫著反白的畫布；換分頁後舊畫布已不在登記表，擦除要靠這份記錄
        self._anchor: Caret | None = None
        self._focus: Caret | None = None
        self._dragging = False

    @property
    def active(self) -> bool:
        """目前是否有非空的選取。"""
        return self.span() is not None

    @property
    def dragging(self) -> bool:
        """目前是否正在拖曳選取（拖曳期間畫面不該自動貼底）。"""
        return self._dragging

    def use_rows(self, rows: dict) -> None:
        """清除選取，改用呼叫端持有的 dict 當登記表（換顯示中的分頁時用）。"""
        self.clear("rows replaced")
        self._rows = rows

    def forget(self, row) -> None:
        """訊息即將被刪除：解除登記。

        列落在選取範圍內就一併清掉選取 —— 留著的話 caret 會指向已刪除的文字 item。
        列在選取**之前**則把兩個 caret 的序號各往前挪一格：訊息是以序號定位的，
        而 `prune` 每輪都可能砍掉最舊的一則，不挪的話選取會整段錯位。"""
        if row not in self._rows:
            return
        message = list(self._rows).index(row)
        low = high = None
        if self._anchor is not None and self._focus is not None:
            low = min(self._anchor, self._focus).message
            high = max(self._anchor, self._focus).message
        touched = low is not None and low <= message <= high
        if touched:
            self._erase()   # 趁該列還在時先擦掉反白，孤兒高亮不會留下來
        self._rows.pop(row, None)
        if low is None:
            return
        if touched:
            self._anchor = self._focus = None
            self._dragging = False
            log("[ui] selection cleared (row destroyed)")
        elif message < low:
            self._anchor = self._anchor._replace(message=self._anchor.message - 1)
            self._focus = self._focus._replace(message=self._focus.message - 1)

    def holds(self, row) -> bool:
        """這一列是否落在目前的選取範圍內。"""
        span = self.span()
        if span is None or row not in self._rows:
            return False
        return span[0].message <= list(self._rows).index(row) <= span[1].message

    def span(self) -> tuple[Caret, Caret] | None:
        """正規化後的 `(start, end)`；沒有選取或選取為空則 None。"""
        if self._anchor is None or self._focus is None or self._anchor == self._focus:
            return None
        return min(self._anchor, self._focus), max(self._anchor, self._focus)

    def text(self) -> str:
        """目前選取到的文字（同一則內換行相接，訊息之間空一行）。"""
        span = self.span()
        if span is None:
            return ""
        return selected_text([row.texts for row in self._rows.values()], *span)

    def begin(self, x_root: int, y_root: int) -> bool:
        """框選起手；回傳是否命中某一則訊息。"""
        caret = self._hit(x_root, y_root)
        self.clear("new press")
        if caret is None:
            return False
        self._anchor = self._focus = caret
        self._dragging = True
        log(f"[ui] selection begin message={caret.message} line={caret.line} "
            f"index={caret.index}")
        return True

    def extend(self, x_root: int, y_root: int) -> None:
        """拖曳中：更新 focus 並重畫反白。"""
        if not self._dragging:
            return
        self._focus = self._clamped(x_root, y_root)
        self._draw()

    def finish(self) -> None:
        """放開滑鼠。"""
        if not self._dragging:
            return
        self._dragging = False
        span = self.span()
        if span is None:
            log("[ui] selection end empty")
            return
        start, end = span
        log(f"[ui] selection end start={start.message}:{start.line}:{start.index} "
            f"end={end.message}:{end.line}:{end.index} chars={len(self.text())}")

    def clear(self, reason: str) -> None:
        """清除選取與反白。`reason` 只進 log，方便事後定位「選取莫名消失」。"""
        if self._anchor is None:
            return
        self._erase()
        self._anchor = self._focus = None
        self._dragging = False
        log(f"[ui] selection cleared ({reason})")

    def redraw(self) -> None:
        """換行寬度改變後依現有的 anchor／focus 重畫（字元索引不受換行影響）。"""
        self._draw()

    # --- 內部 ---
    def _font(self, line: Line):
        # 以字型描述為鍵：實際只有原文／譯文兩種，行被刪除或重畫也不會留下孤兒快取
        desc = line.canvas.itemcget(line.fg, "font")
        if desc not in self._fonts:
            self._fonts[desc] = line_font(line)
        return self._fonts[desc]

    @staticmethod
    def _screen_top(line: Line) -> int:
        """該行頂緣的螢幕 y（已扣掉 canvas 的捲動偏移）。"""
        canvas = line.canvas
        return canvas.winfo_rooty() + round(line_origin(line)[1] - canvas.canvasy(0))

    def _caret_in(self, line: Line, message: int, n: int,
                  x_root: int, y_root: int) -> Caret:
        canvas = line.canvas
        ox, _ = line_origin(line)
        x = x_root - canvas.winfo_rootx() + round(canvas.canvasx(0) - ox)
        return Caret(message, n, caret_at(line, x, y_root - self._screen_top(line)))

    def _hit(self, x_root: int, y_root: int) -> Caret | None:
        """螢幕座標落在哪個 caret；都沒命中回 None。"""
        for message, row in enumerate(self._rows.values()):
            for n, line in enumerate(row.lines):
                canvas = line.canvas
                visible = (canvas.winfo_rooty() <= y_root
                           < canvas.winfo_rooty() + canvas.winfo_height())
                if visible and point_in_rect(x_root, y_root,
                                             canvas.winfo_rootx(), self._screen_top(line),
                                             canvas.winfo_width(), line_height(line)):
                    return self._caret_in(line, message, n, x_root, y_root)
        return None

    def _clamped(self, x_root: int, y_root: int) -> Caret:
        """拖曳點換算成 caret。垂直方向決定落在哪一則的哪一行，超出全部訊息的
        上下界就夾到最前／最後；水平方向交給 Tk 自己夾（見 `caret_at`）。"""
        drawn = [(message, row) for message, row in enumerate(self._rows.values()) if row.lines]
        first, last = drawn[0], drawn[-1]
        if y_root < self._screen_top(first[1].lines[0]):
            return Caret(first[0], 0, 0)
        for message, row in drawn:
            for n, line in enumerate(row.lines):
                if y_root <= self._screen_top(line) + line_height(line) - 1:
                    return self._caret_in(line, message, n, x_root, y_root)
        return Caret(last[0], len(last[1].texts) - 1, len(last[1].texts[-1]))

    def _erase(self) -> None:
        for canvas in self._drawn:
            if canvas.winfo_exists():
                canvas.delete("sel")
        self._drawn.clear()

    def _draw(self) -> None:
        self._erase()
        span = self.span()
        if span is None:
            return
        rows = list(self._rows.values())
        for message in range(span[0].message, span[1].message + 1):
            self._draw_row(message, rows[message], *span)

    def paint(self, row) -> None:
        """補畫某一列的反白（該列在選取範圍內且已畫時），不動其他列。"""
        span = self.span()
        if span is not None and self.holds(row):
            message = list(self._rows).index(row)
            self._draw_row(message, self._rows[row], *span)

    def _draw_row(self, message: int, row: Row, start: Caret, end: Caret) -> None:
        for n, line in enumerate(row.lines):
            if not (start.message, start.line) <= (message, n) <= (end.message, end.line):
                continue
            lo = start.index if (message, n) == (start.message, start.line) else 0
            hi = (end.index if (message, n) == (end.message, end.line)
                  else len(row.texts[n]))
            rects = highlight_rects(line, self._font(line), lo, hi)
            if not rects:
                continue
            ox, oy = line_origin(line)
            for x0, y0, x1, y1 in rects:
                rect = line.canvas.create_rectangle(
                    x0 + ox, y0 + oy, x1 + ox, y1 + oy,
                    fill=SELECT_BG, outline="", tags="sel")
                # 壓到整張畫布最底：效果同壓在該行之下，但以 tag 為基準每次都要掃整張畫布
                line.canvas.tag_lower(rect)
            if line.canvas not in self._drawn:
                self._drawn.append(line.canvas)
