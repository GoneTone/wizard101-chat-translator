"""疊加視窗訊息的框選：把 Tk 的排版問出來，換算成反白矩形與選取字串。

一則訊息由兩個描邊文字 canvas 組成（原文行、譯文行，見 `overlay._outlined_line`）。
Tk 的 canvas 原生選取全 app 只有一份、跨不了這兩個 canvas，所以反白自繪；但換行
規則不自行實作——逐行問 Tk「這個座標是第幾個字元」即可還原排版。
"""
import tkinter.font as tkfont
from typing import NamedTuple

from src.log import log
from src.ui.palette import SELECT_BG

TEXT_ORIGIN = 2   # 描邊文字 item 的繪製原點（見 overlay._outlined_line 的 create_text(2, 2, ...)）


class Caret(NamedTuple):
    """選取端點。line＝該則訊息內第幾個 canvas（0＝原文行，1＝譯文行）；
    index＝該行文字內的字元索引。NamedTuple 的比較順序恰好就是閱讀順序。"""
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


def selected_text(texts: list[str], start: Caret, end: Caret) -> str:
    """選取範圍內的文字；跨行時以換行相接。純字串運算，不碰 Tk。"""
    if start >= end:
        return ""
    if start.line == end.line:
        return texts[start.line][start.index:end.index]
    parts = [texts[start.line][start.index:]]
    parts.extend(texts[line] for line in range(start.line + 1, end.line))
    parts.append(texts[end.line][:end.index])
    return "\n".join(parts)


class Selection:
    """疊加視窗訊息的選取狀態：命中、夾取、反白繪製與取字。

    只認螢幕座標——事件從文字 canvas（墨跡像素）或 backdrop（其餘區域因透明色鍵
    而穿透過去）進來都一樣，呼叫端負責換算。選取不跨訊息：起手決定作用中的那一則，
    拖出範圍就夾到該則的頭或尾。"""

    def __init__(self):
        self._rows: dict = {}          # 訊息列 → 該列的行 canvas（插入順序＝顯示順序）
        self._fonts: dict = {}         # 行 canvas → Font；拖曳時每個事件都要量測，不能重建
        self._active = None            # 作用中的那一則（訊息列）
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
        """訊息列即將被銷毀：解除登記，選取落在該列時一併清掉——否則選取會指向
        已銷毀的 widget，之後任何一次重畫都會炸。"""
        canvases = self._rows.pop(row, None)
        if canvases is None:
            return
        for canvas in canvases:
            self._fonts.pop(canvas, None)
        if self._active is row:
            self._active = self._anchor = self._focus = None
            self._dragging = False
            log("[ui] selection cleared (row destroyed)")

    def holds(self, row) -> bool:
        """目前的選取是否落在這一列。"""
        return self._active is row

    def span(self) -> tuple[Caret, Caret] | None:
        """正規化後的 `(start, end)`；沒有選取或選取為空則 None。"""
        if self._anchor is None or self._focus is None or self._anchor == self._focus:
            return None
        return min(self._anchor, self._focus), max(self._anchor, self._focus)

    def text(self) -> str:
        """目前選取到的文字（跨行以換行相接）。"""
        span = self.span()
        if span is None:
            return ""
        return selected_text([line_text(c) for c in self._rows[self._active]], *span)

    def begin(self, x_root: int, y_root: int) -> bool:
        """框選起手；回傳是否命中某一則訊息。"""
        hit = self._hit(x_root, y_root)
        self.clear("new press")
        if hit is None:
            return False
        self._active, caret = hit
        self._anchor = self._focus = caret
        self._dragging = True
        log(f"[ui] selection begin line={caret.line} index={caret.index}")
        return True

    def extend(self, x_root: int, y_root: int) -> None:
        """拖曳中：更新 focus（夾在起手那一則內）並重畫反白。"""
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
        log(f"[ui] selection end start={start.line}:{start.index} "
            f"end={end.line}:{end.index} chars={len(self.text())}")

    def clear(self, reason: str) -> None:
        """清除選取與反白。`reason` 只進 log，方便事後定位「選取莫名消失」。"""
        if self._active is None:
            return
        self._erase()
        self._active = self._anchor = self._focus = None
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

    def _caret_in(self, canvas, line: int, x_root: int, y_root: int) -> Caret:
        return Caret(line, caret_at(canvas, x_root - canvas.winfo_rootx(),
                                    y_root - canvas.winfo_rooty()))

    def _hit(self, x_root: int, y_root: int):
        """螢幕座標落在哪一則的哪個 caret；都沒命中回 None。"""
        for row, canvases in self._rows.items():
            for line, canvas in enumerate(canvases):
                if canvas.winfo_rooty() <= y_root <= canvas.winfo_rooty() + canvas.winfo_height() - 1:
                    return row, self._caret_in(canvas, line, x_root, y_root)
        return None

    def _clamped(self, x_root: int, y_root: int) -> Caret:
        """拖曳點換算成作用中那一則的 caret。垂直方向決定落在哪一行、超出則夾到
        該則的頭或尾；水平方向交給 Tk 自己夾（見 `caret_at`）。"""
        canvases = self._rows[self._active]
        if y_root < canvases[0].winfo_rooty():
            return Caret(0, 0)
        for line, canvas in enumerate(canvases):
            if y_root <= canvas.winfo_rooty() + canvas.winfo_height() - 1:
                return self._caret_in(canvas, line, x_root, y_root)
        return Caret(len(canvases) - 1, len(line_text(canvases[-1])))

    def _erase(self) -> None:
        for canvas in self._rows.get(self._active, ()):
            canvas.delete("sel")

    def _draw(self) -> None:
        self._erase()
        span = self.span()
        if span is None:
            return
        start, end = span
        for line, canvas in enumerate(self._rows[self._active]):
            if line < start.line or line > end.line:
                continue
            lo = start.index if line == start.line else 0
            hi = end.index if line == end.line else len(line_text(canvas))
            rects = highlight_rects(canvas, self._font(canvas), lo, hi)
            if rects:
                for x0, y0, x1, y1 in rects:
                    canvas.create_rectangle(x0, y0, x1, y1, fill=SELECT_BG,
                                            outline="", tags="sel")
                canvas.tag_lower("sel", "txt")
