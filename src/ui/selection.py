"""疊加視窗訊息的框選：把 Tk 的排版問出來，換算成反白矩形與選取字串。

一則訊息由兩個描邊文字 canvas 組成（原文行、譯文行，見 `overlay._outlined_line`）。
Tk 的 canvas 原生選取全 app 只有一份、跨不了這兩個 canvas，所以反白自繪；但換行
規則不自行實作——逐行問 Tk「這個座標是第幾個字元」即可還原排版。
"""
import tkinter.font as tkfont
from typing import NamedTuple

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
