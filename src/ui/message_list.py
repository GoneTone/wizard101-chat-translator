"""overlay 的訊息列表：每個（客戶端, 分頁）一個畫布、細捲軸與空狀態提示。

訊息依（客戶端, 分頁）分開保存，各自直接畫成自己畫布上的文字物件，切分頁只換顯示中的
畫布。負責訊息的繪製／就地更新／移除、訊息上限與淡出，以及捲動帳目：黏在底部時新訊息
自動跟到底，往上讀歷史時不被搶走，內容變動前取錨、變動後復位。框選、右鍵選單與
泡泡未讀數不在這裡 —— OverlayWindow 透過 `selection` 與 `bind_line` 接手。
"""
import itertools
import time
import tkinter as tk
from dataclasses import dataclass, field
from typing import NamedTuple

from src.log import log
from src.ui.fonts import ui_font
from src.ui.geometry import EDGE
from src.ui.outlined_text import Line, draw_outlined, line_height, line_items, line_origin
from src.ui.palette import (
    BG,
    DIM_FACTOR,
    FG_BAR,
    FG_ERROR,
    FG_ORIGINAL,
    FG_PENDING,
    FG_TRANSLATED,
)
from src.ui.selection import Selection
from src.ui.thin_scrollbar import ThinScrollbar

_STICK_THRESHOLD = 0.999
_ROW_PAD = 2   # 每則訊息上下各留的間距（列與列之間共 4px）


def dimmed(color: str, factor: float = DIM_FACTOR) -> str:
    """把 `#rrggbb` 各通道乘上係數調暗 —— 原文行用遊戲色的暗版，維持原文暗、譯文亮的層次。"""
    r, g, b = (int(color[i:i + 2], 16) for i in (1, 3, 5))
    return f"#{round(r * factor):02x}{round(g * factor):02x}{round(b * factor):02x}"


def should_stick_to_bottom(view_bottom_fraction: float,
                           threshold: float = _STICK_THRESHOLD) -> bool:
    """視圖底緣接近最底時，新訊息應自動跟到底；使用者往上捲時則否。"""
    return view_bottom_fraction >= threshold


ViewKey = tuple[int | None, int]   # （客戶端 slot, 遊戲分頁編號）


class _Message(NamedTuple):
    """列表中的一則訊息記錄。msg_id 為 None 代表不需要就地更新（例如測試直接塞完成品）；
    color 為該則在遊戲內的顯示色（None＝退回預設配色）；lines＝（原文行, 譯文行），
    tag＝該則所有文字物件共用的 tag，同時是它在選取登記表裡的鍵。"""
    ts: float
    original: str
    translated: str
    msg_id: int | None
    color: str | None
    pending: bool
    failed: bool
    lines: tuple[Line, ...] = ()
    tag: str = ""

    def translated_fill(self) -> str:
        if self.pending:
            return FG_PENDING
        return FG_ERROR if self.failed else (self.color or FG_TRANSLATED)


@dataclass
class _View:
    """一個（客戶端, 分頁）的訊息：畫布、記錄、選取登記表，以及排版時用的換行寬度。"""
    canvas: tk.Canvas
    wrap: int
    records: list[_Message] = field(default_factory=list)
    rows: dict = field(default_factory=dict)


def _row_bottom(m: _Message) -> float:
    translated = m.lines[1]
    return line_origin(translated)[1] + line_height(translated)


def _move_line(line: Line, dy: float) -> None:
    for item in line_items(line):
        line.canvas.move(item, 0, dy)


class MessageList:
    """可捲動的訊息列表元件。`frame` 是它佔的那塊區域（橫幅 pack 時要排在它之前）；
    `wrap` 是目前文字換行寬度，隨畫布寬度更新後經 `on_wrap_change` 通知呼叫端。
    `bind_line(canvas)` 在每個分頁畫布建好時呼叫一次，讓呼叫端接框選與右鍵事件。"""

    def __init__(self, parent: tk.Misc, selection: Selection, *, max_messages: int,
                 fade_seconds: int, wrap: int, bottom_inset: int, bind_line, on_wrap_change):
        self._selection = selection
        self._bind_line = bind_line
        self._on_wrap_change = on_wrap_change
        self._max = max_messages
        self._fade = fade_seconds
        self.wrap = wrap
        self._tags = itertools.count()
        self._views: dict[ViewKey, _View] = {}
        # 視圖是否黏在底部。只在使用者主動捲動時重新評估：縮放視窗／橫幅進出也會把視圖
        # 推離底部，每次加訊息時當場採樣會誤判成「使用者往上捲」。
        self._follow = True
        # 同一輪事件內連續新增只排一次捲動更新；值是第一則新增前的錨點與分頁
        self._scroll_pending: tuple[ViewKey, tuple[str, int] | None] | None = None

        self.frame = tk.Frame(parent, bg=BG)
        self.frame.pack(side="top", fill="both", expand=True)
        self._scrollbar = ThinScrollbar(self.frame, command=self._user_scroll)
        self._scrollbar.bind("<MouseWheel>", self.on_wheel)  # 游標壓在捲軸上也能滾
        # 底部讓出縮放把手的高度：把手 place 在視窗右下角，捲軸鋪到底會被它壓住
        self._scrollbar.pack(side="right", fill="y", padx=(0, EDGE),
                             pady=(0, bottom_inset))
        self.shown_key: ViewKey = (None, 0)
        self._attach(self._view(self.shown_key))

        # 空狀態提示：沒有訊息時把目前狀態大字置中顯示
        self._placeholder = tk.Label(self.frame, text="", bg=BG, fg=FG_BAR,
                                     font=ui_font(11))
        self._placeholder.place(relx=0.5, rely=0.5, anchor="center")

    @property
    def _messages(self) -> list[_Message]:
        return self._views[self.shown_key].records

    @property
    def _canvas(self) -> tk.Canvas:
        return self._views[self.shown_key].canvas

    def _view(self, key: ViewKey) -> _View:
        """取 key 的分頁；第一次用到時才建它的畫布。"""
        view = self._views.get(key)
        if view is None:
            c = tk.Canvas(self.frame, bg=BG, highlightthickness=0)
            c.bind("<Configure>", self._on_canvas_configure)
            c.bind("<Enter>", lambda e: c.bind_all("<MouseWheel>", self.on_wheel))
            c.bind("<Leave>", lambda e: c.unbind_all("<MouseWheel>"))
            self._bind_line(c)
            view = self._views[key] = _View(c, self.wrap)
        return view

    def _attach(self, view: _View) -> None:
        """讓這個分頁成為顯示中的那個：選取改用它的登記表、畫布接上捲軸並 pack。"""
        self._selection.use_rows(view.rows)
        view.canvas.configure(yscrollcommand=self._scrollbar.set)
        view.canvas.pack(side="left", fill="both", expand=True)

    # --- 排版 ---
    def _rewrap(self, key: ViewKey) -> None:
        """以目前的換行寬度重畫 key 那個分頁（換行變了，行高與其下各則的位置都會變）。

        重畫而不是改寬度再搬移：文字物件每移動一次 Tk 就重新排版一次，搬移等於排兩次。
        各則沿用原本的 tag，捲動錨點與選取登記因此不受重畫影響。"""
        start = time.perf_counter()
        view = self._views[key]
        view.wrap = self.wrap
        records = view.records[:]
        view.canvas.delete("txt")
        view.records.clear()
        view.rows.clear()
        for m in records:
            self._append(view, m)
        log(f"[ui] view rewrapped (slot={key[0]}, tab={key[1]}, messages={len(records)}, "
            f"wrap={self.wrap}, ms={(time.perf_counter() - start) * 1000:.0f})")

    def _region(self) -> tuple[float, float]:
        """顯示中分頁內容的捲動範圍（上緣, 下緣）；沒有訊息時為 (0, 0)。"""
        records = self._messages
        if not records:
            return 0, 0
        return line_origin(records[0].lines[0])[1] - _ROW_PAD, _row_bottom(records[-1]) + _ROW_PAD

    # --- 捲動 ---
    def _on_canvas_configure(self, e) -> None:
        self.wrap = max(80, e.width - 12)
        # 既有訊息的換行寬度也要同步更新，否則縮小視窗後右緣被切；隱藏中的分頁切過去時才補
        if self._views[self.shown_key].wrap != self.wrap:
            self._rewrap(self.shown_key)
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
        return (f"messages={len(self._messages)} follow={self._follow} "
                f"yview_bottom={self._canvas.yview()[1]:.4f} "
                f"region_h={region[3] if len(region) == 4 else '?'} "
                f"content_h={self._region()[1]} "
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
        top, bottom = self._region()
        height = bottom - top
        if height <= 0:
            return False
        self._canvas.yview_moveto(
            min(1.0, max(0.0, self._canvas.yview()[0] + pixels / height)))
        self.note_scroll()
        return True

    def _view_anchor(self) -> tuple[str, int] | None:
        """視圖目前對齊到的內容位置：（最新一則的 tag，它的列頂相對視口頂端的偏移）。

        畫布記的是像素原點而非「看到哪一則」，清掉上方舊訊息或改變某列高度時底下
        內容會整段滑動、正在讀的行就跳掉 —— 內容變動前取錨、變動後交給
        `refresh_scroll` 復位。跟隨底部時直接貼底不需要錨；最新一則不會被上方的
        清除移走，拿它當錨最穩。拖曳框選中即使處於跟隨狀態也要給錨 —— prune 把上方
        訊息清掉會讓下方內容整段上移，沒有錨點補位的話游標下的字就會被換掉。"""
        if (self._follow and not self._selection.dragging) or not self._messages:
            return None
        newest = self._messages[-1]
        return newest.tag, round(line_origin(newest.lines[0])[1]) - int(self._canvas.canvasy(0))

    def _restore_anchor(self, tag: str, offset: int) -> None:
        """把捲動位置移回「錨點那則仍在視口同一偏移」處（見 `_view_anchor`）。"""
        top, bottom = self._region()
        # 由新往舊找：錨點是取錨當下最新的一則，之後最多再多幾則新訊息
        m = next((m for m in reversed(self._messages) if m.tag == tag), None)
        alive = m is not None
        if not alive or bottom <= top:
            # 罕見（錨點那則被清光、內容量不出來）：放棄補位，視圖會跳一下
            log(f"[ui] scroll anchor unusable, view may jump "
                f"(row_alive={alive} region=({top}, {bottom})) "
                f"({self.scroll_debug()})")
            return
        target = max(top, line_origin(m.lines[0])[1] - offset)
        self._canvas.yview_moveto((target - top) / (bottom - top))

    def refresh_scroll(self, anchor: tuple[str, int] | None = None) -> None:
        """重算捲動範圍；跟隨模式貼回底部，否則依 anchor 維持原位。

        任何改變畫布內容或幾何的動作都要呼叫（新增／更新／清除訊息、縮放、橫幅進出、
        視窗重新顯示）。漏呼叫的後果是視圖從此停在舊位置 —— `_follow` 仍為真卻沒人
        貼底，之後每則新訊息都落在畫面外，看起來就像訊息漏掉了。
        框選拖曳期間不貼底：畫面被新訊息拉走的話，游標下的字會整個換掉。"""
        self._canvas.update_idletasks()
        top, bottom = self._region()
        self._canvas.configure(scrollregion=(0, top, self._canvas.winfo_width(), bottom))
        if self._follow and not self._selection.dragging:
            self._canvas.yview_moveto(1.0)
        elif anchor is not None:
            self._restore_anchor(*anchor)

    # --- 訊息 ---
    def _drop(self, view: _View, m: _Message) -> None:
        """刪除一則訊息的文字物件並解除選取登記。兩者必須成對 —— 漏掉任一處，
        選取就會指向已刪除的物件。"""
        self._selection.forget(m.tag)
        view.rows.pop(m.tag, None)
        view.canvas.delete(*line_items(m.lines[0]), *line_items(m.lines[1]))

    def _draw(self, view: _View, m: _Message) -> _Message:
        """把一則訊息畫在該分頁畫布的最下方（原文行＋譯文行），回傳帶著行與 tag 的記錄。"""
        c = view.canvas
        tag = m.tag or f"msg{next(self._tags)}"   # 全域唯一：Selection.forget 只認 tag，跨分頁不能撞名
        top = _row_bottom(view.records[-1]) + 2 * _ROW_PAD if view.records else _ROW_PAD
        original = draw_outlined(c, 0, top, m.original,
                                 fill=dimmed(m.color) if m.color else FG_ORIGINAL,
                                 font=ui_font(9), wrap=self.wrap, tag=f"{tag}.0")
        translated = draw_outlined(c, 0, top + line_height(original), m.translated,
                                   fill=m.translated_fill(), font=ui_font(11),
                                   wrap=self.wrap, tag=f"{tag}.1")
        for line in (original, translated):
            for item in line_items(line):
                c.addtag_withtag(tag, item)
        return m._replace(lines=(original, translated), tag=tag)

    def _append(self, view: _View, m: _Message) -> None:
        """畫出一則訊息、記進清單，並登記給選取（顯示中分頁的 rows 就是 Selection 的登記表）。"""
        m = self._draw(view, m)
        view.records.append(m)
        view.rows[m.tag] = m.lines

    def _trim(self, view: _View) -> bool:
        """把超過上限的最舊記錄移除；回傳是否有移除。"""
        removed = len(view.records) > self._max
        while len(view.records) > self._max:
            self._drop(view, view.records.pop(0))
        return removed

    def add_message(self, key: ViewKey, original: str, translated: str,
                    now: float | None = None, msg_id: int | None = None,
                    pending: bool = False, color: str | None = None) -> None:
        """把一則訊息加到 key 那份清單最下方。pending＝譯文欄位目前是佔位字樣，以較暗的
        顏色標示，待 update_message 填入真正的譯文時才恢復正常顏色。
        color＝該則在遊戲內的顯示色：譯文直接用它、原文用調暗版；None 退回預設配色。"""
        shown = key == self.shown_key
        anchor = self._view_anchor() if shown else None
        view = self._view(key)
        self._append(view, _Message(now if now is not None else time.time(), original,
                                    translated, msg_id, color, pending, False))
        self._trim(view)
        if shown:
            self._refresh_placeholder()
            if self._scroll_pending is None:
                self._scroll_pending = (key, anchor)
                self._canvas.after_idle(self._flush_scroll)

    def _flush_scroll(self) -> None:
        """執行 add_message 排好的那一次捲動更新；期間已換分頁就略過（show 自己會處理）。"""
        key, anchor = self._scroll_pending
        self._scroll_pending = None
        if key == self.shown_key:
            self.refresh_scroll(anchor)

    def update_message(self, msg_id: int, translated: str,
                       failed: bool = False) -> None:
        """把某則佔位訊息的譯文填入每份含它的清單（廣播會落在多個分頁），顯示中的
        順帶調整捲動。failed＝填入的是失敗提示，改用錯誤色與一般對話區隔。
        找不到 msg_id 代表該則已被 prune 或 max_messages 擠掉，安靜忽略。"""
        anchor = self._view_anchor()
        for key, view in self._views.items():
            for i, m in enumerate(view.records):
                if m.msg_id != msg_id:
                    continue
                m = view.records[i] = m._replace(translated=translated, pending=False,
                                                 failed=failed)
                if self._selection.holds(m.tag):
                    self._selection.clear("translated line replaced")
                line = m.lines[1]
                before = line_height(line)
                for item in line_items(line):
                    view.canvas.itemconfigure(item, text=translated)
                view.canvas.itemconfigure(line.fg, fill=m.translated_fill())
                delta = line_height(line) - before
                if delta:
                    for later in view.records[i + 1:]:
                        for later_line in later.lines:
                            _move_line(later_line, delta)
                if key == self.shown_key:
                    if delta:
                        self._selection.redraw()   # 反白矩形不帶訊息 tag，不會跟著搬
                    self.refresh_scroll(anchor)
                break

    def show(self, key: ViewKey) -> None:
        """換成顯示 key 那個分頁的畫布（已畫好的直接沿用），捲動回到跟隨底部。"""
        if key == self.shown_key:
            return
        self._selection.clear("view switched")
        old = self._canvas
        old.pack_forget()
        old.configure(yscrollcommand="")
        self.shown_key = key
        view = self._view(key)
        self._attach(view)
        if view.wrap != self.wrap:
            self._rewrap(key)
        self._follow = True
        self._refresh_placeholder()
        self.refresh_scroll()
        log(f"[ui] view switched (slot={key[0]}, tab={key[1]}, "
            f"messages={len(view.records)})")

    def set_limits(self, max_messages: int, fade_seconds: int) -> None:
        """套用新的訊息上限與淡出秒數；每份清單超出上限的最舊訊息立即移除。"""
        anchor = self._view_anchor()
        self._max = max_messages
        self._fade = fade_seconds
        removed = False
        for key, view in self._views.items():
            removed |= self._trim(view) and key == self.shown_key
        self._refresh_placeholder()
        if removed:
            self.refresh_scroll(anchor)

    def prune(self, now: float | None = None) -> None:
        """移除每份清單中已超過淡出秒數的訊息（fade_seconds <= 0 時不做）；`now` 供測試指定時刻。"""
        if self._fade <= 0:
            return  # fade_seconds <= 0：永不依時間清除訊息（可滾動看歷史）
        anchor = self._view_anchor()
        cutoff = (now if now is not None else time.time()) - self._fade
        shown_changed = False
        for key, view in self._views.items():
            keep = [m for m in view.records if m.ts > cutoff]
            if len(keep) == len(view.records):
                continue
            for m in view.records:
                if m.ts <= cutoff:
                    self._drop(view, m)
            view.records[:] = keep
            shown_changed |= key == self.shown_key
        if shown_changed:
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
