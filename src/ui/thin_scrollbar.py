"""疊加視窗用的自繪細捲軸。

Windows 的原生捲軸忽略 bg／troughcolor（實測改色無效），只能自繪才配得上深色
疊加視窗；介面與 tk.Scrollbar 相容（set／command），可直接接 yscrollcommand。"""
import tkinter as tk

from src.ui.palette import BG, OUTLINE

_SCROLLBAR_WIDTH = 8
_MIN_THUMB = 20      # 滑塊最短長度（px）：訊息很多時仍抓得住
_THUMB = "#8a8ab0"
_THUMB_HOVER = "#c0c0e0"


def thumb_span(first: float, last: float, track_height: int,
               min_thumb: int = _MIN_THUMB) -> tuple[int, int] | None:
    """捲軸滑塊在軌道上的 (top, bottom) 像素範圍；內容塞得下＝不需捲動時回 None。
    比例算出的長度不足 min_thumb 時就撐到 min_thumb（並保持不超出軌道）。"""
    if track_height <= 0 or last - first >= 1.0:
        return None
    top = round(first * track_height)
    bottom = round(last * track_height)
    if bottom - top < min_thumb:
        bottom = min(track_height, top + min_thumb)
        top = max(0, bottom - min_thumb)
    return top, bottom


def scroll_fraction(pointer_y: int, grab_offset: float, track_height: int) -> float:
    """拖曳滑塊時的 yview_moveto 比例：游標位置扣掉抓取點偏移，夾在 0–1。"""
    if track_height <= 0:
        return 0.0
    return min(1.0, max(0.0, pointer_y / track_height - grab_offset))


class ThinScrollbar(tk.Canvas):
    """自繪細捲軸。軌道留 BG＝視窗的透明色鍵，露出底下那片半透明底板。"""

    def __init__(self, parent, command, width: int = _SCROLLBAR_WIDTH):
        # Canvas 預設要求 7cm 高，會把靠內容決定高度的容器（框選結果卡片）撐大；高度交給容器
        super().__init__(parent, width=width, height=1, bg=BG, highlightthickness=0, bd=0)
        self._command = command
        self._pad = 2
        self._thickness = width - self._pad * 2
        self._first, self._last = 0.0, 1.0
        self._grab_offset = 0.0
        self._color = _THUMB
        self.bind("<Configure>", lambda e: self._redraw())
        self.bind("<ButtonPress-1>", self._press)
        self.bind("<B1-Motion>", self._drag)
        self.bind("<Enter>", lambda e: self._recolor(_THUMB_HOVER))
        self.bind("<Leave>", lambda e: self._recolor(_THUMB))

    def set(self, first, last) -> None:
        """yscrollcommand 介面：目前可見範圍的起訖比例。"""
        self._first, self._last = float(first), float(last)
        self._redraw()

    def _recolor(self, color: str) -> None:
        self._color = color
        self._redraw()

    def _redraw(self) -> None:
        self.delete("thumb")
        span = thumb_span(self._first, self._last, self.winfo_height())
        if span is None:
            return  # 不需捲動：整條隱形
        top, bottom = span
        # 先畫大一圈的深色描邊再疊本色：底板半透明，壓在亮色畫面上會整片提亮，沒有
        # 描邊的滑塊就融進背景看不見（與 overlay 的 _outlined_line 同一套處理）。
        self._draw_thumb(top, bottom, OUTLINE, grow=1)
        self._draw_thumb(top, bottom, self._color, grow=0)

    def _draw_thumb(self, top: int, bottom: int, color: str, grow: int) -> None:
        """圓角滑塊＝上下各一個圓 + 中間矩形（Canvas 沒有圓角矩形）。
        grow 讓整個形狀往外長一圈，用來畫描邊。"""
        x0, x1 = self._pad - grow, self._pad + self._thickness + grow
        top, bottom, r = top - grow, bottom + grow, x1 - x0
        self.create_oval(x0, top, x1, top + r, fill=color, outline="", tags="thumb")
        self.create_oval(x0, bottom - r, x1, bottom, fill=color, outline="", tags="thumb")
        self.create_rectangle(x0, top + r / 2, x1, bottom - r / 2, fill=color,
                              outline="", tags="thumb")

    def _press(self, e) -> None:
        height = max(self.winfo_height(), 1)
        self._grab_offset = e.y / height - self._first

    def _drag(self, e) -> None:
        self._command("moveto", scroll_fraction(e.y, self._grab_offset,
                                                self.winfo_height()))
