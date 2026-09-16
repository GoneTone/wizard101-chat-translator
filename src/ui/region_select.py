"""框選用的全螢幕選取層：蓋滿遊戲所在的那顆螢幕，顯示 `RegionFlow` 事先凍結好的那一幀
遊戲畫面（見 `capture_window`），拖曳畫矩形，放開回報螢幕座標。

整層不透明：底圖是暗化過的凍結畫面，不是「半透明看穿桌面」。遊戲視窗以外的區域另外疊一張
暗化過的整顆螢幕截圖（`backdrop`，見 `capture_screen`）墊在遊戲畫面底下，讓桌面其餘部分
還看得見、只是暗一點（跟 Snipping Tool 一樣）；沒有 backdrop（例如螢幕截圖失敗）時退回
純黑，選取層照樣開得起來。拖曳中的框選範圍內用原始亮度的遊戲畫面取代暗化底，做出類似
Snipping Tool 的聚光燈效果，讓使用者看得出框選範圍實際框住了什麼（聚光燈只作用在遊戲畫面
範圍內，backdrop 部分不參與）。點一下沒拖動視為取消（is_click）。
提示文字另開一層不透明視窗疊在選取層之上，跟底圖的明暗變化無關。
"""
import tkinter as tk

from PIL import ImageEnhance, ImageTk

from src.composer.paste import force_foreground
from src.i18n import t
from src.region.capture import Frame
from src.ui.fonts import ui_font
from src.ui.geometry import is_click
from src.ui.palette import BAR, FG_TRANSLATED, FG_UPDATE, GRIP
from src.ui.winstyle import make_non_activating, root_hwnd

_DIM_FACTOR = 0.45
_BAND_WIDTH = 2
_HINT_Y = 40


class RegionSelector:
    """`show(monitor, frame, on_select, on_cancel, backdrop)` 開層；
    使用者放開滑鼠後層先關、再回呼。"""

    def __init__(self, root: tk.Tk):
        self._root = root
        self._win: tk.Toplevel | None = None
        self._hint: tk.Toplevel | None = None
        self._hint_label: tk.Label | None = None
        self._canvas: tk.Canvas | None = None
        self._rubber_band = None
        self._size_label = None
        self._spot = None
        self._dim_photo = None
        self._spot_photo = None
        self._backdrop_photo = None
        self._frame: Frame | None = None
        self._frame_pos = (0, 0)
        self._origin = (0, 0)
        self._start: tuple[int, int] | None = None
        self._on_select = None
        self._on_cancel = None

    @property
    def is_open(self) -> bool:
        return self._win is not None

    def show(self, monitor: tuple[int, int, int, int], frame: Frame, on_select,
             on_cancel=None, backdrop=None) -> None:
        """在 monitor（螢幕矩形 x, y, w, h）上開選取層，顯示 frame 這張已經凍結的遊戲畫面；
        backdrop（同尺寸的整顆螢幕截圖，見 `capture_screen`）給的話墊在遊戲畫面底下暗化
        顯示，沒給就維持純黑。已開著就先關掉重開。"""
        self.cancel(notify=False)
        self._origin = (monitor[0], monitor[1])
        self._on_select, self._on_cancel = on_select, on_cancel
        self._start = None
        self._frame = frame
        win = tk.Toplevel(self._root)
        win.overrideredirect(True)
        win.attributes("-topmost", True)
        win.attributes("-alpha", 1.0)
        win.configure(bg="black", cursor="crosshair")
        win.geometry(f"{monitor[2]}x{monitor[3]}+{monitor[0]}+{monitor[1]}")
        canvas = tk.Canvas(win, bg="black", highlightthickness=0, cursor="crosshair")
        canvas.pack(fill="both", expand=True)
        if backdrop is not None:
            dim_backdrop = ImageEnhance.Brightness(backdrop).enhance(_DIM_FACTOR)
            self._backdrop_photo = ImageTk.PhotoImage(dim_backdrop, master=canvas)
            # 建立順序決定疊放順序：backdrop 先畫，才會墊在遊戲畫面（下面接著建立）底下
            canvas.create_image(0, 0, image=self._backdrop_photo, anchor="nw")
        dim_image = ImageEnhance.Brightness(frame.image).enhance(_DIM_FACTOR)
        self._dim_photo = ImageTk.PhotoImage(dim_image, master=canvas)
        self._frame_pos = (frame.client_origin[0] - monitor[0], frame.client_origin[1] - monitor[1])
        canvas.create_image(*self._frame_pos, image=self._dim_photo, anchor="nw")
        # 建立順序決定疊放順序：聚光燈要蓋過暗化底，但框線與尺寸文字要蓋過聚光燈
        self._spot = canvas.create_image(0, 0, anchor="nw", state="hidden")
        self._rubber_band = canvas.create_rectangle(0, 0, 0, 0, outline=FG_UPDATE,
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
        self._build_hint(monitor)
        # Tk 內部焦點：純 Tcl 層級操作，被系統前景鎖擋下時不會真的搶走 Windows 前景
        # （沒有 force_foreground 那支 AttachThreadInput），但 <Escape> 綁定要收得到
        # 事件非設不可，所以獨立於 _take_focus 之外、恆定執行，測試也不用停用它。
        win.focus_force()
        self._take_focus()
        # 啟用選取層會把它抬到 topmost 帶的最上面，提示視窗得在那之後再抬一次才不會被壓在底下
        self._hint.lift()

    def _build_hint(self, monitor: tuple[int, int, int, int]) -> None:
        """提示文字開在自己的不透明視窗，避免跟選取層的明暗切換互相干擾。"""
        hint = tk.Toplevel(self._root)
        hint.overrideredirect(True)
        hint.attributes("-topmost", True)
        hint.configure(bg=GRIP)   # 外層底色當 1px 邊框，內層 body 靠 padx/pady=1 露出來
        body = tk.Frame(hint, bg=BAR)
        body.pack(padx=1, pady=1)
        label = tk.Label(body, text=t("region.hint"), bg=BAR, fg=FG_TRANSLATED,
                         font=ui_font(13), padx=16, pady=8)
        label.pack()
        make_non_activating(hint)
        hint.update_idletasks()
        x = monitor[0] + (monitor[2] - hint.winfo_reqwidth()) // 2
        y = monitor[1] + _HINT_Y
        hint.geometry(f"+{x}+{y}")
        hint.lift(self._win)
        self._hint, self._hint_label = hint, label

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
        self._hint.destroy()
        self._win = self._canvas = self._rubber_band = self._size_label = None
        self._spot = self._dim_photo = self._spot_photo = self._backdrop_photo = None
        self._frame = None
        self._frame_pos = (0, 0)
        self._hint = self._hint_label = None
        self._start = None

    def _press(self, e) -> None:
        self._start = (e.x, e.y)
        self._canvas.coords(self._rubber_band, e.x, e.y, e.x, e.y)
        self._canvas.itemconfigure(self._rubber_band, state="normal")
        self._canvas.itemconfigure(self._spot, state="hidden")

    def _drag(self, e) -> None:
        if self._start is None:
            return
        x0, y0 = self._start
        self._canvas.coords(self._rubber_band, x0, y0, e.x, e.y)
        self._canvas.coords(self._size_label, min(x0, e.x), min(y0, e.y) - 2)
        self._canvas.itemconfigure(self._size_label, state="normal",
                                   text=f"{abs(e.x - x0)}×{abs(e.y - y0)}")
        self._update_spotlight(x0, y0, e.x, e.y)

    def _update_spotlight(self, x0: int, y0: int, x1: int, y1: int) -> None:
        """框選範圍內用原始亮度畫面取代暗化底，範圍要跟遊戲畫面的可見範圍取交集 ——
        框選拖出遊戲視窗外的部分裁掉，維持暗底（那裡本來就沒有遊戲畫面可以聚光）。"""
        left, top = max(min(x0, x1), self._frame_pos[0]), max(min(y0, y1), self._frame_pos[1])
        right = min(max(x0, x1), self._frame_pos[0] + self._frame.client_size[0])
        bottom = min(max(y0, y1), self._frame_pos[1] + self._frame.client_size[1])
        if right <= left or bottom <= top:
            self._canvas.itemconfigure(self._spot, state="hidden")
            return
        fx, fy = self._frame_pos
        crop_box = (left - fx, top - fy, right - fx, bottom - fy)
        self._spot_photo = ImageTk.PhotoImage(self._frame.image.crop(crop_box),
                                              master=self._canvas)
        self._canvas.itemconfigure(self._spot, image=self._spot_photo, state="normal")
        self._canvas.coords(self._spot, left, top)

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
