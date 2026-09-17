"""框選翻譯的結果卡片：貼在框選矩形正下方、與它同寬；辨識中／譯文／錯誤三態。

不奪焦點（與彈出選單同一招）：遊戲的鍵盤操作不中斷，代價是收不到 Esc，
關閉方式是點一下卡片、點 ✕、或下一次框選時被換掉；✕ 與內文下方的一行提示文字是
讓使用者看得出怎麼關。位置每次重排：譯文回來後高度變了，下方放不下翻到矩形上方，
上方也放不下就移到左右較寬的一側（`geometry.anchored_geometry`）。貼上下時與矩形同寬、
高隨內容；貼側邊時改用內文不換行的寬度、高度夾在工作區內，多出來的內容靠滾輪／細捲軸
捲動。側邊的落點會黏住直到內容換掉：側邊可能比矩形寬，換行後變矮，若拿這個高度重判
會誤以為下方放得下而跳回去、再換行變高、再跳回側邊，來回不止。
原文（本機 OCR 辨識出的文字）以暗色小字放在譯文上方 —— 與聊天疊加視窗「原文在上、
譯文在下」一致，也讓使用者能核對模型有沒有多翻或漏翻。原文與譯文放在同一顆
`RichLabel`（`richtext.RichLabel.set_blocks`）裡、中間空一行分隔，拖曳選取才能一路
跨過兩段文字。
內文可拖曳選取（沿用 Tk 對唯讀 Text 的原生選取，不必解除 `state="disabled"`）；
純點擊（按下到放開沒有明顯位移，見 `geometry.is_click`）才關卡片，拖曳不關。
拖曳選取結束後會比照疊加視窗跟 backdrop 借鍵盤焦點（`_focus_for_copy`），讓 Ctrl+C
收得到；右鍵會彈出單項的複製選單（與疊加視窗共用 `Popup`）——有選取就複製選取範圍，
沒有就複製整顆內容（原文＋空行＋譯文，或單純譯文）。
使用者自己關卡片（點一下、✕）會呼叫 `on_close`；流程換位置重開或自己收掉不算，
`RegionFlow` 靠這個分別收掉留在畫面上的框選框。
"""
import tkinter as tk

from src.i18n import t
from src.log import log
from src.ui.clipboard import copy_to_clipboard
from src.ui.fonts import ui_font
from src.ui.geometry import anchored_geometry, beside_geometry, is_click
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
from src.ui.region_box import HANDLE, MARGIN
from src.ui.richtext import RichLabel
from src.ui.thin_scrollbar import ThinScrollbar
from src.ui.tooltip import Tooltip
from src.ui.winstyle import make_non_activating

MIN_WIDTH = 240   # 矩形再窄也不跟：譯文會擠成一長條
SIDE_MIN_WIDTH = 360  # 移到側邊時的寬度下限：文字再短也不縮成窄條，蓋到矩形一角無妨
MIN_HEIGHT = 120  # 高度下限：內文再短也留這麼高；側邊被夾矮時也不低於此，剩下的捲
ANCHOR_GAP = MARGIN + HANDLE // 2 + 2   # 與框選矩形的垂直間距（px）：讓出框選框露在框外的把手
_PAD_X = 10
_PAD_Y = 6


class RegionCard:
    """一次只有一張；`show_pending(rect)` 建窗並佔位，`show_text`／`show_error` 換內容。"""

    def __init__(self, root: tk.Tk, alpha: float):
        self._root = root
        self._alpha = alpha
        self.on_close = None      # 使用者自己關掉卡片時呼叫（流程換位置重開不算）
        self._win: tk.Toplevel | None = None
        self._label: RichLabel | None = None
        self._scrollbar: ThinScrollbar | None = None
        self._side: str | None = None   # 目前落點；貼側邊時黏住，內容換掉才重判（見檔頭）
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
        self._close.bind("<Button-1>", lambda e: self._close_by_user())
        self._close_tooltip = Tooltip(self._close, lambda: t("tooltip.close"))
        # 提示先 pack 在底部：卡片被夾矮時 pack 依序讓位，最後放進來的內文列才是被縮的那個
        self._hint = tk.Label(body, text=t("region.close_hint"), bg=BG, fg=FG_PENDING,
                              font=ui_font(8), anchor="w")
        self._hint.pack(side="bottom", fill="x", padx=_PAD_X, pady=(0, _PAD_Y))
        row = tk.Frame(body, bg=BG)
        row.pack(fill="both", expand=True, padx=_PAD_X, pady=_PAD_Y)
        self._label = RichLabel(row, fg=FG_PENDING, bg=BG, font=ui_font(11),
                                link_fg=FG_UPDATE, on_height_change=self._layout)
        # 捲軸常駐（內容放得下時滑塊隱形）：隨需要 pack／pack_forget 會改內文寬度、
        # 換行後高度又變，在臨界高度來回切換（見 scrollable.ScrollableFrame）
        self._scrollbar = ThinScrollbar(row, command=self._label.yview)
        self._scrollbar.pack(side="right", fill="y")
        self._label.configure(yscrollcommand=self._scrollbar.set)
        self._label.pack(side="left", fill="both", expand=True)
        # inactiveselectbackground 在 Windows 預設為空：文字欄沒有鍵盤焦點時選取不會畫出來，
        # 拖曳中與放開後看起來都像沒選到；設成同一個顏色，反白才留得住
        self._label.configure(selectbackground=SELECT_BG, selectforeground=FG_TRANSLATED,
                              inactiveselectbackground=SELECT_BG)
        # 捲軸上的按放不往上傳到視窗層：點一下滑塊不能算「點卡片」而把卡片關掉
        for sequence in ("<ButtonPress-1>", "<ButtonRelease-1>"):
            self._scrollbar.bind(sequence, lambda e: "break", add="+")
        # 滾輪比照 message_list：卡片不奪焦點，滾輪事件不保證落在內文元件上，
        # 游標移入時 bind_all 接管、移出時還回去
        win.bind("<Enter>", lambda e: win.bind_all("<MouseWheel>", self._on_wheel))
        win.bind("<Leave>", lambda e: win.unbind_all("<MouseWheel>"))
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
        `set_blocks`）；空字串則只放譯文一段。"""
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
        self._content_changed()

    def show_error(self, message: str) -> None:
        if self._label is None:
            return
        self._shown_text = message
        self._label.set(message, FG_ERROR)
        self._content_changed()

    def _content_changed(self) -> None:
        """內容換了就重判落點。先把寬度收回矩形寬再量高：上一次若貼在側邊、寬度比矩形寬，
        量到的高度會偏矮而誤判。重排排到 idle：RichLabel 也是在 idle 才量行數，排在它後面
        才拿得到新高度（行數沒變時它不會回呼，這一次就是唯一的重排）。"""
        self._side = None
        self._win.geometry(f"{self._width}x{self._win.winfo_height()}")
        self._win.after_idle(self._layout)

    def hide(self) -> None:
        if self._win is not None:
            if self._popup is not None:
                self._popup.hide()
            # _close_tooltip 的視窗是 self._win 的子視窗（見 Tooltip._build），
            # destroy 這裡會一併收掉，不必另外呼叫 hide()
            self._win.destroy()
            self._win = None
            self._label = None
            self._scrollbar = None
            self._side = None
            self._close = None
            self._close_tooltip = None
            self._hint = None
            self._popup = None
            self._shown_text = ""
            self._shown_source = ""

    def text(self) -> str:
        """目前顯示的譯文（測試用）；原文與譯文同放一顆 Text，從 widget 反推不出來，
        改記錄 `show_text`／`show_pending`／`show_error` 實際放的字串。"""
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
            self._close_by_user()
        else:
            self._focus_for_copy()

    def _close_by_user(self) -> None:
        self.hide()
        if self.on_close is not None:
            self.on_close()

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
        copy_to_clipboard(self._win, text)
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
            copy_to_clipboard(self._win, text)
            kind = "selection" if selected else "text"
            log(f"[region] copied {kind} ({len(text)} chars)")
        if self._popup is not None:
            self._popup.hide()

    def _layout(self) -> None:
        """依內容高度重新定位：下方 → 上方 → 左右較寬側（見檔頭）。
        `winfo_reqheight` 是內容的自然高度，不受上一次夾過的視窗尺寸影響。"""
        if self._win is None or self._rect is None:
            return
        self._win.update_idletasks()
        natural = self._win.winfo_reqheight()
        area = work_area_at(self._rect[0], self._rect[1])
        if self._side in ("left", "right"):
            placed = beside_geometry(self._rect, self._natural_width(), natural, area,
                                     ANCHOR_GAP, SIDE_MIN_WIDTH, MIN_HEIGHT)
        else:
            placed = anchored_geometry(self._rect, self._width, natural, area, ANCHOR_GAP,
                                       self._natural_width(), SIDE_MIN_WIDTH, MIN_HEIGHT)
        self._side = placed.side
        self._win.geometry(f"{placed.w}x{placed.h}+{placed.x}+{placed.y}")
        log(f"[region] card placed (rect={self._rect}, natural_height={natural}, "
            f"side={placed.side}, size={placed.w}x{placed.h}, at=({placed.x}, {placed.y}))")

    def _natural_width(self) -> int:
        """卡片不換行時的寬度：內文最寬一行加左右留白、捲軸與 1px 邊框。"""
        return (self._label.natural_width() + 2 * _PAD_X
                + self._scrollbar.winfo_reqwidth() + 2)

    def _on_wheel(self, event: tk.Event) -> None:
        """滾輪捲內文；內容放得下時 Text 的 yview 本來就不會動，不必另外判斷。"""
        if self._label is not None:
            self._label.yview_scroll(-int(event.delta / 120), "units")
