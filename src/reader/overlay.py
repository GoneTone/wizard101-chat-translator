"""疊加視窗：無邊框、置頂、半透明、固定大小、可拖曳移動與縮放、可滾動。
顯示原文 + 譯文（最新在最下，可向上滾動看歷史）。
捲動定位：在底部時新訊息自動跟到最底；向上捲看歷史時不會被硬拉回底部。
不滑鼠穿透 —— 視窗蓋住的區域點擊不會傳到遊戲，視窗永遠可互動。"""
import sys
import time
import tkinter as tk

import win32con
import win32gui

from src.config import APP_NAME

BG = "#101018"
BAR = "#23233a"
GRIP = "#3a3a55"
FG_ORIGINAL = "#9a9aa8"
FG_TRANSLATED = "#f2f2f7"
FG_ERROR = "#ff5f5f"
FG_BAR = "#c8c8d8"

MIN_WIDTH = 200
MIN_HEIGHT = 90
_BAR_HEIGHT = 20
_GRIP_SIZE = 16
_STICK_THRESHOLD = 0.999
_BUBBLE_SIZE = 64
_CLICK_THRESHOLD = 5
_TRANSPARENT = "#010101"  # 泡泡視窗的透明色鍵（方形視窗只露出圓形）


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
                 width: int = 460, height: int = 300,
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
        self._messages: list[tuple[float, str, str, tk.Frame]] = []
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
        px = x if x is not None else 40
        py = y if y is not None else 40
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
        self._scrollbar = tk.Scrollbar(scroll_area, orient="vertical",
                                       command=self._canvas.yview)
        self._canvas.configure(yscrollcommand=self._scrollbar.set)
        self._scrollbar.pack(side="right", fill="y")
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

        # 右下角縮放把手
        grip = tk.Frame(self._win, bg=GRIP, width=_GRIP_SIZE, height=_GRIP_SIZE,
                        cursor="size_nw_se")
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

    def expand(self) -> None:
        """從泡泡展開回完整視窗，未讀歸零。"""
        if not self._minimized:
            return
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

    def _bubble_press(self, e) -> None:
        self._bubble_drag_state = (e.x_root, e.y_root,
                                   self._bubble.winfo_x(), self._bubble.winfo_y())

    def _bubble_drag(self, e) -> None:
        sx, sy, ox, oy = self._bubble_drag_state
        nx, ny = moved_to(ox, oy, e.x_root - sx, e.y_root - sy)
        self._bubble.geometry(f"+{nx}+{ny}")

    def _bubble_release(self, e) -> None:
        sx, sy, _, _ = self._bubble_drag_state
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

    def _on_wheel(self, e) -> None:
        self._canvas.yview_scroll(int(-e.delta / 120), "units")

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
    def add_message(self, original: str, translated: str, now: float | None = None) -> None:
        stick = should_stick_to_bottom(self._canvas.yview()[1])

        row = tk.Frame(self._inner, bg=BG)
        tk.Label(row, text=original, bg=BG, fg=FG_ORIGINAL,
                 font=("Microsoft JhengHei", 9), anchor="w", justify="left",
                 wraplength=self._wrap).pack(fill="x")
        tk.Label(row, text=translated, bg=BG, fg=FG_TRANSLATED,
                 font=("Microsoft JhengHei", 11), anchor="w", justify="left",
                 wraplength=self._wrap).pack(fill="x")
        row.pack(side="top", fill="x", pady=2)  # 最新在最下
        self._messages.append((now if now is not None else time.time(), original, translated, row))
        while len(self._messages) > self._max:
            _, _, _, old_row = self._messages.pop(0)
            old_row.destroy()

        self._refresh_placeholder()
        self._canvas.update_idletasks()
        self._canvas.configure(scrollregion=self._canvas.bbox("all"))
        if stick:
            self._canvas.yview_moveto(1.0)
        if self._minimized:
            self._unread += 1
            self._update_badge()

    def set_limits(self, max_messages: int, fade_seconds: int) -> None:
        """套用新的訊息上限與淡出秒數；超出上限的最舊訊息立即移除。"""
        self._max = max_messages
        self._fade = fade_seconds
        while len(self._messages) > self._max:
            _, _, _, old_row = self._messages.pop(0)
            old_row.destroy()
        self._refresh_placeholder()

    def prune(self, now: float | None = None) -> None:
        if self._fade <= 0:
            return  # fade_seconds <= 0：永不依時間清除訊息（可滾動看歷史）
        cutoff = (now if now is not None else time.time()) - self._fade
        keep = []
        for entry in self._messages:
            if entry[0] <= cutoff:
                entry[3].destroy()
            else:
                keep.append(entry)
        self._messages = keep
        self._refresh_placeholder()

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
        return [(orig, trans) for _, orig, trans, _ in self._messages]

    def error_text(self) -> str | None:
        return self._error_label.cget("text") if self._error_label else None

    def status_text(self) -> str:
        return self._status_label.cget("text")

    def placeholder_visible(self) -> bool:
        return self._placeholder.winfo_manager() == "place"
