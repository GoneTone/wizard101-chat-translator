"""overlay 的訊息列表：每個（客戶端, 分頁）一個畫布、細捲軸與空狀態提示。

訊息依（客戶端, 分頁）分開保存，各自直接畫成自己畫布上的文字物件，切分頁只換顯示中的
畫布；換行寬度改變後只同步重畫視口附近，其餘在背景分批補畫、捲到時先補。負責訊息的繪製／就地更新／移除、
訊息上限與淡出，以及捲動帳目：黏在底部時新訊息自動跟到底，往上讀歷史時不被搶走，
內容變動前取錨、變動後復位。框選、右鍵選單與泡泡未讀數不在這裡 —— OverlayWindow 透過
`selection` 與 `bind_line` 接手。
"""
import bisect
import itertools
import math
import time
import tkinter as tk
from dataclasses import dataclass, field
from typing import NamedTuple

from src.log import log
from src.ui.fonts import ui_font
from src.ui.geometry import EDGE
from src.ui.outlined_text import (
    Line,
    draw_outlined,
    line_height,
    line_items,
    line_origin,
    measure_height,
)
from src.ui.palette import (
    BG,
    DIM_FACTOR,
    FG_BAR,
    FG_ERROR,
    FG_ORIGINAL,
    FG_PENDING,
    FG_TRANSLATED,
)
from src.ui.selection import Row, Selection
from src.ui.thin_scrollbar import ThinScrollbar

_STICK_THRESHOLD = 0.999
_ROW_PAD = 2   # 每則訊息上下各留的間距（列與列之間共 4px）
_SLOW_RELAYOUT_MS = 50
_BACKFILL_BUDGET = 0.020   # 秒
_BACKFILL_DELAY_MS = 150


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
    color 為該則在遊戲內的顯示色（None＝退回預設配色）；lines＝（原文行, 譯文行），空＝尚未畫出；
    tag＝該則所有文字物件共用的 tag，同時是它在選取登記表裡的鍵；height／drawn_wrap＝上次畫出時的
    整則高度（含上下間距）與當時的換行寬度，供未畫時估計高度。"""
    ts: float
    original: str
    translated: str
    msg_id: int | None
    color: str | None
    pending: bool
    failed: bool
    lines: tuple[Line, ...] = ()
    tag: str = ""
    height: float = 0
    drawn_wrap: int = 0

    def translated_fill(self) -> str:
        if self.pending:
            return FG_PENDING
        return FG_ERROR if self.failed else (self.color or FG_TRANSLATED)


@dataclass
class _View:
    """一個（客戶端, 分頁）的訊息：畫布、記錄、選取登記表、排版時用的換行寬度，
    已畫範圍 `records[lo:hi]`（範圍外的記錄沒有文字物件），以及進行中那輪重排的開始時刻（沒有則 None）。"""
    canvas: tk.Canvas
    wrap: int
    records: list[_Message] = field(default_factory=list)
    rows: dict = field(default_factory=dict)
    lo: int = 0
    hi: int = 0
    rewrap_start: float | None = None


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
        self._one_line = 0.0   # 兩種字型各一行的整則高度，估計未畫訊息用；每次重排時量
        self._views: dict[ViewKey, _View] = {}
        # 視圖是否黏在底部。只在使用者主動捲動時重新評估：縮放視窗／橫幅進出也會把視圖
        # 推離底部，每次加訊息時當場採樣會誤判成「使用者往上捲」。
        self._follow = True
        # 同一輪事件內連續新增只排一次捲動更新；值是第一則新增前的錨點（包成 tuple，錨點本身可能是 None）
        self._scroll_pending: tuple[tuple[str, int] | None] | None = None
        self._backfill_job: str | None = None
        self._backfill_gen = 0
        self._idle_jobs: dict[str, str] = {}   # 名稱 → 已排未跑的 after_idle id，元件銷毀時要取消

        self.frame = tk.Frame(parent, bg=BG)
        self.frame.pack(side="top", fill="both", expand=True)
        self.frame.bind("<Destroy>", lambda e: self._cancel_jobs())
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
    def _relayout(self) -> None:
        """以目前的換行寬度重排顯示中的分頁：刪掉全部文字物件，從錨點同步只畫視口所需，其餘排背景補畫。

        錨點：跟隨底部時是最新一則（底緣留在原本的內容底緣），否則是視口頂端那則（列頂不動）。"""
        start = time.perf_counter()
        self._cancel_backfill()
        key = self.shown_key
        view = self._views[key]
        view.wrap = self.wrap
        records = view.records
        if not records:
            view.rewrap_start = None
            return
        follow = self._follow
        if follow:
            # 估計高度可能帶小數，取整才與一次全部重畫逐像素一致
            anchor, edge = len(records) - 1, round(self._region()[1] - _ROW_PAD)
        else:
            anchor = self._top_drawn(view)
            edge = line_origin(records[anchor].lines[0])[1]
        c = view.canvas
        c.delete("txt")
        for i in range(view.lo, view.hi):
            m = records[i] = records[i]._replace(lines=())
            view.rows[m.tag] = view.rows[m.tag]._replace(lines=())
        self._selection.redraw()
        self._one_line = (measure_height(c, "x", font=ui_font(9), wrap=self.wrap)
                          + measure_height(c, "x", font=ui_font(11), wrap=self.wrap) + 2 * _ROW_PAD)
        view.lo, view.hi = anchor, anchor + 1
        if follow:
            self._draw_above(view, anchor, edge)
        else:
            self._draw(view, anchor, edge)
        self._apply_region()
        if follow:
            c.yview_moveto(1.0)
        self._fill_viewport(log_slow=False)
        # 上一輪還沒補完就再重排（拖拉邊框）算同一輪：同步階段只記第一次，免得每個 Configure 都記
        if view.rewrap_start is None:
            view.rewrap_start = start
            ms = (time.perf_counter() - start) * 1000
            if ms >= _SLOW_RELAYOUT_MS:
                log(f"[ui] view relaid out (slot={key[0]}, tab={key[1]}, drawn={view.hi - view.lo}, "
                    f"messages={len(records)}, wrap={self.wrap}, ms={ms:.0f})")
        self._schedule_backfill(_BACKFILL_DELAY_MS)

    def _schedule_backfill(self, delay_ms: int) -> None:
        """delay_ms 後開始為顯示中分頁背景補畫；先前排的一併作廢。"""
        self._cancel_backfill()
        self._backfill_job = self.frame.after(delay_ms, self._backfill_step, self._backfill_gen)

    def _cancel_backfill(self) -> None:
        """取消已排的背景補畫（換行寬度又變、切走分頁、元件銷毀時）。"""
        self._backfill_gen += 1
        if self._backfill_job is not None:
            self.frame.after_cancel(self._backfill_job)
            self._backfill_job = None

    def _after_idle(self, name: str, func) -> None:
        """把 func 排到 idle；同名的還沒跑就不重複排（跑的時候讀的是當下狀態，跑一次即可）。"""
        def run():
            del self._idle_jobs[name]
            func()

        if name not in self._idle_jobs:
            self._idle_jobs[name] = self.frame.after_idle(run)

    def _cancel_jobs(self) -> None:
        """元件銷毀時取消所有已排的工作：留著的話會呼叫到已刪除的 Tcl 命令。"""
        self._cancel_backfill()
        for job in self._idle_jobs.values():
            self.frame.after_cancel(job)
        self._idle_jobs.clear()

    def _backfill_step(self, gen: int) -> None:
        """補畫一批，還有待補就讓出事件迴圈後接續；補完記 log。"""
        self._backfill_job = None
        if gen != self._backfill_gen:
            return
        if self._backfill_once():
            # 經 idle 再排計時：批次耗時比 1 ms 長，直接 after(1) 會一直到期，Tk 的重繪（idle）輪不到
            self._backfill_job = self.frame.after_idle(self._schedule_backfill, 1)
            return
        view = self._views[self.shown_key]
        if view.rewrap_start is not None and view.records:
            ms = (time.perf_counter() - view.rewrap_start) * 1000
            log(f"[ui] view rewrapped (slot={self.shown_key[0]}, tab={self.shown_key[1]}, "
                f"messages={len(view.records)}, wrap={view.wrap}, ms={ms:.0f})")
        view.rewrap_start = None

    def _backfill_once(self) -> bool:
        """為顯示中分頁補畫一批（約 `_BACKFILL_BUDGET` 秒、至少一則），離視口近的先補；回傳是否還有待補。"""
        view = self._views[self.shown_key]
        records = view.records
        c = view.canvas
        middle = c.canvasy(c.winfo_height() / 2)
        start = time.perf_counter()
        drawn = 0
        while view.lo or view.hi < len(records):
            above = middle - line_origin(records[view.lo].lines[0])[1] if view.lo else math.inf
            below = _row_bottom(records[view.hi - 1]) - middle if view.hi < len(records) else math.inf
            if above <= below:
                self._draw_up(view)
            else:
                self._draw_down(view)
            drawn += 1
            elapsed = time.perf_counter() - start
            # 照本批平均每則耗時預估，再畫一則會超出預算就先停，而非超出後才停
            if elapsed + elapsed / drawn >= _BACKFILL_BUDGET:
                break
        if drawn:
            self._apply_region()
        return bool(view.lo or view.hi < len(records))

    def _estimate(self, m: _Message) -> float:
        """不排版的估計高度：多行訊息依換行寬度比例放大，單行與沒畫過的以兩種字型各一行計。"""
        if m.height <= self._one_line:
            return self._one_line
        return max(self._one_line, m.height * m.drawn_wrap / self.wrap)

    def _region(self) -> tuple[float, float]:
        """顯示中分頁的捲動範圍（上緣, 下緣）：已畫範圍加上兩側待補記錄的估計高度；沒有訊息時為 (0, 0)。"""
        view = self._views[self.shown_key]
        records = view.records
        if not records:
            return 0, 0
        above = sum(self._estimate(m) for m in records[:view.lo])
        below = sum(self._estimate(m) for m in records[view.hi:])
        return (line_origin(records[view.lo].lines[0])[1] - _ROW_PAD - above,
                _row_bottom(records[view.hi - 1]) + _ROW_PAD + below)

    def _apply_region(self) -> None:
        top, bottom = self._region()
        self._canvas.configure(scrollregion=(0, top, self._canvas.winfo_width(), bottom))

    def _fill_viewport(self, log_slow: bool = True) -> None:
        """視口上下緣離已畫範圍不足一個視口高度就同步補畫（呼叫端都在 Tk 重繪之前）。

        視口原本停在捲動範圍底端就畫到最新一則並貼底；停在頂端就畫到第一則並貼頂；落到內容上方也貼頂
        —— 估計高度與實際有落差，不貼的話會停在半途。log_slow＝耗時過久時記 log（重排另有自己的 log）。"""
        start = time.perf_counter()
        view = self._views[self.shown_key]
        before = view.hi - view.lo
        c = view.canvas
        at_top, at_bottom = c.yview()[0] <= 0, c.yview()[1] >= 1
        while view.records:
            height = c.winfo_height()
            top, bottom = c.canvasy(0) - height, c.canvasy(height) + height
            drew = False
            while view.lo and (at_top or line_origin(view.records[view.lo].lines[0])[1] > top):
                self._draw_up(view)
                drew = True
            while view.hi < len(view.records) and (
                    at_bottom or _row_bottom(view.records[view.hi - 1]) < bottom):
                self._draw_down(view)
                drew = True
            if not drew:
                break
            self._apply_region()
            if at_bottom:
                c.yview_moveto(1.0)
            elif view.lo == 0 and (at_top or c.canvasy(0) < self._region()[0]):
                c.yview_moveto(0.0)
        ms = (time.perf_counter() - start) * 1000
        drawn = view.hi - view.lo - before
        if log_slow and drawn and ms >= _SLOW_RELAYOUT_MS:
            log(f"[ui] viewport filled (slot={self.shown_key[0]}, tab={self.shown_key[1]}, "
                f"drawn={drawn}, ms={ms:.0f})")

    @staticmethod
    def _top_drawn(view: _View) -> int:
        """視口頂端那則已畫訊息的位置：列頂落在視口內的第一則（都不在時取最後一則已畫的）。"""
        # 不取被視口上緣切到的那則：它長高（譯文回填）時錨在它的列頂，視口內的內容就會整段下移
        i = bisect.bisect_left(view.records, view.canvas.canvasy(0), view.lo, view.hi,
                               key=lambda m: line_origin(m.lines[0])[1])
        return min(i, view.hi - 1)

    # --- 捲動 ---
    def _on_canvas_configure(self, e) -> None:
        self.wrap = max(80, e.width - 12)
        # 既有訊息的換行寬度也要同步更新，否則縮小視窗後右緣被切；隱藏中的分頁切過去時才補
        if self._views[self.shown_key].wrap != self.wrap:
            self._relayout()
        self._on_wrap_change(self.wrap)
        # 視窗拉高時視口變大：畫布重繪已排在 idle，補畫要趕在它之前
        self._fill_viewport()
        # 排到 idle 再貼底，不在事件處理中直接 update_idletasks() —— 那會讓下一個
        # Configure 事件重入本函式；此時排版也尚未完成，量到的高度是舊的。
        self._after_idle("refresh", self.refresh_scroll)

    def on_wheel(self, e) -> None:
        """滾輪捲動（畫布、捲軸與底板的空白處共用）。"""
        self._canvas.yview_scroll(int(-e.delta / 120), "units")
        self.note_scroll()
        self._fill_viewport()

    def _user_scroll(self, *args) -> None:
        """捲軸拖曳入口：捲完記下使用者是否仍要跟隨底部。"""
        self._canvas.yview(*args)
        self.note_scroll()
        self._fill_viewport()

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
        self._fill_viewport()
        return True

    def _view_anchor(self) -> tuple[str, int] | None:
        """視圖目前對齊到的內容位置：（視口頂端那則已畫訊息的 tag，它的列頂相對視口頂端的偏移）。

        畫布記的是像素原點而非「看到哪一則」，清掉上方舊訊息或改變某列高度時底下
        內容會整段滑動、正在讀的行就跳掉 —— 內容變動前取錨、變動後交給
        `refresh_scroll` 復位。跟隨底部時直接貼底不需要錨。拖曳框選中即使處於跟隨狀態
        也要給錨 —— prune 把上方訊息清掉會讓下方內容整段上移，沒有錨點補位的話游標下的字
        就會被換掉。"""
        if (self._follow and not self._selection.dragging) or not self._messages:
            return None
        view = self._views[self.shown_key]
        m = view.records[self._top_drawn(view)]
        return m.tag, round(line_origin(m.lines[0])[1]) - int(self._canvas.canvasy(0))

    def _restore_anchor(self, tag: str, offset: int) -> None:
        """把捲動位置移回「錨點那則仍在視口同一偏移」處（見 `_view_anchor`）；
        錨點那則已移除或未畫時改用第一則已畫訊息、偏移不變。"""
        view = self._views[self.shown_key]
        top, bottom = self._region()
        drawn = view.records[view.lo:view.hi]
        m = next((m for m in drawn if m.tag == tag), drawn[0] if drawn else None)
        if m is None or bottom <= top:
            # 罕見（訊息被清光、內容量不出來）：放棄補位，視圖會跳一下
            log(f"[ui] scroll anchor unusable, view may jump "
                f"(drawn={len(drawn)} region=({top}, {bottom})) "
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
        self._scroll_pending = None
        self._canvas.update_idletasks()
        self._apply_region()
        if self._follow and not self._selection.dragging:
            self._canvas.yview_moveto(1.0)
        elif anchor is not None:
            self._restore_anchor(*anchor)
        self._fill_viewport()

    # --- 訊息 ---
    def _drop(self, view: _View, m: _Message) -> None:
        """刪除一則訊息的文字物件（已畫時）並解除選取登記。兩者必須成對 —— 漏掉任一處，
        選取就會指向已刪除的物件。"""
        self._selection.forget(m.tag)
        view.rows.pop(m.tag, None)
        if m.lines:
            view.canvas.delete(*line_items(m.lines[0]), *line_items(m.lines[1]))

    def _draw(self, view: _View, i: int, top: float) -> None:
        """把第 i 則畫成列頂在 top 的原文行＋譯文行，更新記錄與選取登記並補上反白。"""
        c = view.canvas
        m = view.records[i]
        original = draw_outlined(c, 0, top, m.original,
                                 fill=dimmed(m.color) if m.color else FG_ORIGINAL,
                                 font=ui_font(9), wrap=self.wrap, tag=f"{m.tag}.0")
        translated = draw_outlined(c, 0, top + line_height(original), m.translated,
                                   fill=m.translated_fill(), font=ui_font(11),
                                   wrap=self.wrap, tag=f"{m.tag}.1")
        m = view.records[i] = m._replace(
            lines=(original, translated), drawn_wrap=self.wrap,
            height=line_height(original) + line_height(translated) + 2 * _ROW_PAD)
        view.rows[m.tag] = Row((m.original, m.translated), m.lines)
        self._selection.paint(m.tag)

    def _draw_above(self, view: _View, i: int, bottom: float) -> None:
        """把第 i 則畫成底緣在 bottom：先量高度再畫，文字物件畫下後就不再搬移。"""
        m = view.records[i]
        height = (measure_height(view.canvas, m.original, font=ui_font(9), wrap=self.wrap)
                  + measure_height(view.canvas, m.translated, font=ui_font(11), wrap=self.wrap))
        self._draw(view, i, bottom - height)

    def _draw_down(self, view: _View) -> None:
        """往下多畫一則，接在已畫範圍下緣。"""
        top = _row_bottom(view.records[view.hi - 1]) + 2 * _ROW_PAD if view.hi else _ROW_PAD
        self._draw(view, view.hi, top)
        view.hi += 1

    def _draw_up(self, view: _View) -> None:
        """往上多畫一則，接在已畫範圍上緣。"""
        self._draw_above(view, view.lo - 1,
                         line_origin(view.records[view.lo].lines[0])[1] - 2 * _ROW_PAD)
        view.lo -= 1

    def _append(self, view: _View, m: _Message) -> None:
        """記進清單並登記給選取（顯示中分頁的 rows 就是 Selection 的登記表）；
        最新一則已畫時才畫在最下方，否則等補畫輪到它。"""
        view.records.append(m)
        view.rows[m.tag] = Row((m.original, m.translated), ())
        if view.hi == len(view.records) - 1:
            self._draw_down(view)

    def _remove(self, view: _View, gone) -> None:
        """移除這些位置（由小到大）的記錄並維護已畫範圍。

        已畫的整段被移光時就地補畫一則：已畫範圍空了就沒有排版的基準點。"""
        top = None
        for i in reversed(gone):
            m = view.records.pop(i)
            if m.lines:
                top = line_origin(m.lines[0])[1]
            self._drop(view, m)
            if i < view.lo:
                view.lo -= 1
            if i < view.hi:
                view.hi -= 1
        if view.records and view.lo == view.hi:
            i = min(view.lo, len(view.records) - 1)
            self._draw(view, i, top)
            view.lo, view.hi = i, i + 1

    def _trim(self, view: _View) -> bool:
        """把超過上限的最舊記錄移除；回傳是否有移除。"""
        excess = len(view.records) - self._max
        if excess > 0:
            self._remove(view, range(excess))
        return excess > 0

    def add_message(self, key: ViewKey, original: str, translated: str,
                    now: float | None = None, msg_id: int | None = None,
                    pending: bool = False, color: str | None = None) -> None:
        """把一則訊息加到 key 那份清單最下方。pending＝譯文欄位目前是佔位字樣，以較暗的
        顏色標示，待 update_message 填入真正的譯文時才恢復正常顏色。
        color＝該則在遊戲內的顯示色：譯文直接用它、原文用調暗版；None 退回預設配色。"""
        shown = key == self.shown_key
        anchor = self._view_anchor() if shown else None
        view = self._view(key)
        # tag 全域唯一：Selection.forget 只認 tag，跨分頁不能撞名
        self._append(view, _Message(now if now is not None else time.time(), original,
                                    translated, msg_id, color, pending, False,
                                    tag=f"msg{next(self._tags)}"))
        self._trim(view)
        if shown:
            self._refresh_placeholder()
            if self._scroll_pending is None:
                self._scroll_pending = (anchor,)
                self._after_idle("flush", self._flush_scroll)

    def _flush_scroll(self) -> None:
        """執行 add_message 排好的那一次捲動更新；期間若已有人同步刷新過（含換分頁）就略過。"""
        if self._scroll_pending is not None:
            self.refresh_scroll(self._scroll_pending[0])

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
                view.rows[m.tag] = view.rows[m.tag]._replace(texts=(m.original, translated))
                if self._selection.holds(m.tag):
                    self._selection.clear("translated line replaced")
                delta = 0
                if m.lines:
                    line = m.lines[1]
                    before = line_height(line)
                    for item in line_items(line):
                        view.canvas.itemconfigure(item, text=translated)
                    view.canvas.itemconfigure(line.fg, fill=m.translated_fill())
                    delta = line_height(line) - before
                if delta:
                    view.records[i] = m._replace(height=m.height + delta)
                    for later in view.records[i + 1:view.hi]:
                        for later_line in later.lines:
                            _move_line(later_line, delta)
                if key == self.shown_key:
                    if delta:
                        self._selection.redraw()   # 反白矩形不在 line_items 裡，不會跟著搬
                    self.refresh_scroll(anchor)
                break

    def show(self, key: ViewKey) -> None:
        """換成顯示 key 那個分頁的畫布，捲動回到跟隨底部。

        已畫範圍含最新一則且換行寬度沒變就沿用並恢復背景補畫，否則從底部重排。"""
        if key == self.shown_key:
            return
        self._cancel_backfill()
        self._selection.clear("view switched")
        old = self._canvas
        old.pack_forget()
        old.configure(yscrollcommand="")
        self.shown_key = key
        view = self._view(key)
        self._attach(view)
        self._follow = True
        # 最新一則沒畫時貼底要把中間全部同步畫完，不如從底部重排
        if view.wrap != self.wrap or view.hi < len(view.records):
            view.rewrap_start = None   # 隱藏前沒補完的那輪作廢，ms 不該算進隱藏的時間
            self._relayout()
        else:
            self._schedule_backfill(_BACKFILL_DELAY_MS)
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
            gone = [i for i, m in enumerate(view.records) if m.ts <= cutoff]
            if not gone:
                continue
            self._remove(view, gone)
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
    def complete_layout(self) -> None:
        """把顯示中分頁剩下的訊息全部同步畫完（測試用）。"""
        view = self._views[self.shown_key]
        while view.lo:
            self._draw_up(view)
        while view.hi < len(view.records):
            self._draw_down(view)
        self._apply_region()

    def visible_messages(self) -> list[tuple[str, str]]:
        return [(m.original, m.translated) for m in self._messages]

    def placeholder_visible(self) -> bool:
        return self._placeholder.winfo_manager() == "place"
