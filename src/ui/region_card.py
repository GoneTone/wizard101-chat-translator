"""框選翻譯的結果卡片：貼在框選矩形正下方、與它同寬；辨識中／譯文／錯誤三態。

不奪焦點（與彈出選單同一招）：遊戲的鍵盤操作不中斷，代價是收不到 Esc，
關閉方式是點卡片任一處、或下一次框選時被換掉。位置每次重排：譯文回來後高度變了，
下方放不下要翻到矩形上方。標頭右上角另放一個 ✕、內文下方帶一行提示文字，
點擊關閉本來就存在，只是沒人知道，這兩處純粹是把既有行為講出來。
譯文回來時若帶原文（看圖路徑的逐字抄寫、OCR 路徑的辨識文字），在譯文上方另用一行
暗色小字顯示 —— 與聊天疊加視窗「原文在上、譯文在下」一致，也讓使用者能核對模型
有沒有多翻或漏翻。
"""
import tkinter as tk

from src.i18n import t
from src.log import log
from src.ui.fonts import ui_font
from src.ui.geometry import anchored_position
from src.ui.monitors import work_area_at
from src.ui.palette import (
    BG,
    FG_BAR,
    FG_ERROR,
    FG_ORIGINAL,
    FG_PENDING,
    FG_TRANSLATED,
    FG_UPDATE,
    GRIP,
)
from src.ui.richtext import RichLabel
from src.ui.winstyle import make_non_activating

MIN_WIDTH = 240   # 矩形再窄也不跟：譯文會擠成一長條
ANCHOR_GAP = 4    # 與框選矩形的垂直間距（px）
_PAD_X = 10
_PAD_Y = 6


class RegionCard:
    """一次只有一張；`show_pending(rect)` 建窗並佔位，`show_text`／`show_error` 換內容。"""

    def __init__(self, root: tk.Tk, alpha: float):
        self._root = root
        self._alpha = alpha
        self._win: tk.Toplevel | None = None
        self._label: RichLabel | None = None
        self._source: RichLabel | None = None
        self._close: tk.Label | None = None
        self._hint: tk.Label | None = None
        self._rect: tuple[int, int, int, int] | None = None
        self._width = MIN_WIDTH

    @property
    def is_open(self) -> bool:
        return self._win is not None

    def set_alpha(self, alpha: float) -> None:
        """設定視窗調不透明度時跟著疊加視窗一起變。"""
        self._alpha = alpha
        if self._win is not None:
            self._win.attributes("-alpha", alpha)

    def show_pending(self, rect: tuple[int, int, int, int]) -> None:
        """在 rect（螢幕座標）下方開一張「翻譯中…」的卡片；已開著就換位置重來。"""
        self.hide()
        self._rect = rect
        area = work_area_at(rect[0], rect[1])
        self._width = max(MIN_WIDTH, min(rect[2], area[2]))
        win = tk.Toplevel(self._root)
        win.withdraw()
        win.overrideredirect(True)
        win.attributes("-topmost", True)
        win.attributes("-alpha", self._alpha)
        win.configure(bg=GRIP)   # 外層底色當 1px 邊框
        body = tk.Frame(win, bg=BG, cursor="hand2")
        body.pack(fill="both", expand=True, padx=1, pady=1)
        header = tk.Frame(body, bg=BG)
        header.pack(fill="x")
        self._close = tk.Label(header, text="✕", bg=BG, fg=FG_BAR, font=ui_font(10),
                               cursor="hand2", padx=6)
        self._close.pack(side="right")
        self._close.bind("<Button-1>", lambda e: self.hide())
        # 原文列先建好但不 pack：預設隱藏，show_text 帶 source 時才插進 header 與譯文之間
        self._source = RichLabel(body, fg=FG_ORIGINAL, bg=BG, font=ui_font(9),
                                 on_height_change=self._layout)
        self._label = RichLabel(body, fg=FG_PENDING, bg=BG, font=ui_font(11),
                                link_fg=FG_UPDATE, on_height_change=self._layout)
        self._label.pack(fill="x", padx=_PAD_X, pady=_PAD_Y)
        self._hint = tk.Label(body, text=t("region.close_hint"), bg=BG, fg=FG_PENDING,
                              font=ui_font(8), anchor="w")
        self._hint.pack(fill="x", padx=_PAD_X, pady=(0, _PAD_Y))
        for widget in (win, body, self._label, self._source):
            widget.bind("<Button-1>", lambda e: self.hide(), add="+")
        self._win = win
        win.geometry(f"{self._width}x1")   # 寬度先定，RichLabel 才能依它換行；高度由 _layout 量
        make_non_activating(win)
        self._label.set(t("notice.pending"), FG_PENDING)
        self._layout()
        win.deiconify()

    def show_text(self, text: str, source: str = "") -> None:
        """譯文回來了；空字串＝畫面上沒有文字，用暗色提示。
        `source` 非空時在譯文上方插入一行暗色小字顯示原文（見檔頭）；空字串時原文列
        收起（`pack_forget`），不佔版面。"""
        if self._label is None or self._source is None:
            return
        if source:
            self._source.set(source, FG_ORIGINAL)
            self._source.pack(fill="x", padx=_PAD_X, pady=(_PAD_Y, 0), before=self._label)
        else:
            self._source.set("")
            self._source.pack_forget()
        if text:
            self._label.set(text, FG_TRANSLATED)
        else:
            self._label.set(t("region.no_text"), FG_PENDING)
        self._layout()

    def show_error(self, message: str) -> None:
        if self._label is None:
            return
        self._label.set(message, FG_ERROR)
        self._layout()

    def hide(self) -> None:
        if self._win is not None:
            self._win.destroy()
            self._win = None
            self._label = None
            self._source = None
            self._close = None
            self._hint = None

    def text(self) -> str:
        """目前顯示的譯文（測試用），不含原文列。"""
        if self._label is None:
            return ""
        return self._label.get("1.0", "end-1c")

    def source_text(self) -> str:
        """目前顯示的原文（測試用）；沒有原文（列被收起）時回傳空字串。"""
        if self._source is None:
            return ""
        return self._source.get("1.0", "end-1c")

    def _layout(self) -> None:
        """依內容高度重新定位：貼在矩形下方、放不下翻到上方、夾在工作區內。"""
        if self._win is None or self._rect is None:
            return
        self._win.update_idletasks()
        height = self._win.winfo_reqheight()
        area = work_area_at(self._rect[0], self._rect[1])
        x, y = anchored_position(self._rect, self._width, height, area, gap=ANCHOR_GAP)
        self._win.geometry(f"{self._width}x{height}+{x}+{y}")
        log(f"[region] card placed (rect={self._rect}, size={self._width}x{height}, "
            f"at=({x}, {y}))")
