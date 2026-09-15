"""框選翻譯的結果卡片：貼在框選矩形正下方、與它同寬；辨識中／譯文／錯誤三態。

不奪焦點（與彈出選單同一招）：遊戲的鍵盤操作不中斷，代價是收不到 Esc，
關閉方式是點卡片任一處、或下一次框選時被換掉。位置每次重排：譯文回來後高度變了，
下方放不下要翻到矩形上方。標頭右上角另放一個 ✕、內文下方帶一行提示文字，
點擊關閉本來就存在，只是沒人知道，這兩處純粹是把既有行為講出來。
譯文回來時若帶原文（看圖路徑的逐字抄寫、OCR 路徑的辨識文字），在譯文上方另用一行
暗色小字顯示 —— 與聊天疊加視窗「原文在上、譯文在下」一致，也讓使用者能核對模型
有沒有多翻或漏翻。
譯文與原文列都是可拖曳選取的 `tk.Text`（見 `richtext.RichLabel`）：在文字上拖曳滑鼠
＝反白選取（沿用 Tk 對唯讀 Text 的原生選取，不必解除 `state="disabled"`），選取不能
跨兩列；純點擊（按下到放開沒有明顯位移，見 `geometry.is_click`）才關卡片，拖曳不關。
拖曳選取結束後會比照疊加視窗跟 backdrop 借鍵盤焦點（`_focus_for_copy`），讓 Ctrl+C
收得到；右鍵點譯文或原文列會彈出單項的複製選單（與疊加視窗共用 `Popup`）——
有選取就複製選取範圍，沒有就複製整行。
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
        self._popup: Popup | None = None
        self._copy_target = "text"   # 右鍵點的是哪一行："text"＝譯文、"source"＝原文
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
        # 原文列先建好但不 pack：預設隱藏，show_text 帶 source 時才插進 header 與譯文之間
        self._source = RichLabel(body, fg=FG_ORIGINAL, bg=BG, font=ui_font(9),
                                 on_height_change=self._layout)
        self._label = RichLabel(body, fg=FG_PENDING, bg=BG, font=ui_font(11),
                                link_fg=FG_UPDATE, on_height_change=self._layout)
        self._label.pack(fill="x", padx=_PAD_X, pady=_PAD_Y)
        for label in (self._source, self._label):
            # inactiveselectbackground 在 Windows 預設為空：文字欄沒有鍵盤焦點時選取不會畫出來，
            # 拖曳中與放開後看起來都像沒選到；設成同一個顏色，反白才留得住
            label.configure(selectbackground=SELECT_BG, selectforeground=FG_TRANSLATED,
                            inactiveselectbackground=SELECT_BG)
        self._hint = tk.Label(body, text=t("region.close_hint"), bg=BG, fg=FG_PENDING,
                              font=ui_font(8), anchor="w")
        self._hint.pack(fill="x", padx=_PAD_X, pady=(0, _PAD_Y))
        for widget in (win, body, self._label, self._source):
            widget.bind("<ButtonPress-1>", self._button_press, add="+")
            widget.bind("<ButtonRelease-1>", self._button_release, add="+")
        for widget in (win, self._label, self._source):
            widget.bind("<Control-c>", self.copy_selection, add="+")
            widget.bind("<Control-C>", self.copy_selection, add="+")
        self._win = win
        self._popup = Popup(win, self._copy)
        self._label.bind("<Button-3>", lambda e: self._right_click(e, "text"))
        self._source.bind("<Button-3>", lambda e: self._right_click(e, "source"))
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
            if self._popup is not None:
                self._popup.hide()
            self._win.destroy()
            self._win = None
            self._label = None
            self._source = None
            self._close = None
            self._hint = None
            self._popup = None

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
        `make_non_activating` 不奪焦點，這裡是唯一例外（見 overlay 的同名函式）。"""
        try:
            log("[region] forcing keyboard focus")
            self._win.focus_force()
            for label in (self._label, self._source):
                if label is not None and label.tag_ranges("sel"):
                    label.focus_set()   # 焦點給有選取的欄位：Ctrl+C 直達、選取用作用中的顏色畫
                    break
            log("[region] selection took keyboard focus")
        except tk.TclError as exc:
            log(f"[region] selection focus failed: {exc}")

    def _selected_text(self) -> str:
        """譯文或原文列目前的選取文字；兩者都沒有選取就回傳空字串（選取不跨兩列）。"""
        for label in (self._label, self._source):
            if label is not None and label.tag_ranges("sel"):
                return label.get("sel.first", "sel.last")
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

    def _right_click(self, event: tk.Event, target: str) -> None:
        """右鍵點譯文或原文列：記下要複製哪一行（沒有選取時的備援），彈出複製選單。"""
        self._copy_target = target
        if self._popup is not None:
            self._popup.show(event.x_root, event.y_root, t("menu.copy"))

    def _copy(self) -> None:
        """選單「複製」被點：有選取就複製選取範圍，沒有就複製右鍵點的那一整行；
        都沒有文字就只收起選單。"""
        selected = self._selected_text()
        text = selected or (self.text() if self._copy_target == "text" else self.source_text())
        if text and self._win is not None:
            self._win.clipboard_clear()
            self._win.clipboard_append(text)
            self._win.update()   # Windows 下要 flush 過，內容才真的落進系統剪貼簿
            kind = "selection" if selected else self._copy_target
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
