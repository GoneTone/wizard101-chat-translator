"""疊加視窗訊息的框選：把 Tk 的排版問出來，換算成反白矩形與選取字串。

一則訊息由兩個描邊文字 canvas 組成（原文行、譯文行，見 `overlay._outlined_line`）。
Tk 的 canvas 原生選取全 app 只有一份、跨不了這兩個 canvas，所以反白自繪；但換行
規則不自行實作 —— 逐行問 Tk「這個座標是第幾個字元」即可還原排版。
"""
import tkinter.font as tkfont
from typing import NamedTuple

from src.log import log
from src.ui.geometry import point_in_rect
from src.ui.palette import SELECT_BG

TEXT_ORIGIN = 2   # 描邊文字 item 的繪製原點（見 overlay._outlined_line 的 create_text(2, 2, ...)）


class Caret(NamedTuple):
    """選取端點。message＝第幾則訊息；line＝該則內第幾個 canvas（0＝原文行，
    1＝譯文行）；index＝該行文字內的字元索引。欄位順序即閱讀順序，NamedTuple
    的字典序比較因此可以直接拿來排先後。"""
    message: int
    line: int
    index: int


def line_text(canvas) -> str:
    """該行目前顯示的文字。本色 item（"fg" tag）是唯一真實來源，不在別處另記一份。"""
    return canvas.itemcget("fg", "text")


def line_font(canvas) -> tkfont.Font:
    """該行的字型，包成可量測的 Font 物件。"""
    return tkfont.Font(canvas, font=canvas.itemcget("fg", "font"))


def caret_at(canvas, x: int, y: int) -> int:
    """canvas 本地座標對應的字元索引。座標超出文字範圍時 Tk 自己會夾到頭／尾。"""
    return canvas.index("fg", f"@{x},{y}")


def visual_lines(canvas, font) -> list[tuple[int, int, int]]:
    """換行後每個視覺行的 `(起始索引, 結束索引, y_top)`。

    bbox 只取本色 item：描邊副本往外多 1px，用 `bbox("all")` 會把視覺行數多算出來。"""
    box = canvas.bbox("fg")
    text = line_text(canvas)
    if not box or not text:
        return []
    spacing = font.metrics("linespace")
    count = max(1, round((box[3] - box[1]) / spacing))
    starts = [caret_at(canvas, TEXT_ORIGIN, TEXT_ORIGIN + n * spacing + spacing // 2)
              for n in range(count)]
    return [(start, starts[n + 1] if n + 1 < count else len(text),
             TEXT_ORIGIN + n * spacing)
            for n, start in enumerate(starts)]


def highlight_rects(canvas, font, start: int, end: int) -> list[tuple[int, int, int, int]]:
    """`[start, end)` 在該行要反白的矩形（canvas 本地座標，每個視覺行一個）。"""
    if start >= end:
        return []
    text = line_text(canvas)
    spacing = font.metrics("linespace")
    rects = []
    for first, last, top in visual_lines(canvas, font):
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

    只認螢幕座標 —— 事件從文字 canvas（墨跡像素）或 backdrop（其餘區域因透明色鍵
    而穿透過去）進來都一樣，呼叫端負責換算。選取可跨訊息；拖出全部訊息的上下界
    就夾到最前／最後。

    caret 以「第幾則」定位而非物件參照，所以列被移除時序號要跟著維護（見 `forget`）。
    """

    def __init__(self):
        self._rows: dict = {}          # 訊息列 → 該列的行 canvas（插入順序＝顯示順序）
        self._fonts: dict = {}         # 行 canvas → Font；拖曳時每個事件都要量測，不能重建
        self._drawn: list = []         # 目前畫著反白的 canvas；擦除只掃這些，不掃全部訊息
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

    def register(self, row, canvases) -> None:
        """登記一則訊息的行 canvas（由上而下）。"""
        self._rows[row] = list(canvases)

    def forget(self, row) -> None:
        """訊息列即將被銷毀：解除登記。

        列落在選取範圍內就一併清掉選取 —— 留著的話 caret 會指向已銷毀的 widget。
        列在選取**之前**則把兩個 caret 的序號各往前挪一格：訊息是以序號定位的，
        而 `prune` 每輪都可能砍掉最舊的一則，不挪的話選取會整段錯位。"""
        canvases = self._rows.get(row)
        if canvases is None:
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
        for canvas in canvases:
            self._fonts.pop(canvas, None)
            if canvas in self._drawn:
                self._drawn.remove(canvas)
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
        messages = [[line_text(c) for c in canvases] for canvases in self._rows.values()]
        return selected_text(messages, *span)

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
    def _font(self, canvas):
        if canvas not in self._fonts:
            self._fonts[canvas] = line_font(canvas)
        return self._fonts[canvas]

    def _caret_in(self, canvas, message: int, line: int,
                  x_root: int, y_root: int) -> Caret:
        return Caret(message, line, caret_at(canvas, x_root - canvas.winfo_rootx(),
                                             y_root - canvas.winfo_rooty()))

    def _hit(self, x_root: int, y_root: int) -> Caret | None:
        """螢幕座標落在哪個 caret；都沒命中回 None。"""
        for message, canvases in enumerate(self._rows.values()):
            for line, canvas in enumerate(canvases):
                if point_in_rect(x_root, y_root,
                                 canvas.winfo_rootx(), canvas.winfo_rooty(),
                                 canvas.winfo_width(), canvas.winfo_height()):
                    return self._caret_in(canvas, message, line, x_root, y_root)
        return None

    def _clamped(self, x_root: int, y_root: int) -> Caret:
        """拖曳點換算成 caret。垂直方向決定落在哪一則的哪一行，超出全部訊息的
        上下界就夾到最前／最後；水平方向交給 Tk 自己夾（見 `caret_at`）。"""
        rows = list(self._rows.values())
        if y_root < rows[0][0].winfo_rooty():
            return Caret(0, 0, 0)
        for message, canvases in enumerate(rows):
            for line, canvas in enumerate(canvases):
                if y_root <= canvas.winfo_rooty() + canvas.winfo_height() - 1:
                    return self._caret_in(canvas, message, line, x_root, y_root)
        return Caret(len(rows) - 1, len(rows[-1]) - 1, len(line_text(rows[-1][-1])))

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
        start, end = span
        rows = list(self._rows.values())
        for message in range(start.message, end.message + 1):
            for line, canvas in enumerate(rows[message]):
                if not ((start.message, start.line) <= (message, line)
                        <= (end.message, end.line)):
                    continue
                lo = start.index if (message, line) == (start.message, start.line) else 0
                hi = (end.index if (message, line) == (end.message, end.line)
                      else len(line_text(canvas)))
                rects = highlight_rects(canvas, self._font(canvas), lo, hi)
                if rects:
                    for x0, y0, x1, y1 in rects:
                        canvas.create_rectangle(x0, y0, x1, y1, fill=SELECT_BG,
                                                outline="", tags="sel")
                    canvas.tag_lower("sel", "txt")
                    self._drawn.append(canvas)
