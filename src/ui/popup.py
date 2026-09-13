"""右鍵彈出選單：自繪的小視窗，配色與疊加視窗的自製標題列、捲軸一致。

不用 `tk.Menu`：它在 Windows 會被畫上一圈系統的淺色外框，`borderwidth`／`relief`
都拔不掉（實測截圖確認），擺在深色疊加視窗旁邊像是外來的系統選單。代價是關閉與
螢幕邊緣夾位得自己處理 —— 關閉沿用 overlay 既有的滑鼠路由（點到疊加視窗任何一處都
會呼叫 `hide()`），不另外抓 grab。
"""
import tkinter as tk

from src.log import log
from src.ui.fonts import ui_font
from src.ui.geometry import clamped_position
from src.ui.palette import BAR, FG_BAR, FG_TRANSLATED, GRIP
from src.ui.winstyle import make_non_activating

_PAD_X = 10
_PAD_Y = 5
_GAP = 6          # 圖示與文字的間距
_ICON = 13        # 圖示邊長，配合 ui_font(9) 的字高
_SHEET_OFFSET = 4  # 兩張紙錯開的距離


class Popup:
    """單一項目的彈出選單。建一次、重複使用（每次 `show` 才帶入當下語言的文字）。"""

    def __init__(self, master: tk.Misc, on_click):
        self._on_click = on_click
        self._win = tk.Toplevel(master)
        self._win.withdraw()
        self._win.overrideredirect(True)
        self._win.attributes("-topmost", True)
        # 外層底色當作 1px 邊框，內層 row 靠 padx/pady=1 讓它露出來
        self._win.configure(bg=GRIP)
        # 絕不能被啟用成作用中視窗：overrideredirect 視窗一 deiconify 就會被 Windows
        # 啟用，鍵盤事件從此落到這裡，疊加視窗本體的 Ctrl+C 與 Esc 全部收不到（實機
        # 踩過），而且還會把焦點從遊戲搶走。滑鼠事件不受影響 —— backdrop 用的是同一招。
        make_non_activating(self._win)

        self._row = tk.Frame(self._win, bg=BAR, cursor="hand2")
        self._row.pack(padx=1, pady=1)
        self._icon = tk.Canvas(self._row, width=_ICON, height=_ICON, bg=BAR,
                               highlightthickness=0, bd=0)
        # 「兩張疊起來的紙」：後面那張只露出左上角。用 canvas 而非 PhotoImage —— 底色能
        # 跟著 hover 換，也避開 PhotoImage 在別的執行緒被 GC 時對 Tk 呼叫的那個坑
        # （見 bubble.destroy）。
        self._icon.create_rectangle(0, 0, _ICON - _SHEET_OFFSET - 1, _ICON - _SHEET_OFFSET - 1,
                                    outline=FG_BAR, tags="ink")
        self._icon.create_rectangle(_SHEET_OFFSET, _SHEET_OFFSET, _ICON - 1, _ICON - 1,
                                    fill=BAR, outline=FG_BAR, tags=("ink", "front"))
        self._icon.pack(side="left", padx=(_PAD_X, 0), pady=_PAD_Y)
        self._label = tk.Label(self._row, bg=BAR, fg=FG_BAR, font=ui_font(9))
        self._label.pack(side="left", padx=(_GAP, _PAD_X), pady=_PAD_Y)

        for widget in (self._row, self._icon, self._label):
            widget.bind("<Enter>", lambda e: self._hover(True))
            widget.bind("<Leave>", lambda e: self._hover(False))
            widget.bind("<Button-1>", self._clicked)
        # Esc 不綁在這裡：本視窗從不取得鍵盤焦點（取走的話疊加視窗的 Ctrl+C 就斷了），
        # 焦點在疊加視窗本體上，所以由 overlay 綁 Esc 再呼叫 hide()。

    @property
    def visible(self) -> bool:
        return bool(self._win.winfo_ismapped())

    def label_text(self) -> str:
        """目前顯示的項目文字（測試／除錯用）。"""
        return self._label.cget("text")

    def show(self, x_root: int, y_root: int, label: str) -> None:
        """在指定的螢幕座標彈出。`label` 每次傳入，介面語言換了就跟著換。"""
        self._label.configure(text=label, font=ui_font(9))
        self._hover(False)
        self._win.update_idletasks()
        x, y = clamped_position(x_root, y_root,
                                self._win.winfo_reqwidth(), self._win.winfo_reqheight(),
                                self._win.winfo_screenwidth(),
                                self._win.winfo_screenheight())
        self._win.geometry(f"+{x}+{y}")
        self._win.deiconify()
        self._win.lift()
        log(f"[ui] copy popup shown at {x},{y}")

    def hide(self) -> None:
        if not self.visible:
            return
        self._win.withdraw()
        log("[ui] copy popup hidden")

    def _hover(self, on: bool) -> None:
        bg, fg = (GRIP, FG_TRANSLATED) if on else (BAR, FG_BAR)
        self._row.configure(bg=bg)
        self._icon.configure(bg=bg)
        self._icon.itemconfigure("ink", outline=fg)
        self._icon.itemconfigure("front", fill=bg)   # 前面那張要蓋住後面那張的線
        self._label.configure(bg=bg, fg=fg)

    def _clicked(self, _event) -> None:
        self.hide()
        self._on_click()
