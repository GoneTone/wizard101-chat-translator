"""框選用的全螢幕選取層：蓋滿遊戲所在的那顆螢幕，拖曳畫矩形，放開回報螢幕座標。

暗色半透明底讓遊戲畫面仍看得見。這層需要鍵盤（Esc）與滑鼠，所以會奪焦點；
關閉時由呼叫端把前景還給遊戲（RegionFlow）。點一下沒拖動視為取消（is_click）。
"""
import tkinter as tk

from src.composer.paste import force_foreground
from src.i18n import t
from src.ui.fonts import ui_font
from src.ui.geometry import is_click
from src.ui.palette import FG_BAR, FG_UPDATE
from src.ui.winstyle import root_hwnd

_TINT_ALPHA = 0.35
_BAND_WIDTH = 2
_HINT_Y = 40


class RegionSelector:
    """`show(monitor, on_select, on_cancel)` 開層；使用者放開滑鼠後層先關、再回呼。"""

    def __init__(self, root: tk.Tk):
        self._root = root
        self._win: tk.Toplevel | None = None
        self._canvas: tk.Canvas | None = None
        self._band = None
        self._size_label = None
        self._origin = (0, 0)
        self._start: tuple[int, int] | None = None
        self._on_select = None
        self._on_cancel = None

    @property
    def is_open(self) -> bool:
        return self._win is not None

    def show(self, monitor: tuple[int, int, int, int], on_select, on_cancel=None) -> None:
        """在 monitor（螢幕矩形 x, y, w, h）上開選取層；已開著就先關掉重開。"""
        self.cancel(notify=False)
        self._origin = (monitor[0], monitor[1])
        self._on_select, self._on_cancel = on_select, on_cancel
        self._start = None
        win = tk.Toplevel(self._root)
        win.overrideredirect(True)
        win.attributes("-topmost", True)
        win.attributes("-alpha", _TINT_ALPHA)
        win.configure(bg="black", cursor="crosshair")
        win.geometry(f"{monitor[2]}x{monitor[3]}+{monitor[0]}+{monitor[1]}")
        canvas = tk.Canvas(win, bg="black", highlightthickness=0, cursor="crosshair")
        canvas.pack(fill="both", expand=True)
        canvas.create_text(monitor[2] // 2, _HINT_Y, text=t("region.hint"),
                           fill=FG_BAR, font=ui_font(12))
        self._band = canvas.create_rectangle(0, 0, 0, 0, outline=FG_UPDATE,
                                             width=_BAND_WIDTH, state="hidden")
        self._size_label = canvas.create_text(0, 0, text="", fill=FG_UPDATE,
                                              font=ui_font(9), anchor="sw", state="hidden")
        canvas.bind("<ButtonPress-1>", self._press)
        canvas.bind("<B1-Motion>", self._drag)
        canvas.bind("<ButtonRelease-1>", self._release)
        win.bind("<Escape>", lambda e: self.cancel())
        canvas.bind("<Button-3>", lambda e: self.cancel())
        self._win, self._canvas = win, canvas
        win.update_idletasks()
        # Tk 內部焦點：純 Tcl 層級操作，被系統前景鎖擋下時不會真的搶走 Windows 前景
        # （沒有 force_foreground 那支 AttachThreadInput），但 <Escape> 綁定要收得到
        # 事件非設不可，所以獨立於 _take_focus 之外、恆定執行，測試也不用停用它。
        win.focus_force()
        self._take_focus()

    def _take_focus(self) -> None:
        """把 Windows 前景切給選取層；Esc 要收得到，從遊戲熱鍵開層時遊戲仍是前景。
        會用 AttachThreadInput 硬切前景，測試環境須停用，避免搶走使用者當下操作中的視窗。"""
        force_foreground(root_hwnd(self._win))

    def cancel(self, notify: bool = True) -> None:
        """關掉選取層；`notify=True` 時呼叫 on_cancel。"""
        if self._win is None:
            return
        self._destroy()
        if notify and self._on_cancel is not None:
            self._on_cancel()

    def _destroy(self) -> None:
        self._win.destroy()
        self._win = self._canvas = self._band = self._size_label = None
        self._start = None

    def _press(self, e) -> None:
        self._start = (e.x, e.y)
        self._canvas.coords(self._band, e.x, e.y, e.x, e.y)
        self._canvas.itemconfigure(self._band, state="normal")

    def _drag(self, e) -> None:
        if self._start is None:
            return
        x0, y0 = self._start
        self._canvas.coords(self._band, x0, y0, e.x, e.y)
        self._canvas.coords(self._size_label, min(x0, e.x), min(y0, e.y) - 2)
        self._canvas.itemconfigure(self._size_label, state="normal",
                                   text=f"{abs(e.x - x0)}×{abs(e.y - y0)}")

    def _release(self, e) -> None:
        if self._start is None:
            return
        x0, y0 = self._start
        if is_click(e.x - x0, e.y - y0):
            self.cancel()
            return
        left, top = min(x0, e.x), min(y0, e.y)
        rect = (self._origin[0] + left, self._origin[1] + top,
                abs(e.x - x0), abs(e.y - y0))
        on_select = self._on_select
        self._destroy()
        on_select(rect)
