"""疊加視窗：無邊框、置頂、半透明、固定大小、可拖曳移動與縮放、可滾動。
顯示原文 + 譯文（最新在最下，可向上滾動看歷史）。
捲動定位：在底部時新訊息自動跟到最底；向上捲看歷史時不會被硬拉回底部。
不滑鼠穿透 —— 視窗蓋住的區域點擊不會傳到遊戲，視窗永遠可互動。"""
import sys
import time
import tkinter as tk
from typing import NamedTuple

import win32con
import win32gui

from src.config import APP_NAME

BG = "#101018"
BAR = "#23233a"
GRIP = "#3a3a55"
FG_ORIGINAL = "#c0c0cd"
FG_TRANSLATED = "#f2f2f7"
# 原文對譯文的調暗係數：原文是輔助資訊，壓暗到譯文之下讓視線先落在譯文上
DIM_FACTOR = 0.71
FG_PENDING = "#9398a8"  # 佔位中的譯文：比原文更暗，一眼看出這則還沒翻好
FG_ERROR = "#ff5f5f"
FG_BAR = "#c8c8d8"

_OUTLINE = "#0a0a10"  # 字幕描邊色:深色輪廓讓文字在任何遊戲畫面上都保有對比
_OUTLINE_OFFSETS = ((-1, -1), (-1, 0), (-1, 1), (0, -1),
                    (0, 1), (1, -1), (1, 0), (1, 1))
_FONT_ORIGINAL = ("Microsoft JhengHei", 9)
_FONT_TRANSLATED = ("Microsoft JhengHei", 11)


def _fit_line_height(c: "tk.Canvas") -> None:
    """把文字行 canvas 的高度縮放到剛好容納（換行後的）文字內容。"""
    bbox = c.bbox("all")
    if bbox:
        c.configure(height=bbox[3] + 2)


def _outlined_line(parent, text: str, fg: str, font: tuple, wrap: int) -> "tk.Canvas":
    """字幕式描邊文字行：canvas 先畫八方向 1px 偏移的描邊副本、再疊本色——
    Label 無法描邊，透明度調低時文字壓在亮色遊戲畫面上會失去對比。"""
    c = tk.Canvas(parent, bg=BG, highlightthickness=0, bd=0)
    for dx, dy in _OUTLINE_OFFSETS:
        c.create_text(2 + dx, 2 + dy, text=text, fill=_OUTLINE, font=font,
                      anchor="nw", width=wrap, tags="txt")
    # 本色最後畫，疊在描邊之上。額外掛 "fg" tag：改色時只動本色，描邊不能跟著變
    c.create_text(2, 2, text=text, fill=fg, font=font, anchor="nw",
                  width=wrap, tags=("txt", "fg"))
    _fit_line_height(c)
    return c

def dimmed(color: str, factor: float = DIM_FACTOR) -> str:
    """把 `#rrggbb` 各通道乘上係數調暗——原文行用遊戲色的暗版，維持原文暗、譯文亮的層次。"""
    r, g, b = (int(color[i:i + 2], 16) for i in (1, 3, 5))
    return f"#{round(r * factor):02x}{round(g * factor):02x}{round(b * factor):02x}"


class _Message(NamedTuple):
    """overlay 中的一則訊息。msg_id 為 None 代表不需要就地更新（例如測試直接塞完成品）；
    color 為該則在遊戲內的顯示色（None＝退回預設配色）。"""
    ts: float
    original: str
    translated: str
    row: "tk.Frame"
    msg_id: int | None
    color: str | None = None


MIN_WIDTH = 200
MIN_HEIGHT = 90
_BAR_HEIGHT = 20
_GRIP_SIZE = 16
_STICK_THRESHOLD = 0.999
_BUBBLE_SIZE = 64
_CLICK_THRESHOLD = 5
_FOREGROUND_POLL_MS = 300
_BACKDROP_POLL_MS = 1000
_TRANSPARENT = "#010101"  # 泡泡視窗的透明色鍵（方形視窗只露出圓形）
_SCROLLBAR_WIDTH = 8
_MIN_THUMB = 20      # 滑塊最短長度（px）：訊息很多時仍抓得住
_THUMB = "#8a8ab0"
_THUMB_HOVER = "#c0c0e0"


def moved_to(start_x: int, start_y: int, dx: int, dy: int) -> tuple[int, int]:
    """拖曳位移後的新左上角座標。"""
    return start_x + dx, start_y + dy


def resized_to(start_w: int, start_h: int, dx: int, dy: int,
               min_w: int, min_h: int) -> tuple[int, int]:
    """拖曳縮放後的新寬高（不小於最小值）。"""
    return max(min_w, start_w + dx), max(min_h, start_h + dy)


def should_stick_to_bottom(view_bottom_fraction: float,
                           threshold: float = _STICK_THRESHOLD) -> bool:
    """視圖底緣接近最底時，新訊息應自動跟到底；使用者往上捲時則否。"""
    return view_bottom_fraction >= threshold


def is_click(dx: int, dy: int, threshold: int = _CLICK_THRESHOLD) -> bool:
    """按下到放開的位移是否算點擊（否則視為拖曳）。"""
    return abs(dx) < threshold and abs(dy) < threshold


def point_in_rect(px: int, py: int, x: int, y: int, w: int, h: int) -> bool:
    """(px, py) 是否落在左上角 (x, y)、寬 w 高 h 的矩形內（含邊界）。"""
    return x <= px <= x + w - 1 and y <= py <= y + h - 1


def should_auto_expand(foreground: int, previous: int, bubble_hwnd: int,
                       cursor_on_bubble: bool) -> bool:
    """泡泡被切成前景（點工作列按鈕／Alt+Tab）時是否該自動展開回完整視窗。

    只認「這一輪才變成前景」的轉換，持續在前景時不重複觸發；游標壓在泡泡上
    代表使用者正直接操作泡泡，交給既有的按下／拖曳／放開邏輯處理——否則按下
    的瞬間就展開，泡泡再也拖不動。bubble_hwnd 取不到（0）時一律不觸發，
    避免與 GetForegroundWindow() 的 0（無前景視窗）誤判成相等。"""
    if not bubble_hwnd or foreground != bubble_hwnd or previous == bubble_hwnd:
        return False
    return not cursor_on_bubble


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
    """自繪細捲軸，介面與 tk.Scrollbar 相容（set／command），可直接接 yscrollcommand。
    Windows 的原生捲軸忽略 bg／troughcolor（實測改色無效），只能自繪才配得上深色
    疊加視窗；軌道留 BG＝視窗的透明色鍵，露出底下那片半透明底板。"""

    def __init__(self, parent, command, width: int = _SCROLLBAR_WIDTH):
        super().__init__(parent, width=width, bg=BG, highlightthickness=0, bd=0)
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
        # 先畫大一圈的深色描邊再疊本色。底板是半透明的，合成後的亮度取決於視窗後面
        # 是什麼——壓在亮色畫面上時整片會被提亮，沒有描邊的滑塊就融進背景看不見了。
        # 與訊息文字同一套處理（見 _outlined_line）。
        self._draw_thumb(top, bottom, _OUTLINE, grow=1)
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


def _make_non_activating(win: tk.Toplevel) -> None:
    """讓視窗攔截滑鼠事件但點擊不奪焦點、不改變疊序（WS_EX_NOACTIVATE）。
    用於底板：點到透明背景區不會穿到遊戲，也不會把底板抬到文字層之上。
    失敗的後果是點擊底板可能改變疊序，另有 lower() 保險擋著。"""
    try:
        win.update_idletasks()
        hwnd = win32gui.GetAncestor(win.winfo_id(), 2)  # GA_ROOT
        style = win32gui.GetWindowLong(hwnd, win32con.GWL_EXSTYLE)
        style |= win32con.WS_EX_NOACTIVATE
        win32gui.SetWindowLong(hwnd, win32con.GWL_EXSTYLE, style)
    except Exception as exc:
        print(f"[ui] non-activating setup failed: {exc}", file=sys.stderr)


def _first_visible_below(hwnd: int) -> int:
    """疊序上位於 hwnd 正後方的第一個**可見**視窗（0＝沒有）。
    GW_HWNDNEXT 連隱藏視窗也一起走訪，直接取下一個會把早已 withdraw 的視窗
    當成插隊者——只有看得見的視窗才會真的擋住透明區露出的內容。"""
    nxt = win32gui.GetWindow(hwnd, win32con.GW_HWNDNEXT)
    while nxt and not win32gui.IsWindowVisible(nxt):
        nxt = win32gui.GetWindow(nxt, win32con.GW_HWNDNEXT)
    return nxt


def _enable_taskbar_button(win: tk.Toplevel, alpha: float | None = None) -> None:
    """讓無邊框視窗出現在工作列與 Alt+Tab。
    overrideredirect 視窗預設拿不到工作列按鈕，把 WS_EX_APPWINDOW 加進
    extended style 即可；需 withdraw→deiconify 一次讓樣式生效，
    之後重設 topmost（與 alpha，若有）。失敗只是少個按鈕，不影響功能。"""
    try:
        win.update_idletasks()
        hwnd = win32gui.GetAncestor(win.winfo_id(), 2)  # GA_ROOT
        style = win32gui.GetWindowLong(hwnd, win32con.GWL_EXSTYLE)
        style = (style & ~win32con.WS_EX_TOOLWINDOW) | win32con.WS_EX_APPWINDOW
        win32gui.SetWindowLong(hwnd, win32con.GWL_EXSTYLE, style)
        win.withdraw()
        win.deiconify()
        win.attributes("-topmost", True)
        if alpha is not None:
            win.attributes("-alpha", alpha)
    except Exception as exc:
        print(f"[ui] taskbar button setup failed: {exc}", file=sys.stderr)


class OverlayWindow:
    def __init__(self, root: tk.Tk, x: int | None, y: int | None,
                 width: int = 640, height: int = 420,
                 max_messages: int = 50, fade_seconds: int = 180,
                 on_geometry_change=None, on_settings=None, on_close=None,
                 bubble_position: dict | None = None, on_bubble_move=None,
                 alpha: float = 0.80):
        self._max = max_messages
        self._fade = fade_seconds
        self._alpha = alpha
        self._on_geometry_change = on_geometry_change
        self._on_bubble_move = on_bubble_move
        self._bubble_pos = dict(bubble_position) if bubble_position else {"x": None, "y": None}
        self._minimized = False
        self._unread = 0
        self._bubble: tk.Toplevel | None = None
        self._bubble_hwnd = 0
        # 泡泡按下的起點；None＝這次放開沒有對應的按下（見 _bubble_release）
        self._bubble_drag_state: tuple[int, int, int, int] | None = None
        self._prev_foreground = 0
        self._watch_job: str | None = None
        self._backdrop_intact = True
        self._messages: list[_Message] = []
        # 視圖是否黏在底部。只在使用者主動捲動時重新評估，不在每次加訊息時當場採樣——
        # 縮放視窗／錯誤橫幅進出都會把視圖推離底部，當場採樣會把它誤判成「使用者往上捲」。
        self._follow = True
        self._error_label: tk.Label | None = None
        self._w = max(width, MIN_WIDTH)
        self._h = max(height, MIN_HEIGHT)
        self._wrap = self._w - 40
        self._drag = (0, 0, 0, 0)

        # 雙層視窗:tk 的 -alpha 是整窗生效、無法只透背景,故拆兩層——
        # 下層 backdrop 承擔半透明底板(透明度設定作用於此),
        # 上層本體以 -transparentcolor 把背景色挖空,文字與控制項保持完全不透明。
        # 本體由 backdrop 擁有(owned window),Windows 保證永遠疊在其上。
        self._backdrop = tk.Toplevel(root)
        self._backdrop.overrideredirect(True)
        self._backdrop.attributes("-topmost", True)
        self._backdrop.attributes("-alpha", self._alpha)
        self._backdrop.configure(bg=BG)
        # 底板攔截透明背景區的滑鼠事件(不穿透到遊戲),但點擊不奪焦點、不改疊序;
        # 空白處的滾輪由底板轉發給訊息區捲動。
        _make_non_activating(self._backdrop)
        self._backdrop.bind("<MouseWheel>", self._on_wheel)

        # master 用 root 而非 backdrop：Tk 的 master 連動 restack 會在點擊本體時
        # 把 backdrop 一起抬起、反而蓋過文字層（實測）；OS 擁有關係於下方另設。
        self._win = tk.Toplevel(root)
        self._win.overrideredirect(True)
        self._win.attributes("-topmost", True)
        self._win.attributes("-transparentcolor", BG)
        self._win.configure(bg=BG)
        # 未設定過位置（首次啟動）：擺螢幕正中央，比擺角落更容易被注意到
        px = x if x is not None else (self._win.winfo_screenwidth() - self._w) // 2
        py = y if y is not None else (self._win.winfo_screenheight() - self._h) // 2
        self._apply_geometry(px, py, self._w, self._h)

        # 標題列（可拖曳移動）
        bar = tk.Frame(self._win, bg=BAR, height=_BAR_HEIGHT, cursor="fleur")
        bar.pack(side="top", fill="x")
        bar.pack_propagate(False)
        label = tk.Label(bar, text=f"≡  {APP_NAME}", bg=BAR, fg=FG_BAR,
                         font=("Microsoft JhengHei", 8), anchor="w")
        label.pack(side="left", padx=6)
        # side="right" 先 pack 者占最外側：由右到左依序為 ✕、⚙、狀態字。
        # overlay 是無邊框視窗、打包版沒有主控台，✕ 是唯一的正常關閉途徑。
        if on_close is not None:
            close = tk.Label(bar, text="✕", bg=BAR, fg=FG_BAR,
                             font=("Microsoft JhengHei", 9), cursor="hand2")
            close.pack(side="right", padx=(0, 6))
            close.bind("<Button-1>", lambda e: on_close())
        mini = tk.Label(bar, text="─", bg=BAR, fg=FG_BAR,
                        font=("Microsoft JhengHei", 9), cursor="hand2")
        mini.pack(side="right", padx=(0, 4))
        mini.bind("<Button-1>", lambda e: self.minimize())
        if on_settings is not None:
            gear = tk.Label(bar, text="⚙", bg=BAR, fg=FG_BAR,
                            font=("Microsoft JhengHei", 9), cursor="hand2")
            gear.pack(side="right", padx=(0, 4))
            gear.bind("<Button-1>", lambda e: on_settings())
        self._status_label = tk.Label(bar, text="", bg=BAR, fg=FG_BAR,
                                      font=("Microsoft JhengHei", 8), anchor="e")
        self._status_label.pack(side="right", padx=6)
        for w in (bar, label, self._status_label):
            w.bind("<ButtonPress-1>", self._move_start)
            w.bind("<B1-Motion>", self._move_drag)
            w.bind("<ButtonRelease-1>", lambda e: self._emit_geometry())

        # 內容區：錯誤橫幅（固定在下，不隨捲動）+ 可滾動訊息區
        self._frame = tk.Frame(self._win, bg=BG)
        self._frame.pack(side="top", fill="both", expand=True)

        scroll_area = tk.Frame(self._frame, bg=BG)
        scroll_area.pack(side="top", fill="both", expand=True)
        self._canvas = tk.Canvas(scroll_area, bg=BG, highlightthickness=0)
        self._scrollbar = ThinScrollbar(scroll_area, command=self._user_scroll)
        self._scrollbar.bind("<MouseWheel>", self._on_wheel)  # 游標壓在捲軸上也能滾
        self._canvas.configure(yscrollcommand=self._scrollbar.set)
        # 底部讓出縮放把手的高度：把手 place 在視窗右下角，捲軸鋪到底會被它壓住
        self._scrollbar.pack(side="right", fill="y", pady=(0, _GRIP_SIZE))
        self._canvas.pack(side="left", fill="both", expand=True)
        self._inner = tk.Frame(self._canvas, bg=BG)
        self._inner_id = self._canvas.create_window((0, 0), window=self._inner, anchor="nw")
        self._inner.bind(
            "<Configure>",
            lambda e: self._canvas.configure(scrollregion=self._canvas.bbox("all")),
        )
        self._canvas.bind("<Configure>", self._on_canvas_configure)
        # 滑鼠移入時啟用滾輪捲動
        self._canvas.bind("<Enter>", lambda e: self._canvas.bind_all("<MouseWheel>", self._on_wheel))
        self._canvas.bind("<Leave>", lambda e: self._canvas.unbind_all("<MouseWheel>"))

        # 空狀態提示：沒有任何訊息時，把目前狀態大字顯示在視窗正中間
        self._placeholder = tk.Label(scroll_area, text="", bg=BG, fg=FG_BAR,
                                     font=("Microsoft JhengHei", 11))
        self._placeholder.place(relx=0.5, rely=0.5, anchor="center")

        # 右下角縮放把手：只畫斜線、背景留透明色鍵，才不會在遊戲畫面上多出一塊方形。
        # 代價是只有線條那些實心像素接得到滑鼠（透明色鍵的像素在 Windows 下點不穿），
        # 所以線畫粗一點把抓取範圍撐回來；亮色配描邊，任何背景上都有對比。
        grip = tk.Canvas(self._win, width=_GRIP_SIZE, height=_GRIP_SIZE, bg=BG,
                         highlightthickness=0, bd=0, cursor="size_nw_se")
        for inset in (4, 9, 14):
            ends = (_GRIP_SIZE - inset, _GRIP_SIZE - 2, _GRIP_SIZE - 2, _GRIP_SIZE - inset)
            grip.create_line(*ends, fill=_OUTLINE, width=4)
            grip.create_line(*ends, fill=FG_BAR, width=2)
        grip.place(relx=1.0, rely=1.0, anchor="se")
        grip.bind("<ButtonPress-1>", self._resize_start)
        grip.bind("<B1-Motion>", self._resize_drag)
        grip.bind("<ButtonRelease-1>", lambda e: self._emit_geometry())

        self._win.title(APP_NAME)  # 工作列按鈕顯示的名稱
        _enable_taskbar_button(self._win)  # 文字層不透明，不需重設 alpha
        # 用 Win32 直接建立 OS 擁有關係：owned window 在 OS 層永遠疊在 owner 之上，
        # 任何點擊／啟用都不會反轉（Tk 的 master 參數實測不會設定 GW_OWNER）。
        try:
            win_hwnd = win32gui.GetAncestor(self._win.winfo_id(), 2)
            bd_hwnd = win32gui.GetAncestor(self._backdrop.winfo_id(), 2)
            win32gui.SetWindowLong(win_hwnd, win32con.GWL_HWNDPARENT, bd_hwnd)
        except Exception as exc:
            print(f"[ui] owner setup failed: {exc}", file=sys.stderr)
        self._backdrop.lower(self._win)  # 疊序保險：底板壓在文字層之下
        self._win.after(_BACKDROP_POLL_MS, self._watch_backdrop)

    def _watch_backdrop(self) -> None:
        """定期確認半透明底板還在文字層正後方。

        文字層是用透明色鍵挖空的，透明區露出的就是它正後方那個視窗。正常情況下
        那是底板（那層暗底），捲軸滑塊與右下角縮放把手全靠它撐出對比；底板一旦被
        藏起來或被別的視窗插進中間，露出的變成遊戲畫面，這兩者就會突然「消失」
        （訊息有描邊，所以照常讀得到——症狀只挑細小的元件下手，很難自己看出原因）。

        目前只記錄、不自動修復：得先從 app.log 分辨是「底板被藏起來」還是
        「疊序被插隊」，才知道該補 deiconify 還是補 lower，否則修法只是猜的。"""
        self._win.after(_BACKDROP_POLL_MS, self._watch_backdrop)
        if self._minimized:
            return   # 泡泡狀態下兩層本來就都收起來了
        try:
            text_layer = win32gui.GetAncestor(self._win.winfo_id(), 2)      # GA_ROOT
            backdrop = win32gui.GetAncestor(self._backdrop.winfo_id(), 2)
            visible = bool(win32gui.IsWindowVisible(backdrop))
            below = _first_visible_below(text_layer)
        except Exception as exc:
            print(f"[ui] backdrop check failed: {exc}", file=sys.stderr)
            return
        intact = visible and below == backdrop
        if intact == self._backdrop_intact:
            return
        self._backdrop_intact = intact
        if intact:
            print("[ui] backdrop is back behind the text layer", file=sys.stderr)
        else:
            print(f"[ui] backdrop lost from behind the text layer "
                  f"(backdrop_visible={visible}, backdrop=0x{backdrop:x}, "
                  f"below_text_layer=0x{below:x})", file=sys.stderr)

    # --- 縮小成泡泡 ---
    @property
    def minimized(self) -> bool:
        """是否處於縮小（泡泡）狀態。"""
        return self._minimized

    def unread_count(self) -> int:
        """縮小期間累積的未讀訊息數（展開歸零）。"""
        return self._unread

    def minimize(self) -> None:
        """縮小成浮動泡泡：隱藏本體（訊息照常累積），點泡泡展開、拖曳移動。"""
        if self._minimized:
            return
        self._win.update_idletasks()
        if self._bubble_pos.get("x") is None:
            # 無記憶位置：預設出現在 overlay 右上角（縮小按鈕附近），視覺上「收進泡泡」
            self._bubble_pos = {
                "x": self._win.winfo_x() + self._win.winfo_width() - _BUBBLE_SIZE,
                "y": self._win.winfo_y(),
            }
        self._minimized = True
        self._unread = 0
        self._win.withdraw()
        self._backdrop.withdraw()
        self._show_bubble()
        # 泡泡剛建立時可能已經是前景（_enable_taskbar_button 的 deiconify 會啟用它），
        # 拿當下的前景當基準才不會第一輪就誤判成「使用者切回本工具」
        self._prev_foreground = self._foreground_window()
        self._watch_job = self._win.after(_FOREGROUND_POLL_MS, self._watch_foreground)
        print(f"[ui] minimized to bubble ({self._scroll_debug()})", file=sys.stderr)

    def expand(self) -> None:
        """從泡泡展開回完整視窗，未讀歸零。"""
        if not self._minimized:
            return
        if self._watch_job is not None:
            self._win.after_cancel(self._watch_job)
            self._watch_job = None
        self._minimized = False
        self._unread = 0
        if self._bubble is not None:
            self._bubble.destroy()
            self._bubble = None
        self._backdrop.deiconify()
        self._backdrop.attributes("-topmost", True)
        self._backdrop.attributes("-alpha", self._alpha)
        self._win.deiconify()
        self._win.attributes("-topmost", True)
        self._backdrop.lower(self._win)  # 疊序保險：底板永遠壓在文字層之下
        # 重新對齊排到 idle：deiconify 只是送出顯示要求，這一行執行時畫布仍是隱藏的，
        # 當場重算會量到（也擺放到）舊值。等 map 真的完成後再做，才對得齊。
        self._win.after_idle(self._settle_after_expand)

    def _settle_after_expand(self) -> None:
        """視窗重新顯示後把捲動狀態接回來。泡泡期間進來的訊息是在隱藏狀態下排版的，
        必須自己重算——不能指望 deiconify 一定會帶來 <Configure>（幾何沒變就沒有事件）。"""
        self._refresh_scroll()
        print(f"[ui] expanded ({self._scroll_debug()})", file=sys.stderr)

    def _show_bubble(self) -> None:
        b = tk.Toplevel(self._win)
        b.overrideredirect(True)
        b.attributes("-topmost", True)
        b.attributes("-transparentcolor", _TRANSPARENT)
        b.attributes("-alpha", self._alpha)
        b.configure(bg=_TRANSPARENT)
        b.geometry(f"{_BUBBLE_SIZE}x{_BUBBLE_SIZE}"
                   f"+{self._bubble_pos['x']}+{self._bubble_pos['y']}")
        c = tk.Canvas(b, width=_BUBBLE_SIZE, height=_BUBBLE_SIZE,
                      bg=_TRANSPARENT, highlightthickness=0)
        c.pack()

        def px(f: float) -> int:
            return round(_BUBBLE_SIZE * f)  # 圖示座標按泡泡尺寸等比縮放

        c.create_oval(2, 2, _BUBBLE_SIZE - 2, _BUBBLE_SIZE - 2,
                      fill=BAR, outline=GRIP, width=2)
        # 圖示：兩個交疊的對話泡泡（翻譯意象），Canvas 直接繪製、不依賴圖檔
        c.create_oval(px(0.23), px(0.30), px(0.59), px(0.59),
                      outline=FG_TRANSLATED, width=2)
        c.create_polygon(px(0.32), px(0.57), px(0.43), px(0.57), px(0.27), px(0.70),
                         fill=FG_TRANSLATED)
        c.create_oval(px(0.48), px(0.45), px(0.80), px(0.73),
                      outline=FG_ORIGINAL, width=2)
        self._badge = c.create_text(_BUBBLE_SIZE - px(0.23), px(0.20), text="",
                                    fill="#ff9090",
                                    font=("Microsoft JhengHei", 9, "bold"))
        c.bind("<ButtonPress-1>", self._bubble_press)
        c.bind("<B1-Motion>", self._bubble_drag)
        c.bind("<ButtonRelease-1>", self._bubble_release)
        self._bubble = b
        self._bubble_canvas = c
        b.title(APP_NAME)
        _enable_taskbar_button(b)
        try:
            self._bubble_hwnd = win32gui.GetAncestor(b.winfo_id(), 2)  # GA_ROOT
        except Exception as exc:
            self._bubble_hwnd = 0
            print(f"[ui] bubble hwnd lookup failed: {exc}", file=sys.stderr)

    def _foreground_window(self) -> int:
        try:
            return win32gui.GetForegroundWindow()
        except Exception as exc:
            print(f"[ui] foreground lookup failed: {exc}", file=sys.stderr)
            return 0

    def _watch_foreground(self) -> None:
        """泡泡狀態下輪詢前景視窗，本工具被切回前景就展開。
        overrideredirect 視窗不是 OS 意義上的最小化，收不到還原通知，
        點工作列按鈕／Alt+Tab 只會把泡泡切成前景，只能自己輪詢察覺。"""
        self._watch_job = None
        if not self._minimized or self._bubble is None:
            return
        fg = self._foreground_window()
        try:
            px, py = self._bubble.winfo_pointerxy()
            on_bubble = point_in_rect(px, py, self._bubble.winfo_x(),
                                      self._bubble.winfo_y(),
                                      _BUBBLE_SIZE, _BUBBLE_SIZE)
        except Exception as exc:
            print(f"[ui] bubble hit-test failed: {exc}", file=sys.stderr)
            on_bubble = True  # 測不到就當作使用者正壓著泡泡，寧可不展開
        expand = should_auto_expand(fg, self._prev_foreground,
                                    self._bubble_hwnd, on_bubble)
        self._prev_foreground = fg
        if expand:
            print(f"[ui] auto-expand: bubble brought to foreground, fg=0x{fg:x}",
                  file=sys.stderr)
            self.expand()
            return
        self._watch_job = self._win.after(_FOREGROUND_POLL_MS, self._watch_foreground)

    def _bubble_press(self, e) -> None:
        self._bubble_drag_state = (e.x_root, e.y_root,
                                   self._bubble.winfo_x(), self._bubble.winfo_y())

    def _bubble_drag(self, e) -> None:
        if self._bubble_drag_state is None:
            return
        sx, sy, ox, oy = self._bubble_drag_state
        nx, ny = moved_to(ox, oy, e.x_root - sx, e.y_root - sy)
        self._bubble.geometry(f"+{nx}+{ny}")

    def _bubble_release(self, e) -> None:
        # 點 ─ 縮小時 minimize() 會 withdraw 掉正被按住的視窗、隱式 grab 因此斷掉，
        # 放開滑鼠的事件落到剛出現在游標下的泡泡上——沒有對應的按下，當作沒發生
        if self._bubble_drag_state is None:
            return
        sx, sy, _, _ = self._bubble_drag_state
        self._bubble_drag_state = None
        if is_click(e.x_root - sx, e.y_root - sy):
            self.expand()
            return
        self._bubble_pos = {"x": self._bubble.winfo_x(), "y": self._bubble.winfo_y()}
        if self._on_bubble_move is not None:
            self._on_bubble_move(self._bubble_pos["x"], self._bubble_pos["y"])

    def set_alpha(self, alpha: float) -> None:
        """套用新的視窗不透明度（半透明底板與泡泡即時生效；文字層恆為不透明）。"""
        self._alpha = alpha
        self._backdrop.attributes("-alpha", alpha)
        if self._bubble is not None:
            self._bubble.attributes("-alpha", alpha)

    def _update_badge(self) -> None:
        if self._bubble is None:
            return
        text = "99+" if self._unread > 99 else (str(self._unread) if self._unread else "")
        self._bubble_canvas.itemconfigure(self._badge, text=text)

    # --- 幾何 ---
    def _apply_geometry(self, x: int, y: int, w: int, h: int) -> None:
        self._w, self._h = w, h
        self._wrap = w - 40
        self._win.geometry(f"{w}x{h}+{x}+{y}")
        self._backdrop.geometry(f"{w}x{h}+{x}+{y}")

    def _emit_geometry(self) -> None:
        if self._on_geometry_change is None:
            return
        self._win.update_idletasks()
        self._on_geometry_change(self._win.winfo_x(), self._win.winfo_y(),
                                 self._win.winfo_width(), self._win.winfo_height())

    def _on_canvas_configure(self, e) -> None:
        # 內層寬度跟著畫布寬，文字才會依視窗寬換行
        self._canvas.itemconfigure(self._inner_id, width=e.width)
        self._wrap = max(80, e.width - 12)
        # 既有訊息與錯誤橫幅的換行寬度也要同步更新，否則縮小視窗後右緣被切
        for entry in self._messages:
            for child in entry.row.winfo_children():
                child.itemconfigure("txt", width=self._wrap)
                _fit_line_height(child)
        if self._error_label is not None:
            self._error_label.configure(wraplength=self._wrap)
        # 排到 idle 再貼底，不在事件處理中直接 update_idletasks()——那會讓下一個
        # Configure 事件重入本函式；此時排版也尚未完成，量到的高度是舊的。
        self._win.after_idle(self._refresh_scroll)

    def _on_wheel(self, e) -> None:
        self._canvas.yview_scroll(int(-e.delta / 120), "units")
        self._note_scroll()

    def _user_scroll(self, *args) -> None:
        """捲軸拖曳的入口（yview 的包裝）：捲完順手記下使用者要不要繼續跟隨底部。"""
        self._canvas.yview(*args)
        self._note_scroll()

    def _scroll_debug(self) -> str:
        """捲動狀態快照，供 log 定位「訊息看不到／捲不到底」這類回報。
        scrollregion 與 bbox 的高度對不上，就代表捲動範圍是舊的。"""
        region = self._canvas.cget("scrollregion").split()
        bbox = self._canvas.bbox("all")
        return (f"messages={len(self._messages)} follow={self._follow} "
                f"yview_bottom={self._canvas.yview()[1]:.4f} "
                f"region_h={region[3] if len(region) == 4 else '?'} "
                f"content_h={bbox[3] if bbox else '?'} "
                f"canvas_h={self._canvas.winfo_height()}")

    def _note_scroll(self) -> None:
        """使用者主動捲動後重新判定是否繼續跟隨底部：往上捲＝正在讀歷史，
        新訊息不該把畫面搶走；捲回底部則恢復跟隨。"""
        follow = should_stick_to_bottom(self._canvas.yview()[1])
        if follow != self._follow:
            self._follow = follow
            print(f"[ui] auto-follow {'enabled' if follow else 'disabled'} "
                  f"by user scroll ({self._scroll_debug()})", file=sys.stderr)

    def _refresh_scroll(self) -> None:
        """重算捲動範圍，並在跟隨模式下把視圖貼回底部。

        任何改變畫布內容或幾何的動作都要呼叫：新增／更新訊息、清掉過期訊息，
        以及縮放視窗與錯誤橫幅進出所觸發的重新排版。少呼叫一處的後果不是少捲一次，
        而是視圖從此停在舊位置——`_follow` 仍為真但沒人把它貼回底部，
        之後每一則新訊息都落在畫面外，看起來就像訊息漏掉了。"""
        self._canvas.update_idletasks()
        # 視窗隱藏（縮成泡泡、工作列收合）期間畫布不重繪，內嵌的訊息容器就停止跟著
        # 捲動位置移動，與畫布自己的捲動帳目脫節——yview 回報已在底部，畫面卻少了
        # 最後幾則、往下也捲不動。重設一次座標（值不變）即可要求畫布重新擺放它。
        self._canvas.coords(self._inner_id, 0, 0)
        self._canvas.configure(scrollregion=self._canvas.bbox("all"))
        if self._follow:
            self._canvas.yview_moveto(1.0)

    def _move_start(self, e) -> None:
        self._drag = (e.x_root, e.y_root, self._win.winfo_x(), self._win.winfo_y())

    def _move_drag(self, e) -> None:
        sx, sy, ox, oy = self._drag
        nx, ny = moved_to(ox, oy, e.x_root - sx, e.y_root - sy)
        self._win.geometry(f"{self._w}x{self._h}+{nx}+{ny}")
        self._backdrop.geometry(f"{self._w}x{self._h}+{nx}+{ny}")

    def _resize_start(self, e) -> None:
        self._drag = (e.x_root, e.y_root, self._w, self._h)

    def _resize_drag(self, e) -> None:
        sx, sy, ow, oh = self._drag
        nw, nh = resized_to(ow, oh, e.x_root - sx, e.y_root - sy, MIN_WIDTH, MIN_HEIGHT)
        self._w, self._h = nw, nh
        geometry = f"{nw}x{nh}+{self._win.winfo_x()}+{self._win.winfo_y()}"
        self._win.geometry(geometry)
        self._backdrop.geometry(geometry)

    # --- 訊息 ---
    def add_message(self, original: str, translated: str, now: float | None = None,
                    msg_id: int | None = None, pending: bool = False,
                    color: str | None = None) -> None:
        """加入一則訊息。pending＝譯文欄位目前是佔位字樣，以較暗的顏色標示，
        待 update_message 填入真正的譯文時才恢復正常顏色。
        color＝該則在遊戲內的顯示色：譯文直接用它、原文用調暗版；None 退回預設配色。"""
        row = tk.Frame(self._inner, bg=BG)
        _outlined_line(row, original, dimmed(color) if color else FG_ORIGINAL,
                       _FONT_ORIGINAL, self._wrap).pack(fill="x")
        _outlined_line(row, translated,
                       FG_PENDING if pending else (color or FG_TRANSLATED),
                       _FONT_TRANSLATED, self._wrap).pack(fill="x")
        row.pack(side="top", fill="x", pady=2)  # 最新在最下
        self._messages.append(_Message(now if now is not None else time.time(),
                                       original, translated, row, msg_id, color))
        while len(self._messages) > self._max:
            self._messages.pop(0).row.destroy()

        self._refresh_placeholder()
        self._refresh_scroll()
        if self._minimized:
            self._unread += 1
            self._update_badge()

    def update_message(self, msg_id: int, translated: str,
                       failed: bool = False) -> None:
        """把某則佔位訊息的譯文就地填入（原文與位置不動）。
        failed＝這則翻不出來、填入的是失敗提示，改用錯誤色與一般對話區隔。
        找不到 msg_id 代表該則已被 prune 或 max_messages 擠掉，安靜忽略。"""
        for i, m in enumerate(self._messages):
            if m.msg_id != msg_id:
                continue
            line = m.row.winfo_children()[1]  # 0＝原文行，1＝譯文行
            line.itemconfigure("txt", text=translated)
            # 脫離佔位：換回該則的遊戲色；翻譯失敗則一律走錯誤色
            line.itemconfigure("fg", fill=FG_ERROR if failed
                               else (m.color or FG_TRANSLATED))
            _fit_line_height(line)
            self._messages[i] = m._replace(translated=translated)
            self._refresh_scroll()
            return

    def set_limits(self, max_messages: int, fade_seconds: int) -> None:
        """套用新的訊息上限與淡出秒數；超出上限的最舊訊息立即移除。"""
        self._max = max_messages
        self._fade = fade_seconds
        removed = False
        while len(self._messages) > self._max:
            self._messages.pop(0).row.destroy()
            removed = True
        self._refresh_placeholder()
        if removed:
            self._refresh_scroll()

    def prune(self, now: float | None = None) -> None:
        if self._fade <= 0:
            return  # fade_seconds <= 0：永不依時間清除訊息（可滾動看歷史）
        cutoff = (now if now is not None else time.time()) - self._fade
        keep = []
        for entry in self._messages:
            if entry.ts <= cutoff:
                entry.row.destroy()
            else:
                keep.append(entry)
        if len(keep) == len(self._messages):
            return
        self._messages = keep
        self._refresh_placeholder()
        self._refresh_scroll()

    def set_status(self, text: str, color: str = FG_BAR) -> None:
        """更新狀態指示：標題列右側小字；視窗還沒有任何訊息時，同步大字置中顯示。"""
        self._status_label.configure(text=text, fg=color)
        self._placeholder.configure(text=text, fg=color)
        self._refresh_placeholder()

    def _refresh_placeholder(self) -> None:
        """沒有訊息 → 置中顯示狀態；有訊息 → 收掉，讓位給訊息列表。"""
        if self._messages:
            self._placeholder.place_forget()
        else:
            self._placeholder.place(relx=0.5, rely=0.5, anchor="center")
            self._placeholder.lift()

    def set_error(self, text: str) -> None:
        self.clear_error()
        self._error_label = tk.Label(self._frame, text=text, bg=BG, fg=FG_ERROR,
                                     font=("Microsoft JhengHei", 10, "bold"), anchor="w",
                                     wraplength=self._wrap)
        self._error_label.pack(side="bottom", fill="x", pady=2)

    def clear_error(self) -> None:
        if self._error_label is not None:
            self._error_label.destroy()
            self._error_label = None

    # --- 測試/除錯輔助 ---
    def visible_messages(self) -> list[tuple[str, str]]:
        return [(m.original, m.translated) for m in self._messages]

    def error_text(self) -> str | None:
        return self._error_label.cget("text") if self._error_label else None

    def status_text(self) -> str:
        return self._status_label.cget("text")

    def placeholder_visible(self) -> bool:
        return self._placeholder.winfo_manager() == "place"
