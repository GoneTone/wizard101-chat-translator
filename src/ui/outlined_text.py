"""字幕式描邊文字行：在 canvas 上以文字 item 畫出一行，並提供行原點與行高的量法。"""
import tkinter as tk
from typing import NamedTuple

from src.ui.palette import OUTLINE

TEXT_ORIGIN = 2   # 文字 item 相對於行原點的偏移，留給描邊副本往左上多出的 1px
OUTLINE_OFFSETS = ((-1, -1), (-1, 0), (-1, 1), (0, -1),
                   (0, 1), (1, -1), (1, 0), (1, 1))


class Line(NamedTuple):
    """canvas 上的一行描邊文字：fg＝本色文字 item 的 id，tag＝該行 9 個文字 item 共用的 tag。"""
    canvas: tk.Canvas
    fg: int
    tag: str


def draw_outlined(canvas: tk.Canvas, x: int, y: int, text: str, *, fill: str,
                  font: tuple, wrap: int, tag: str) -> Line:
    """以 `(x, y)` 為行原點畫一行：先畫八方向 1px 偏移的描邊副本，再疊本色。

    Label 無法描邊，透明度調低時文字壓在亮色遊戲畫面上會失去對比。"""
    ox, oy = x + TEXT_ORIGIN, y + TEXT_ORIGIN
    # 9 個 item 必須連續建立：line_items 靠 id 連號找回它們
    for dx, dy in OUTLINE_OFFSETS:
        canvas.create_text(ox + dx, oy + dy, text=text, fill=OUTLINE, font=font,
                           anchor="nw", width=wrap, tags=("txt", tag))
    fg = canvas.create_text(ox, oy, text=text, fill=fill, font=font, anchor="nw",
                            width=wrap, tags=("txt", tag, "fg"))
    return Line(canvas, fg, tag)


def line_items(line: Line) -> range:
    """該行 9 個文字 item 的 id（描邊在前、本色最後）。

    畫布上物件一多，以 tag 操作就慢（每次都要掃過整張畫布）；逐一以 id 操作則與物件數無關。"""
    return range(line.fg - len(OUTLINE_OFFSETS), line.fg + 1)


def line_origin(line: Line) -> tuple[float, float]:
    """該行的原點（canvas 座標）。"""
    x, y = line.canvas.coords(line.fg)
    return x - TEXT_ORIGIN, y - TEXT_ORIGIN


def line_height(line: Line) -> int:
    """該行（換行後）佔的高度，含底部 2px 餘裕。"""
    # 只量本色（+1＝描邊副本往下多出的 1px）：以 tag 查 bbox 要掃過整張畫布的物件
    return round(line.canvas.bbox(line.fg)[3] + 1 - line_origin(line)[1]) + 2
