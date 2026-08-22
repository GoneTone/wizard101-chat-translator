"""疊加視窗:無邊框、置頂、半透明、固定大小、可拖曳移動與縮放、可滾動。
顯示原文 + 譯文(最新在最下,可向上滾動看歷史)。
捲動定位:在底部時新訊息自動跟到最底;向上捲看歷史時不會被硬拉回底部。
不滑鼠穿透 —— 視窗永遠可互動。"""
import sys
import time
import tkinter as tk

import win32con
import win32gui

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


def moved_to(start_x: int, start_y: int, dx: int, dy: int) -> tuple[int, int]:
    """拖曳位移後的新左上角座標。"""
    return start_x + dx, start_y + dy


def resized_to(start_w: int, start_h: int, dx: int, dy: int,
               min_w: int, min_h: int) -> tuple[int, int]:
    """拖曳縮放後的新寬高(不小於最小值)。"""
    return max(min_w, start_w + dx), max(min_h, start_h + dy)


def should_stick_to_bottom(view_bottom_fraction: float,
                           threshold: float = _STICK_THRESHOLD) -> bool:
    """視圖底緣接近最底時,新訊息應自動跟到底;使用者往上捲時則否。"""
    return view_bottom_fraction >= threshold


class OverlayWindow:
    def __init__(self, root: tk.Tk, x: int | None, y: int | None,
                 width: int = 460, height: int = 300,
                 max_messages: int = 50, fade_seconds: int = 180,
                 on_geometry_change=None, on_settings=None, on_close=None):
        self._max = max_messages
        self._fade = fade_seconds
        self._on_geometry_change = on_geometry_change
        self._messages: list[tuple[float, str, str, tk.Frame]] = []
        self._error_label: tk.Label | None = None
        self._w = max(width, MIN_WIDTH)
        self._h = max(height, MIN_HEIGHT)
        self._wrap = self._w - 40
        self._drag = (0, 0, 0, 0)

        self._win = tk.Toplevel(root)
        self._win.overrideredirect(True)
        self._win.attributes("-topmost", True)
        self._win.attributes("-alpha", 0.88)
        self._win.configure(bg=BG)
        px = x if x is not None else 40
        py = y if y is not None else 40
        self._apply_geometry(px, py, self._w, self._h)

        # 標題列(可拖曳移動)
        bar = tk.Frame(self._win, bg=BAR, height=_BAR_HEIGHT, cursor="fleur")
        bar.pack(side="top", fill="x")
        bar.pack_propagate(False)
        label = tk.Label(bar, text="≡  Wizard101 翻譯", bg=BAR, fg=FG_BAR,
                         font=("Microsoft JhengHei", 8), anchor="w")
        label.pack(side="left", padx=6)
        # side="right" 先 pack 者占最外側:由右到左依序為 ✕、⚙、狀態字。
        # overlay 是無邊框視窗、打包版沒有主控台,✕ 是唯一的正常關閉途徑。
        if on_close is not None:
            close = tk.Label(bar, text="✕", bg=BAR, fg=FG_BAR,
                             font=("Microsoft JhengHei", 9), cursor="hand2")
            close.pack(side="right", padx=(0, 6))
            close.bind("<Button-1>", lambda e: on_close())
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

        # 內容區:錯誤橫幅(固定在下,不隨捲動)+ 可滾動訊息區
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

        # 空狀態提示:沒有任何訊息時,把目前狀態大字顯示在視窗正中間
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

        self._win.title("Wizard101 聊天翻譯")  # 工作列按鈕顯示的名稱
        self._add_taskbar_button()

    def _add_taskbar_button(self) -> None:
        """讓無邊框視窗出現在工作列與 Alt+Tab。
        overrideredirect 視窗預設拿不到工作列按鈕,把 WS_EX_APPWINDOW 加進
        extended style 即可;需 withdraw→deiconify 一次讓樣式生效,
        之後重設 topmost/alpha(重新顯示會掉)。失敗只是少個按鈕,不影響功能。"""
        try:
            self._win.update_idletasks()
            hwnd = win32gui.GetAncestor(self._win.winfo_id(), 2)  # GA_ROOT
            style = win32gui.GetWindowLong(hwnd, win32con.GWL_EXSTYLE)
            style = (style & ~win32con.WS_EX_TOOLWINDOW) | win32con.WS_EX_APPWINDOW
            win32gui.SetWindowLong(hwnd, win32con.GWL_EXSTYLE, style)
            self._win.withdraw()
            self._win.deiconify()
            self._win.attributes("-topmost", True)
            self._win.attributes("-alpha", 0.88)
        except Exception as exc:
            print(f"[ui] taskbar button setup failed: {exc}", file=sys.stderr)

    # --- 幾何 ---
    def _apply_geometry(self, x: int, y: int, w: int, h: int) -> None:
        self._w, self._h = w, h
        self._wrap = w - 40
        self._win.geometry(f"{w}x{h}+{x}+{y}")

    def _emit_geometry(self) -> None:
        if self._on_geometry_change is None:
            return
        self._win.update_idletasks()
        self._on_geometry_change(self._win.winfo_x(), self._win.winfo_y(),
                                 self._win.winfo_width(), self._win.winfo_height())

    def _on_canvas_configure(self, e) -> None:
        # 內層寬度跟著畫布寬,文字才會依視窗寬換行
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

    def _resize_start(self, e) -> None:
        self._drag = (e.x_root, e.y_root, self._w, self._h)

    def _resize_drag(self, e) -> None:
        sx, sy, ow, oh = self._drag
        nw, nh = resized_to(ow, oh, e.x_root - sx, e.y_root - sy, MIN_WIDTH, MIN_HEIGHT)
        self._w, self._h = nw, nh
        self._win.geometry(f"{nw}x{nh}+{self._win.winfo_x()}+{self._win.winfo_y()}")

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
            return  # fade_seconds <= 0:永不依時間清除訊息(可滾動看歷史)
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
        """更新狀態指示:標題列右側小字;視窗還沒有任何訊息時,同步大字置中顯示。"""
        self._status_label.configure(text=text, fg=color)
        self._placeholder.configure(text=text, fg=color)
        self._refresh_placeholder()

    def _refresh_placeholder(self) -> None:
        """沒有訊息 → 置中顯示狀態;有訊息 → 收掉,讓位給訊息列表。"""
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
