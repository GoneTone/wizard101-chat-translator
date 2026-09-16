"""框選框：框選放開後留在畫面上的矩形，讓使用者事後拖邊線改大小、拖框內移動，
放開時回報新矩形（`RegionFlow` 據此對遊戲當下畫面重新辨識翻譯）。

兩層同大小的視窗（矩形外各多一圈 `MARGIN`，把手要露在框外）：
- 框線層（上）：只負責畫。除了亮色框線與把手之外整片是透明色鍵 —— Windows 對色鍵像素
  連 hit-test 都跳過，滑鼠穿過去落到底下的滑鼠層；線與把手本身收到的點擊也交給同一套
  處理，行為一致。
- 滑鼠層（下）：幾乎全透明（`-alpha` 極小但不為零，全零 Windows 就不派事件給它），
  蓋住整個框，邊帶（`GRAB`）縮放、框內移動。可抓範圍因此不受畫出來的線寬限制；
  代價是框開著時框內的遊戲畫面點不到，關卡片一起收掉就恢復。
兩層都不奪焦點（`make_non_activating`）。
邊線只拉該邊、角落拉兩軸（判定沿用 `geometry.edge_at`，見 `hit_at`），四角與四邊中點
的方塊把手純粹是視覺提示；位移一律用螢幕座標算，視窗在拖曳中自己會動、視窗座標會跟著跑。
放開時位移在點擊門檻內視為誤觸：矩形還原、不回報。
"""
import tkinter as tk

from src.log import log
from src.ui.geometry import edge_at, is_click, moved_to, point_in_rect, resized_edge
from src.ui.palette import BG, FG_UPDATE
from src.ui.winstyle import make_non_activating

MARGIN = 6       # 矩形外側多留的寬度（px）：把手露在框外，也是邊帶在框外的那一半
HANDLE = 10      # 把手方塊邊長（px）
MIN_SIZE = 20    # 縮放時的最小寬高（px）
GRAB = 4         # 邊帶在矩形內側的寬度（px）；與 MARGIN 合起來是整條邊帶
_LINE = 3        # 亮色框線寬度（px），以矩形邊為中心線畫
_TRANSPARENT = "#010101"   # 框線層的透明色鍵
_GRIP_ALPHA = 0.01         # 滑鼠層：看不見但收得到滑鼠（0 就收不到）
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
    """視窗座標 (px, py) 壓在哪：邊帶回邊角代號（`"n"`／`"se"`…）、框內回 `"move"`、
    視窗外回空字串。邊帶厚度＝框外留白加框線內側（`MARGIN + GRAB`），角落區再寬一點、
    才好抓到角落把手。"""
    outer_w, outer_h = w + 2 * MARGIN, h + 2 * MARGIN
    if not point_in_rect(px, py, 0, 0, outer_w, outer_h):
        return ""
    return edge_at(px, py, 0, 0, outer_w, outer_h,
                   edge=MARGIN + GRAB, corner=MARGIN + GRAB + HANDLE // 2) or "move"


class RegionBox:
    """一次只有一個；`show(rect)` 在螢幕矩形 (x, y, w, h) 上開框（已開著就搬過去），
    使用者調整完放開後呼叫 `on_change(rect)`（屬性，與 `RegionCard.on_close` 同一種接法）。"""

    def __init__(self, root: tk.Tk):
        self._root = root
        self.on_change = None
        self._win: tk.Toplevel | None = None
        self._grip: tk.Toplevel | None = None
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
            self._grip.destroy()
            self._win = self._grip = self._canvas = None
            self._items = {}
            self._drag = None

    def _build(self) -> None:
        # 滑鼠層先建：兩層都 topmost，後建的疊在上面，框線層要在滑鼠層之上
        grip = tk.Toplevel(self._root)
        grip.withdraw()
        grip.overrideredirect(True)
        grip.attributes("-topmost", True)
        grip.attributes("-alpha", _GRIP_ALPHA)
        grip.configure(bg=BG)

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
        # 兩層同大小、同位置，視窗座標一致，同一套判定與拖曳處理
        for layer in (grip, canvas):
            layer.bind("<ButtonPress-1>", self._press)
            layer.bind("<B1-Motion>", self._motion)
            layer.bind("<ButtonRelease-1>", self._release)
            layer.bind("<Motion>", self._hover)
        self._win, self._grip, self._canvas = win, grip, canvas
        make_non_activating(grip)
        make_non_activating(win)
        grip.deiconify()
        win.deiconify()

    def _apply(self, rect: tuple[int, int, int, int]) -> None:
        """把兩層視窗與畫面上的框線、把手都對到 rect。"""
        self._rect = rect
        x, y, w, h = rect
        win_w, win_h = w + 2 * MARGIN, h + 2 * MARGIN
        for layer in (self._win, self._grip):
            layer.geometry(f"{win_w}x{win_h}+{x - MARGIN}+{y - MARGIN}")
        canvas = self._canvas
        canvas.coords(self._items["line"], MARGIN, MARGIN, MARGIN + w, MARGIN + h)
        half = HANDLE // 2
        for edge, (cx, cy) in _handles(w, h).items():
            canvas.coords(self._items[edge], cx - half, cy - half, cx + half, cy + half)

    def _hover(self, e) -> None:
        if self._drag is None:
            e.widget.configure(cursor=_CURSORS.get(hit_at(e.x, e.y, *self._rect[2:]), ""))

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
