"""overlay 的訊息列表：可捲動的畫布、內層列容器、細捲軸與空狀態提示。

負責訊息列的建立／就地更新／移除、訊息上限與淡出，以及捲動帳目：黏在底部時新訊息
自動跟到底，往上讀歷史時不被搶走，內容變動前取錨、變動後復位。框選、右鍵選單與
泡泡未讀數不在這裡 —— OverlayWindow 透過 `selection` 與 `bind_line` 接手。
"""
import time
import tkinter as tk
from typing import NamedTuple

from src.log import log
from src.ui.fonts import ui_font
from src.ui.geometry import EDGE
from src.ui.palette import (
    BG,
    DIM_FACTOR,
    FG_BAR,
    FG_ERROR,
    FG_ORIGINAL,
    FG_PENDING,
    FG_TRANSLATED,
    OUTLINE,
)
from src.ui.selection import TEXT_ORIGIN, Selection
from src.ui.thin_scrollbar import ThinScrollbar

_OUTLINE_OFFSETS = ((-1, -1), (-1, 0), (-1, 1), (0, -1),
                    (0, 1), (1, -1), (1, 0), (1, 1))
_STICK_THRESHOLD = 0.999


def _fit_line_height(c: "tk.Canvas") -> None:
    """把文字行 canvas 的高度縮放到剛好容納（換行後的）文字內容。"""
    bbox = c.bbox("txt")   # 只量文字項目：這樣列上其他畫的東西（反白矩形）永遠不會影響列高
    if bbox:
        c.configure(height=bbox[3] + 2)


def _outlined_line(parent, text: str, fg: str, font: tuple, wrap: int) -> "tk.Canvas":
    """字幕式描邊文字行：canvas 先畫八方向 1px 偏移的描邊副本、再疊本色 ——
    Label 無法描邊，透明度調低時文字壓在亮色遊戲畫面上會失去對比。"""
    c = tk.Canvas(parent, bg=BG, highlightthickness=0, bd=0)
    for dx, dy in _OUTLINE_OFFSETS:
        c.create_text(TEXT_ORIGIN + dx, TEXT_ORIGIN + dy, text=text, fill=OUTLINE, font=font,
                      anchor="nw", width=wrap, tags="txt")
    # 本色最後畫，疊在描邊之上。額外掛 "fg" tag：改色時只動本色，描邊不能跟著變
    c.create_text(TEXT_ORIGIN, TEXT_ORIGIN, text=text, fill=fg, font=font, anchor="nw",
                  width=wrap, tags=("txt", "fg"))
    _fit_line_height(c)
    return c


def dimmed(color: str, factor: float = DIM_FACTOR) -> str:
    """把 `#rrggbb` 各通道乘上係數調暗 —— 原文行用遊戲色的暗版，維持原文暗、譯文亮的層次。"""
    r, g, b = (int(color[i:i + 2], 16) for i in (1, 3, 5))
    return f"#{round(r * factor):02x}{round(g * factor):02x}{round(b * factor):02x}"


def should_stick_to_bottom(view_bottom_fraction: float,
                           threshold: float = _STICK_THRESHOLD) -> bool:
    """視圖底緣接近最底時，新訊息應自動跟到底；使用者往上捲時則否。"""
    return view_bottom_fraction >= threshold


def slot_marker(slot: int) -> str:
    """客戶端編號的顯示字：帶圈數字 ①…⑳，超過退回 `[n]`。"""
    if 1 <= slot <= 20:
        return chr(0x2460 + slot - 1)
    return f"[{slot}]"


class _Message(NamedTuple):
    """列表中的一則訊息。msg_id 為 None 代表不需要就地更新（例如測試直接塞完成品）；
    color 為該則在遊戲內的顯示色（None＝退回預設配色）。"""
    ts: float
    original: str
    translated: str
    row: "tk.Frame"
    msg_id: int | None
    color: str | None = None
    slot: int | None = None   # 來自哪個遊戲客戶端（None＝不標示）


class MessageList:
    """可捲動的訊息列表元件。`frame` 是它佔的那塊區域（橫幅 pack 時要排在它之前）；
    `wrap` 是目前文字換行寬度，隨畫布寬度更新後經 `on_wrap_change` 通知呼叫端。
    `bind_line(line)` 在每一行文字 canvas 建好時呼叫，讓呼叫端接框選與右鍵事件。"""

    def __init__(self, parent: tk.Misc, selection: Selection, *, max_messages: int,
                 fade_seconds: int, wrap: int, bottom_inset: int, bind_line, on_wrap_change):
        self._selection = selection
        self._bind_line = bind_line
        self._on_wrap_change = on_wrap_change
        self._max = max_messages
        self._fade = fade_seconds
        self.wrap = wrap
        self._messages: list[_Message] = []
        # 視圖是否黏在底部。只在使用者主動捲動時重新評估：縮放視窗／橫幅進出也會把視圖
        # 推離底部，每次加訊息時當場採樣會誤判成「使用者往上捲」。
        self._follow = True
        # 多客戶端模式：偵測到第二個客戶端才開、開了不關（見 set_multi_client）
        self._multi_client = False

        self.frame = tk.Frame(parent, bg=BG)
        self.frame.pack(side="top", fill="both", expand=True)
        self._canvas = tk.Canvas(self.frame, bg=BG, highlightthickness=0)
        self._scrollbar = ThinScrollbar(self.frame, command=self._user_scroll)
        self._scrollbar.bind("<MouseWheel>", self.on_wheel)  # 游標壓在捲軸上也能滾
        self._canvas.configure(yscrollcommand=self._scrollbar.set)
        # 底部讓出縮放把手的高度：把手 place 在視窗右下角，捲軸鋪到底會被它壓住
        self._scrollbar.pack(side="right", fill="y", padx=(0, EDGE),
                             pady=(0, bottom_inset))
        self._canvas.pack(side="left", fill="both", expand=True)
        self._inner = tk.Frame(self._canvas, bg=BG)
        self._inner_id = self._canvas.create_window((0, 0), window=self._inner, anchor="nw")
        self._inner.bind(
            "<Configure>",
            lambda e: self._canvas.configure(scrollregion=self._canvas.bbox("all")),
        )
        self._canvas.bind("<Configure>", self._on_canvas_configure)
        self._canvas.bind("<Enter>", lambda e: self._canvas.bind_all("<MouseWheel>", self.on_wheel))
        self._canvas.bind("<Leave>", lambda e: self._canvas.unbind_all("<MouseWheel>"))

        # 空狀態提示：沒有訊息時把目前狀態大字置中顯示
        self._placeholder = tk.Label(self.frame, text="", bg=BG, fg=FG_BAR,
                                     font=ui_font(11))
        self._placeholder.place(relx=0.5, rely=0.5, anchor="center")

    # --- 捲動 ---
    def _on_canvas_configure(self, e) -> None:
        # 內層寬度跟著畫布寬，文字才會依視窗寬換行
        self._canvas.itemconfigure(self._inner_id, width=e.width)
        self.wrap = max(80, e.width - 12)
        # 既有訊息的換行寬度也要同步更新，否則縮小視窗後右緣被切
        for entry in self._messages:
            for child in entry.row.winfo_children():
                child.itemconfigure("txt", width=self.wrap)
                _fit_line_height(child)
        self._selection.redraw()
        self._on_wrap_change(self.wrap)
        # 排到 idle 再貼底，不在事件處理中直接 update_idletasks() —— 那會讓下一個
        # Configure 事件重入本函式；此時排版也尚未完成，量到的高度是舊的。
        self._canvas.after_idle(self.refresh_scroll)

    def on_wheel(self, e) -> None:
        """滾輪捲動（畫布、捲軸與底板的空白處共用）。"""
        self._canvas.yview_scroll(int(-e.delta / 120), "units")
        self.note_scroll()

    def _user_scroll(self, *args) -> None:
        """捲軸拖曳入口：捲完記下使用者是否仍要跟隨底部。"""
        self._canvas.yview(*args)
        self.note_scroll()

    def scroll_debug(self) -> str:
        """捲動狀態快照，供 log 定位「訊息看不到／捲不到底」；region_h 與 content_h
        對不上就代表捲動範圍是舊的。"""
        region = self._canvas.cget("scrollregion").split()
        bbox = self._canvas.bbox("all")
        return (f"messages={len(self._messages)} follow={self._follow} "
                f"yview_bottom={self._canvas.yview()[1]:.4f} "
                f"region_h={region[3] if len(region) == 4 else '?'} "
                f"content_h={bbox[3] if bbox else '?'} "
                f"canvas_h={self._canvas.winfo_height()}")

    def note_scroll(self) -> None:
        """使用者主動捲動後重新判定是否繼續跟隨底部：往上捲＝正在讀歷史，
        新訊息不該把畫面搶走；捲回底部則恢復跟隨。"""
        follow = should_stick_to_bottom(self._canvas.yview()[1])
        if follow != self._follow:
            self._follow = follow
            log(f"[ui] auto-follow {'enabled' if follow else 'disabled'} "
                f"by user scroll ({self.scroll_debug()})")

    def viewport(self) -> tuple[int, int, int, int]:
        """訊息視口的螢幕座標與尺寸（x、y、寬、高）；框選判定與自動捲動用。"""
        c = self._canvas
        return c.winfo_rootx(), c.winfo_rooty(), c.winfo_width(), c.winfo_height()

    def scroll_by(self, pixels: int) -> bool:
        """把視圖捲動指定像素（負值往上），內容量不出來時回 False。"""
        bbox = self._canvas.bbox("all")
        height = (bbox[3] - bbox[1]) if bbox else 0
        if height <= 0:
            return False
        self._canvas.yview_moveto(
            min(1.0, max(0.0, self._canvas.yview()[0] + pixels / height)))
        self.note_scroll()
        return True

    def _view_anchor(self) -> tuple["tk.Misc", int] | None:
        """視圖目前對齊到的內容位置：（最新一則的列，它相對視口頂端的偏移）。

        畫布記的是像素原點而非「看到哪一則」，清掉上方舊訊息或改變某列高度時底下
        內容會整段滑動、正在讀的行就跳掉 —— 內容變動前取錨、變動後交給
        `refresh_scroll` 復位。跟隨底部時直接貼底不需要錨；最新一則不會被上方的
        清除移走，拿它當錨最穩。拖曳框選中即使處於跟隨狀態也要給錨 —— prune 把上方
        訊息清掉會讓下方內容整段上移，沒有錨點補位的話游標下的字就會被換掉。"""
        if (self._follow and not self._selection.dragging) or not self._messages:
            return None
        row = self._messages[-1].row
        return row, row.winfo_y() - int(self._canvas.canvasy(0))

    def _restore_anchor(self, row: "tk.Misc", offset: int) -> None:
        """把捲動位置移回「錨點列仍在視口同一偏移」處（見 `_view_anchor`）。"""
        bbox = self._canvas.bbox("all")
        if not row.winfo_exists() or bbox is None or bbox[3] <= bbox[1]:
            # 罕見（錨點列被清光、內容量不出來）：放棄補位，視圖會跳一下
            log(f"[ui] scroll anchor unusable, view may jump "
                f"(row_alive={bool(row.winfo_exists())} bbox={bbox}) "
                f"({self.scroll_debug()})")
            return
        target = max(bbox[1], row.winfo_y() - offset)
        self._canvas.yview_moveto((target - bbox[1]) / (bbox[3] - bbox[1]))

    def refresh_scroll(self, anchor: tuple["tk.Misc", int] | None = None) -> None:
        """重算捲動範圍；跟隨模式貼回底部，否則依 anchor 維持原位。

        任何改變畫布內容或幾何的動作都要呼叫（新增／更新／清除訊息、縮放、橫幅進出、
        視窗重新顯示）。漏呼叫的後果是視圖從此停在舊位置 —— `_follow` 仍為真卻沒人
        貼底，之後每則新訊息都落在畫面外，看起來就像訊息漏掉了。
        框選拖曳期間不貼底：畫面被新訊息拉走的話，游標下的字會整個換掉。"""
        self._canvas.update_idletasks()
        # 視窗隱藏（縮成泡泡、工作列收合）期間畫布不重繪，內嵌容器與捲動帳目脫節 ——
        # yview 回報已在底部，畫面卻少了最後幾則、往下也捲不動。重設一次座標（值不變）
        # 即可要求畫布重新擺放它。
        self._canvas.coords(self._inner_id, 0, 0)
        self._canvas.configure(scrollregion=self._canvas.bbox("all"))
        if self._follow and not self._selection.dragging:
            self._canvas.yview_moveto(1.0)
        elif anchor is not None:
            self._restore_anchor(*anchor)

    # --- 訊息 ---
    def _drop_row(self, entry: _Message) -> None:
        """移除一則訊息的畫面元件。銷毀列與解除選取登記必須成對 —— 漏掉任一處，
        選取就會指向已銷毀的 widget。"""
        self._selection.forget(entry.row)
        entry.row.destroy()

    def add_message(self, original: str, translated: str, now: float | None = None,
                    msg_id: int | None = None, pending: bool = False,
                    color: str | None = None, slot: int | None = None) -> None:
        """加入一則訊息（最新在最下）。pending＝譯文欄位目前是佔位字樣，以較暗的顏色
        標示，待 update_message 填入真正的譯文時才恢復正常顏色。
        color＝該則在遊戲內的顯示色：譯文直接用它、原文用調暗版；None 退回預設配色。
        slot＝來源客戶端編號，只在多客戶端模式下畫成前綴。"""
        anchor = self._view_anchor()
        row = tk.Frame(self._inner, bg=BG)
        original_line = _outlined_line(row, self._display_original(original, slot),
                                       dimmed(color) if color else FG_ORIGINAL,
                                       ui_font(9), self.wrap)
        original_line.pack(fill="x")
        translated_line = _outlined_line(row, translated,
                                         FG_PENDING if pending else (color or FG_TRANSLATED),
                                         ui_font(11), self.wrap)
        translated_line.pack(fill="x")
        row.pack(side="top", fill="x", pady=2)
        self._selection.register(row, (original_line, translated_line))
        for line in (original_line, translated_line):
            self._bind_line(line)
        self._messages.append(_Message(now if now is not None else time.time(),
                                       original, translated, row, msg_id, color, slot))
        while len(self._messages) > self._max:
            self._drop_row(self._messages.pop(0))

        self._refresh_placeholder()
        self.refresh_scroll(anchor)

    def update_message(self, msg_id: int, translated: str,
                       failed: bool = False) -> None:
        """把某則佔位訊息的譯文就地填入（原文與位置不動）。
        failed＝這則翻不出來、填入的是失敗提示，改用錯誤色與一般對話區隔。
        找不到 msg_id 代表該則已被 prune 或 max_messages 擠掉，安靜忽略。"""
        anchor = self._view_anchor()
        for i, m in enumerate(self._messages):
            if m.msg_id != msg_id:
                continue
            line = m.row.winfo_children()[1]  # 0＝原文行，1＝譯文行
            if self._selection.holds(m.row):
                self._selection.clear("translated line replaced")
            line.itemconfigure("txt", text=translated)
            line.itemconfigure("fg", fill=FG_ERROR if failed
                               else (m.color or FG_TRANSLATED))
            _fit_line_height(line)
            self._messages[i] = m._replace(translated=translated)
            self.refresh_scroll(anchor)
            return

    def _display_original(self, original: str, slot: int | None) -> str:
        if self._multi_client and slot is not None:
            return f"{slot_marker(slot)} {original}"
        return original

    def set_multi_client(self) -> None:
        """進入多客戶端模式：既有各列補上來源標記，之後新列直接帶標記。只開不關 ——
        剛關掉的那個客戶端的訊息還在淡出期內，此時正需要看清是誰的。"""
        if self._multi_client:
            return
        self._multi_client = True
        anchor = self._view_anchor()
        for m in self._messages:
            if m.slot is None:
                continue
            line = m.row.winfo_children()[0]  # 0＝原文行
            if self._selection.holds(m.row):
                self._selection.clear("original line relabelled")
            line.itemconfigure("txt", text=self._display_original(m.original, m.slot))
            _fit_line_height(line)
        self.refresh_scroll(anchor)

    def set_limits(self, max_messages: int, fade_seconds: int) -> None:
        """套用新的訊息上限與淡出秒數；超出上限的最舊訊息立即移除。"""
        anchor = self._view_anchor()
        self._max = max_messages
        self._fade = fade_seconds
        removed = False
        while len(self._messages) > self._max:
            self._drop_row(self._messages.pop(0))
            removed = True
        self._refresh_placeholder()
        if removed:
            self.refresh_scroll(anchor)

    def prune(self, now: float | None = None) -> None:
        """移除已超過淡出秒數的訊息（fade_seconds <= 0 時不做）；`now` 供測試指定時刻。"""
        if self._fade <= 0:
            return  # fade_seconds <= 0：永不依時間清除訊息（可滾動看歷史）
        anchor = self._view_anchor()
        cutoff = (now if now is not None else time.time()) - self._fade
        keep = []
        for entry in self._messages:
            if entry.ts <= cutoff:
                self._drop_row(entry)
            else:
                keep.append(entry)
        if len(keep) == len(self._messages):
            return
        self._messages = keep
        self._refresh_placeholder()
        self.refresh_scroll(anchor)

    # --- 空狀態提示 ---
    def set_placeholder(self, text: str, color: str) -> None:
        """設定沒有訊息時置中顯示的狀態字（有訊息時不顯示）。"""
        self._placeholder.configure(text=text, fg=color)
        self._refresh_placeholder()

    def refresh_fonts(self) -> None:
        """介面語言變更後重取字型（不同語言用不同字族）。"""
        self._placeholder.configure(font=ui_font(11))

    def _refresh_placeholder(self) -> None:
        if self._messages:
            self._placeholder.place_forget()
        else:
            self._placeholder.place(relx=0.5, rely=0.5, anchor="center")
            self._placeholder.lift()

    # --- 測試/除錯輔助 ---
    def visible_messages(self) -> list[tuple[str, str]]:
        return [(m.original, m.translated) for m in self._messages]

    def placeholder_visible(self) -> bool:
        return self._placeholder.winfo_manager() == "place"
