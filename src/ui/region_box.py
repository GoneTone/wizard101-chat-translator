"""框選框：框選放開後留在畫面上的矩形，讓使用者事後拖把手改大小、拖框線移動，
放開時回報新矩形（`RegionFlow` 據此對遊戲當下畫面重新辨識翻譯）。

視窗只比矩形大一圈 `MARGIN`（把手要露在框外），除了亮色框線與把手之外整片是透明
色鍵 —— Windows 對色鍵像素連 hit-test 都跳過，框內的遊戲畫面看得到也點得到，只有畫出
來的框線與把手吃滑鼠；跟疊加視窗一樣不奪焦點（`make_non_activating`）。沒辦法有「看
不見又點得到」的區域，框線要好抓只能靠畫粗一點，不另外加暗色外圈。
四角與四邊中點的方塊把手負責縮放，框線其餘部分負責移動，判定集中在純函式 `hit_at`
（canvas 座標）；位移一律用螢幕座標算，視窗在拖曳中自己會動、canvas 座標會跟著跑。
放開時位移在點擊門檻內視為誤觸：矩形還原、不回報。
"""
import tkinter as tk

from src.log import log
from src.ui.geometry import is_click, moved_to, resized_edge
from src.ui.palette import FG_UPDATE
from src.ui.winstyle import make_non_activating

MARGIN = 5       # 矩形外側多留的寬度（px）：把手一半露在框外
HANDLE = 10      # 把手方塊邊長（px）
MIN_SIZE = 20    # 縮放時的最小寬高（px）
_LINE = 3        # 亮色框線寬度（px），以矩形邊為中心線畫
_BAND = 2        # 框線可抓的範圍：矩形邊往內外各幾 px（略寬於畫出來的線，色鍵像素反正收不到事件）
_TRANSPARENT = "#010101"   # 透明色鍵：框內全填這個色，遊戲畫面從這裡露出來
_CURSORS = {"move": "fleur", "n": "size_ns", "s": "size_ns", "e": "size_we", "w": "size_we",
            "nw": "size_nw_se", "se": "size_nw_se", "ne": "size_ne_sw", "sw": "size_ne_sw"}


def _handles(w: int, h: int) -> dict[str, tuple[int, int]]:
    """八個把手在 canvas 座標的中心點（矩形本身從 (MARGIN, MARGIN) 起算）。"""
    left, right = MARGIN, MARGIN + w
    top, bottom = MARGIN, MARGIN + h
    mid_x, mid_y = MARGIN + w // 2, MARGIN + h // 2
    return {"nw": (left, top), "n": (mid_x, top), "ne": (right, top),
            "w": (left, mid_y), "e": (right, mid_y),
            "sw": (left, bottom), "s": (mid_x, bottom), "se": (right, bottom)}


def hit_at(px: int, py: int, w: int, h: int) -> str:
    """canvas 座標 (px, py) 壓在框的哪個部分：把手回邊角代號（`"se"`／`"n"`…）、
    框線其餘部分回 `"move"`、框內（透明，實際上收不到點擊）與框外回空字串。"""
    half = HANDLE // 2
    for edge, (cx, cy) in _handles(w, h).items():
        if abs(px - cx) <= half and abs(py - cy) <= half:
            return edge
    outer_x = MARGIN - _BAND <= px <= MARGIN + w + _BAND
    outer_y = MARGIN - _BAND <= py <= MARGIN + h + _BAND
    inner_x = MARGIN + _BAND < px < MARGIN + w - _BAND
    inner_y = MARGIN + _BAND < py < MARGIN + h - _BAND
    on_band = outer_x and outer_y and not (inner_x and inner_y)
    return "move" if on_band else ""


class RegionBox:
    """一次只有一個；`show(rect)` 在螢幕矩形 (x, y, w, h) 上開框（已開著就搬過去），
    使用者調整完放開後呼叫 `on_change(rect)`（屬性，與 `RegionCard.on_close` 同一種接法）。"""

    def __init__(self, root: tk.Tk):
        self._root = root
        self.on_change = None
        self._win: tk.Toplevel | None = None
        self._canvas: tk.Canvas | None = None
        self._items: dict[str, int] = {}
        self._rect = (0, 0, 0, 0)
        self._drag: tuple[str, int, int, tuple[int, int, int, int]] | None = None

    @property
    def is_open(self) -> bool:
        return self._win is not None

    def rect(self) -> tuple[int, int, int, int]:
        """目前的框選矩形（螢幕座標）。"""
        return self._rect

    def show(self, rect: tuple[int, int, int, int]) -> None:
        if self._win is None:
            self._build()
        self._drag = None
        self._apply(rect)

    def hide(self) -> None:
        if self._win is not None:
            self._win.destroy()
            self._win = self._canvas = None
            self._items = {}
            self._drag = None

    def _build(self) -> None:
        win = tk.Toplevel(self._root)
        win.withdraw()
        win.overrideredirect(True)
        win.attributes("-topmost", True)
        win.attributes("-transparentcolor", _TRANSPARENT)
        win.configure(bg=_TRANSPARENT)
        canvas = tk.Canvas(win, bg=_TRANSPARENT, highlightthickness=0, bd=0)
        canvas.pack(fill="both", expand=True)
        # 建立順序決定疊放順序：框線在下、把手在上
        self._items = {"line": canvas.create_rectangle(0, 0, 0, 0, outline=FG_UPDATE,
                                                       width=_LINE)}
        for edge in _handles(1, 1):
            self._items[edge] = canvas.create_rectangle(0, 0, 0, 0, fill=FG_UPDATE,
                                                        outline=FG_UPDATE, width=1)
        canvas.bind("<ButtonPress-1>", self._press)
        canvas.bind("<B1-Motion>", self._motion)
        canvas.bind("<ButtonRelease-1>", self._release)
        canvas.bind("<Motion>", self._hover)
        self._win, self._canvas = win, canvas
        make_non_activating(win)
        win.deiconify()

    def _apply(self, rect: tuple[int, int, int, int]) -> None:
        """把視窗與畫面上的框線、把手都對到 rect。"""
        self._rect = rect
        x, y, w, h = rect
        win_w, win_h = w + 2 * MARGIN, h + 2 * MARGIN
        self._win.geometry(f"{win_w}x{win_h}+{x - MARGIN}+{y - MARGIN}")
        canvas = self._canvas
        canvas.coords(self._items["line"], MARGIN, MARGIN, MARGIN + w, MARGIN + h)
        half = HANDLE // 2
        for edge, (cx, cy) in _handles(w, h).items():
            canvas.coords(self._items[edge], cx - half, cy - half, cx + half, cy + half)

    def _hover(self, e) -> None:
        if self._drag is None:
            self._canvas.configure(cursor=_CURSORS.get(hit_at(e.x, e.y, *self._rect[2:]), ""))

    def _press(self, e) -> None:
        hit = hit_at(e.x, e.y, *self._rect[2:])
        self._drag = (hit, e.x_root, e.y_root, self._rect) if hit else None

    def _motion(self, e) -> None:
        if self._drag is None:
            return
        hit, sx, sy, start = self._drag
        dx, dy = e.x_root - sx, e.y_root - sy
        if hit == "move":
            self._apply((*moved_to(start[0], start[1], dx, dy), start[2], start[3]))
        else:
            self._apply(resized_edge(hit, *start, dx, dy, MIN_SIZE, MIN_SIZE))

    def _release(self, e) -> None:
        if self._drag is None:
            return
        _, sx, sy, start = self._drag
        self._drag = None
        if is_click(e.x_root - sx, e.y_root - sy):
            self._apply(start)   # 誤觸：位置還原、不回報
            return
        if self._rect != start and self.on_change is not None:
            log(f"[region] box adjusted (rect={self._rect})")
            self.on_change(self._rect)
