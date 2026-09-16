"""滑鼠停留提示：懸停在控件上方一小段時間後，跳出一顆小卡片顯示文字。

獨立成一個模組是因為疊加視窗的標題列按鈕（⛶／⚙／─／✕）與框選卡片的關閉鈕
（region_card.py 的 ✕）都要用到同一種提示，外觀、定位、生命週期完全一樣，
不該各寫一份。外觀與 `popup.py` 的右鍵選單同款（GRIP 描邊、BAR 底色），
但語意不同：Popup 是點擊觸發的選單，Tooltip 是懸停觸發的唯讀提示，不接滑鼠
點擊、也不需要 hover 變色。

不奪焦點（`make_non_activating`，與 Popup、疊加視窗底板同一招）：提示窗一旦被
Windows 啟用，鍵盤事件就會轉移過去，遊戲的鍵盤操作與疊加視窗本體的 Ctrl+C／Esc
都會被打斷 —— 而提示本來就只需要滑鼠停留即可觸發，不該碰鍵盤焦點。
"""
import tkinter as tk

from src.ui.fonts import ui_font
from src.ui.geometry import clamped_position
from src.ui.palette import BAR, FG_BAR, GRIP
from src.ui.winstyle import make_non_activating

_OFFSET = 12    # 提示視窗相對游標的偏移（右下方），避免直接蓋住游標
_PAD_X = 8
_PAD_Y = 4


class Tooltip:
    """綁在單一控件上；`text_fn()` 在每次顯示當下才呼叫，語言或內容（例如熱鍵）
    換了，下一次懸停就會拿到新文字，不必額外接語言切換的刷新流程。"""

    def __init__(self, widget: tk.Widget, text_fn, delay_ms: int = 500):
        self._widget = widget
        self._text_fn = text_fn
        self._delay_ms = delay_ms
        self._win: tk.Toplevel | None = None
        self._label: tk.Label | None = None
        self._after_id: str | None = None
        widget.bind("<Enter>", self._schedule)
        widget.bind("<Leave>", self._cancel)
        widget.bind("<ButtonPress>", self._cancel)

    @property
    def visible(self) -> bool:
        return self._win is not None and bool(self._win.winfo_ismapped())

    def text(self) -> str:
        """目前顯示的文字（測試用）；還沒顯示過就回傳空字串。"""
        return self._label.cget("text") if self._label is not None else ""

    def _schedule(self, _event=None) -> None:
        self._cancel()
        self._after_id = self._widget.after(self._delay_ms, self._show)

    def _cancel(self, _event=None) -> None:
        if self._after_id is not None:
            self._widget.after_cancel(self._after_id)
            self._after_id = None
        self.hide()

    def _build(self) -> None:
        """視窗延遲到第一次顯示才建立：大部分控件的提示永遠不會被看到，
        沒必要跟著控件一起把視窗建好。母視窗用控件所屬的 toplevel —— 控件的
        toplevel（overlay 本體或 region card）被關閉、destroy 時，這顆提示視窗
        身為子視窗會一併被 Tk 收掉，不必另外接管。"""
        win = tk.Toplevel(self._widget.winfo_toplevel())
        win.withdraw()
        win.overrideredirect(True)
        win.attributes("-topmost", True)
        win.configure(bg=GRIP)   # 外層底色當 1px 邊框，與 Popup 同款
        make_non_activating(win)
        label = tk.Label(win, bg=BAR, fg=FG_BAR, font=ui_font(9),
                         padx=_PAD_X, pady=_PAD_Y)
        label.pack(padx=1, pady=1)
        self._win = win
        self._label = label

    def _show(self) -> None:
        self._after_id = None
        try:
            if self._win is None:
                self._build()
            self._label.configure(text=self._text_fn(), font=ui_font(9))
            self._win.update_idletasks()
            x, y = clamped_position(
                self._widget.winfo_pointerx() + _OFFSET,
                self._widget.winfo_pointery() + _OFFSET,
                self._win.winfo_reqwidth(), self._win.winfo_reqheight(),
                self._widget.winfo_screenwidth(), self._widget.winfo_screenheight())
            self._win.geometry(f"+{x}+{y}")
            self._win.deiconify()
            self._win.lift()
        except tk.TclError:
            # 控件或其 toplevel 剛好在延遲期間被收掉（例如卡片被關閉）：安靜放棄這次顯示
            pass

    def hide(self) -> None:
        if self._win is not None:
            try:
                self._win.withdraw()
            except tk.TclError:
                pass
