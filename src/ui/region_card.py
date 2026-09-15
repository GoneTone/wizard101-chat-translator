"""框選翻譯的結果卡片：貼在框選矩形正下方、與它同寬；辨識中／譯文／錯誤三態。

不奪焦點（與彈出選單同一招）：遊戲的鍵盤操作不中斷，代價是收不到 Esc，
關閉方式是點卡片任一處、或下一次框選時被換掉。位置每次重排：譯文回來後高度變了，
下方放不下要翻到矩形上方。標頭右上角另放一個 ✕、內文下方帶一行提示文字，
點擊關閉本來就存在，只是沒人知道，這兩處純粹是把既有行為講出來。
譯文回來時若帶原文（本機 OCR 辨識出的文字），在譯文上方另用一段
暗色小字顯示 —— 與聊天疊加視窗「原文在上、譯文在下」一致，也讓使用者能核對模型
有沒有多翻或漏翻。原文與譯文放在同一顆 `RichLabel`（`richtext.RichLabel.set_blocks`）
裡、中間空一行分隔，不再是兩顆各自獨立的 Text —— 拖曳選取才能一路跨過兩段文字，
不會卡在原文與譯文的交界。
內文可拖曳選取（沿用 Tk 對唯讀 Text 的原生選取，不必解除 `state="disabled"`）；
純點擊（按下到放開沒有明顯位移，見 `geometry.is_click`）才關卡片，拖曳不關。
拖曳選取結束後會比照疊加視窗跟 backdrop 借鍵盤焦點（`_focus_for_copy`），讓 Ctrl+C
收得到；右鍵會彈出單項的複製選單（與疊加視窗共用 `Popup`）——有選取就複製選取範圍，
沒有就複製整顆內容（原文＋空行＋譯文，或單純譯文）。
"""
import tkinter as tk

from src.i18n import t
from src.log import log
from src.ui.fonts import ui_font
from src.ui.geometry import anchored_position, is_click
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
    SELECT_BG,
)
from src.ui.popup import Popup
from src.ui.richtext import RichLabel
from src.ui.tooltip import Tooltip
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
        self._close: tk.Label | None = None
        self._close_tooltip: Tooltip | None = None
        self._hint: tk.Label | None = None
        self._popup: Popup | None = None
        self._shown_text = ""     # text() 用：目前顯示的譯文（或提示文字），widget 內容已合併原文不能反推
        self._shown_source = ""   # source_text() 用：目前顯示的原文；沒有原文就是空字串
        self._rect: tuple[int, int, int, int] | None = None
        self._width = MIN_WIDTH
        self._press_pos: tuple[int, int] | None = None   # 按下時的螢幕座標，放開時判斷是點擊還是拖曳

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
        self._close_tooltip = Tooltip(self._close, lambda: t("tooltip.close"))
        self._label = RichLabel(body, fg=FG_PENDING, bg=BG, font=ui_font(11),
                                link_fg=FG_UPDATE, on_height_change=self._layout)
        self._label.pack(fill="x", padx=_PAD_X, pady=_PAD_Y)
        # inactiveselectbackground 在 Windows 預設為空：文字欄沒有鍵盤焦點時選取不會畫出來，
        # 拖曳中與放開後看起來都像沒選到；設成同一個顏色，反白才留得住
        self._label.configure(selectbackground=SELECT_BG, selectforeground=FG_TRANSLATED,
                              inactiveselectbackground=SELECT_BG)
        self._hint = tk.Label(body, text=t("region.close_hint"), bg=BG, fg=FG_PENDING,
                              font=ui_font(8), anchor="w")
        self._hint.pack(fill="x", padx=_PAD_X, pady=(0, _PAD_Y))
        for widget in (win, body, self._label):
            widget.bind("<ButtonPress-1>", self._button_press, add="+")
            widget.bind("<ButtonRelease-1>", self._button_release, add="+")
        for widget in (win, self._label):
            widget.bind("<Control-c>", self.copy_selection, add="+")
            widget.bind("<Control-C>", self.copy_selection, add="+")
        self._win = win
        self._popup = Popup(win, self._copy)
        self._label.bind("<Button-3>", self._right_click)
        win.geometry(f"{self._width}x1")   # 寬度先定，RichLabel 才能依它換行；高度由 _layout 量
        make_non_activating(win)
        # 單一等待狀態：辨識與翻譯兩步不另外分段顯示
        self._shown_text = t("region.working")
        self._shown_source = ""
        self._label.set(self._shown_text, FG_PENDING)
        self._layout()
        win.deiconify()

    def show_text(self, text: str, source: str = "") -> None:
        """譯文回來了；空字串＝畫面上沒有文字，用暗色提示。
        `source` 非空時在譯文上方另放一段暗色小字顯示原文、中間空一行（見檔頭的
        `set_blocks`）；空字串則跟過去一樣只放譯文一段。"""
        if self._label is None:
            return
        self._shown_source = source
        if text:
            self._shown_text = text
            color = FG_TRANSLATED
        else:
            self._shown_text = t("region.no_text")
            color = FG_PENDING
        if source:
            self._label.set_blocks([(source, FG_ORIGINAL, ui_font(9)),
                                    (self._shown_text, color, ui_font(11))])
            self._label.configure(fg=FG_TRANSLATED)   # 沒有落在任何區塊 tag 的殘餘字色也對齊譯文
        else:
            self._label.set(self._shown_text, color)
        self._layout()

    def show_error(self, message: str) -> None:
        if self._label is None:
            return
        self._shown_text = message
        self._label.set(message, FG_ERROR)
        self._layout()

    def hide(self) -> None:
        if self._win is not None:
            if self._popup is not None:
                self._popup.hide()
            # _close_tooltip 的視窗是 self._win 的子視窗（見 Tooltip._build），
            # destroy 這裡會一併收掉，不必另外呼叫 hide()
            self._win.destroy()
            self._win = None
            self._label = None
            self._close = None
            self._close_tooltip = None
            self._hint = None
            self._popup = None
            self._shown_text = ""
            self._shown_source = ""

    def text(self) -> str:
        """目前顯示的譯文（測試用）；原文與譯文現在同放一顆 Text，不能再從 widget
        反推，改記錄 `show_text`／`show_pending`／`show_error` 實際放的字串。"""
        return self._shown_text

    def source_text(self) -> str:
        """目前顯示的原文（測試用）；沒有原文時回傳空字串。"""
        return self._shown_source

    def _button_press(self, event: tk.Event) -> None:
        """記下按下時的螢幕座標，供放開時判斷是點擊還是拖曳。"""
        self._press_pos = (event.x_root, event.y_root)

    def _button_release(self, event: tk.Event) -> None:
        """放開滑鼠：位移在門檻內＝點擊，關卡片；否則是拖曳選字，改跟 backdrop
        借鍵盤焦點，讓 Ctrl+C 收得到（不關卡片，選出來的字才留得住）。"""
        if self._press_pos is None:
            return
        dx = event.x_root - self._press_pos[0]
        dy = event.y_root - self._press_pos[1]
        self._press_pos = None
        if is_click(dx, dy):
            self.hide()
        else:
            self._focus_for_copy()

    def _focus_for_copy(self) -> None:
        """把鍵盤焦點交給卡片本體，Ctrl+C 才收得到 —— 卡片跟疊加視窗一樣用
        `make_non_activating` 不奪焦點，這裡是唯一例外（見 overlay 的同名函式）；
        再把焦點轉到內容欄位本身，`inactiveselectbackground` 才不用扛起顯示反白的
        全部責任。"""
        try:
            log("[region] forcing keyboard focus")
            self._win.focus_force()
            if self._label is not None:
                self._label.focus_set()
            log("[region] selection took keyboard focus")
        except tk.TclError as exc:
            log(f"[region] selection focus failed: {exc}")

    def _selected_text(self) -> str:
        """目前的選取文字（原文與譯文同一顆 Text，選取可以跨兩段）；沒有選取就回傳
        空字串。"""
        if self._label is not None and self._label.tag_ranges("sel"):
            return self._label.get("sel.first", "sel.last")
        return ""

    def copy_selection(self, _event: tk.Event | None = None) -> None:
        """把目前選取的文字寫進系統剪貼簿；沒有選取就什麼都不做，
        免得把使用者原本的剪貼簿內容清掉。"""
        text = self._selected_text()
        if not text or self._win is None:
            return
        self._win.clipboard_clear()
        self._win.clipboard_append(text)
        self._win.update()   # Windows 下要 flush 過，內容才真的落進系統剪貼簿
        log(f"[region] copied selection ({len(text)} chars)")

    def _right_click(self, event: tk.Event) -> None:
        """右鍵點卡片內容：彈出複製選單。"""
        if self._popup is not None:
            self._popup.show(event.x_root, event.y_root, t("menu.copy"))

    def _copy(self) -> None:
        """選單「複製」被點：有選取就複製選取範圍，沒有就複製整顆 Text 的內容
        （原文＋空行＋譯文，或單純譯文）；都沒有文字就只收起選單。"""
        if self._label is None:
            return
        selected = self._selected_text()
        text = selected or self._label.get("1.0", "end-1c")
        if text and self._win is not None:
            self._win.clipboard_clear()
            self._win.clipboard_append(text)
            self._win.update()   # Windows 下要 flush 過，內容才真的落進系統剪貼簿
            kind = "selection" if selected else "text"
            log(f"[region] copied {kind} ({len(text)} chars)")
        if self._popup is not None:
            self._popup.hide()

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
