"""疊加視窗：無邊框、置頂、半透明、可拖曳移動、可從四邊／四角縮放、可滾動。
顯示原文 + 譯文（最新在最下，可向上滾動看歷史）。
捲動定位：在底部時新訊息自動跟到最底；向上捲看歷史時不會被硬拉回底部。
不滑鼠穿透 —— 視窗蓋住的區域點擊不會傳到遊戲，視窗永遠可互動。"""
import time
import tkinter as tk
import webbrowser
from typing import NamedTuple

import win32con
import win32gui

from src.config import app_name
from src.i18n import t
from src.log import log
from src.ui.bubble import BUBBLE_SIZE, Bubble, bubble_alpha, should_auto_expand
from src.ui.fonts import ui_font
from src.ui.geometry import EDGE, edge_at, moved_to, point_in_rect, resized_edge
from src.ui.icons import load_icon
from src.ui.palette import (
    BAR,
    BG,
    BG_UPDATE,
    DIM_FACTOR,
    FG_BAR,
    FG_ERROR,
    FG_ORIGINAL,
    FG_PENDING,
    FG_TRANSLATED,
    FG_UPDATE,
    OUTLINE,
)
from src.ui.popup import Popup
from src.ui.selection import TEXT_ORIGIN, Selection
from src.ui.thin_scrollbar import ThinScrollbar
from src.ui.winstyle import enable_taskbar_button, make_non_activating, root_hwnd

# 狀態指示的顏色（文字由 i18n 依 state key 取得）
STATUS_COLORS = {
    "locating": "#e0b050",
    "listening": "#7dc87d",
    "translating": "#6fa8dc",
    "waiting_game": "#9a9aa8",
    "access_denied": FG_ERROR,  # 要使用者動手（以管理員重開）才解得掉，用錯誤色
    "version_mismatch": FG_ERROR,  # 同上：等下去也不會好，要更新遊戲或本程式
}

_OUTLINE_OFFSETS = ((-1, -1), (-1, 0), (-1, 1), (0, -1),
                    (0, 1), (1, -1), (1, 0), (1, 1))


def _fit_line_height(c: "tk.Canvas") -> None:
    """把文字行 canvas 的高度縮放到剛好容納（換行後的）文字內容。"""
    bbox = c.bbox("txt")   # 只量文字項目：這樣列上其他畫的東西（反白矩形）永遠不會影響列高
    if bbox:
        c.configure(height=bbox[3] + 2)


def _outlined_line(parent, text: str, fg: str, font: tuple, wrap: int) -> "tk.Canvas":
    """字幕式描邊文字行：canvas 先畫八方向 1px 偏移的描邊副本、再疊本色——
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
    """把 `#rrggbb` 各通道乘上係數調暗——原文行用遊戲色的暗版，維持原文暗、譯文亮的層次。"""
    r, g, b = (int(color[i:i + 2], 16) for i in (1, 3, 5))
    return f"#{round(r * factor):02x}{round(g * factor):02x}{round(b * factor):02x}"


class _Message(NamedTuple):
    """overlay 中的一則訊息。msg_id 為 None 代表不需要就地更新（例如測試直接塞完成品）；
    color 為該則在遊戲內的顯示色（None＝退回預設配色）。"""
    ts: float
    original: str
    translated: str
    row: "tk.Frame"
    msg_id: int | None
    color: str | None = None


MIN_WIDTH = 200
MIN_HEIGHT = 90
_BAR_HEIGHT = 20
_BAR_ICON = 16   # 標題列 icon：留 2px 上下邊給 _BAR_HEIGHT，且是 icon.ico 的原生尺寸
# 標題列文字下推的補償：Label 垂直置中按字型 linespace（含 descent）算，中文與數字
# 沒有下伸部、墨跡整體偏上，與精準置中的 icon 並排就對不齊。實測補 1px 最接近
# （剩下的 0.5px 是墨跡高度奇偶不匹配，補不掉）。
_BAR_TEXT_NUDGE = 1
_GRIP_SIZE = 16
_STICK_THRESHOLD = 0.999
_FOREGROUND_POLL_MS = 300
_EDGE_CURSORS = {"n": "size_ns", "s": "size_ns", "w": "size_we", "e": "size_we",
                 "nw": "size_nw_se", "se": "size_nw_se",
                 "ne": "size_ne_sw", "sw": "size_ne_sw"}


def should_stick_to_bottom(view_bottom_fraction: float,
                           threshold: float = _STICK_THRESHOLD) -> bool:
    """視圖底緣接近最底時，新訊息應自動跟到底；使用者往上捲時則否。"""
    return view_bottom_fraction >= threshold


class OverlayWindow:
    def __init__(self, root: tk.Tk, x: int | None, y: int | None,
                 width: int = 640, height: int = 420,
                 max_messages: int = 50, fade_seconds: int = 180,
                 on_geometry_change=None, on_settings=None, on_close=None,
                 bubble_position: dict | None = None, on_bubble_move=None,
                 alpha: float = 0.80):
        self._max = max_messages
        self._fade = fade_seconds
        self._alpha = alpha
        self._on_geometry_change = on_geometry_change
        self._on_bubble_move = on_bubble_move
        self._on_close = on_close   # 泡泡建立時要拿它接 WM_DELETE_WINDOW
        self._bubble_pos = dict(bubble_position) if bubble_position else {"x": None, "y": None}
        self._minimized = False
        self._unread = 0
        self._bubble: Bubble | None = None
        self._prev_foreground = 0
        self._watch_job: str | None = None
        self._messages: list[_Message] = []
        # 視圖是否黏在底部。只在使用者主動捲動時重新評估：縮放視窗／橫幅進出也會把視圖
        # 推離底部，每次加訊息時當場採樣會誤判成「使用者往上捲」。
        self._follow = True
        self._error_label: tk.Label | None = None
        self._status_state: str | None = None   # 目前狀態的 key，語言切換後重繪用
        self._error_key: str | None = None      # 目前橫幅的 key，同上
        self._update_row: tk.Frame | None = None
        self._update_label: tk.Label | None = None
        self._update_release = None   # 目前橫幅對應的 Release，語言切換後重繪用
        self._w = max(width, MIN_WIDTH)
        self._h = max(height, MIN_HEIGHT)
        self._wrap = self._w - 40
        self._drag = (0, 0, 0, 0)
        # 縮放中的起點與起始幾何；None＝目前沒有在縮放（見 _resize_start）
        self._resize: tuple[int, int, int, int, int, int, str] | None = None

        self._build_backdrop(root)
        # master 用 root 而非 backdrop：Tk 的 master 連動 restack 會在點擊本體時
        # 把 backdrop 一起抬起、反而蓋過文字層（實測）；OS 擁有關係於下方另設。
        self._win = tk.Toplevel(root)
        self._win.overrideredirect(True)
        self._win.attributes("-topmost", True)
        self._win.attributes("-transparentcolor", BG)
        self._win.configure(bg=BG)
        # 未設定過位置（首次啟動）：擺螢幕正中央，比擺角落更容易被注意到
        px = x if x is not None else (self._win.winfo_screenwidth() - self._w) // 2
        py = y if y is not None else (self._win.winfo_screenheight() - self._h) // 2
        self._apply_geometry(px, py, self._w, self._h)
        self._build_title_bar(on_settings, on_close)
        self._build_message_area()
        self._build_resize_handles()
        self._attach_to_shell(on_close)

    def _build_backdrop(self, root: tk.Tk) -> None:
        """半透明底板：雙層視窗的下層（見內文）。"""
        # 雙層視窗：tk 的 -alpha 整窗生效、無法只透背景，故拆兩層——下層 backdrop 承擔
        # 半透明底板（透明度設定作用於此），上層本體以 -transparentcolor 挖空背景色，
        # 文字與控制項保持完全不透明。本體由 backdrop 擁有（owned window），永遠疊在其上。
        self._backdrop = tk.Toplevel(root)
        self._backdrop.overrideredirect(True)
        self._backdrop.attributes("-topmost", True)
        self._backdrop.attributes("-alpha", self._alpha)
        self._backdrop.configure(bg=BG)
        # 底板攔截透明背景區的滑鼠事件（不穿透到遊戲），但點擊不奪焦點、不改疊序；
        # 空白處的滾輪由底板轉發給訊息區捲動。
        make_non_activating(self._backdrop)
        self._backdrop.bind("<MouseWheel>", self._on_wheel)

    def _build_title_bar(self, on_settings, on_close) -> None:
        """標題列：icon、標題、狀態字與 ⚙／─／✕，整列可拖曳移動、上緣可縮放。"""
        # 標題列（可拖曳移動）
        bar = tk.Frame(self._win, bg=BAR, height=_BAR_HEIGHT, cursor="fleur")
        bar.pack(side="top", fill="x")
        bar.pack_propagate(False)
        # 不另放拖曳把手符號，可拖曳的暗示交給 bar 的 fleur 游標
        self._bar_icon = load_icon(self._win, _BAR_ICON)
        self._title_label = tk.Label(bar, text=app_name(), bg=BAR, fg=FG_BAR,
                                     font=ui_font(8), anchor="w")
        # side="right" 先 pack 者占最外側：由右到左為 ✕、⚙、狀態字。打包版沒有主控台，
        # ✕ 是唯一的正常關閉途徑，所以整組控制項都排在標題之前 pack——標題再長
        # 或視窗再窄，被裁掉的只會是標題。
        if on_close is not None:
            close = tk.Label(bar, text="✕", bg=BAR, fg=FG_BAR,
                             font=ui_font(9), cursor="hand2")
            close.pack(side="right", padx=(0, 6))
            close.bind("<Button-1>", lambda e: on_close())
        mini = tk.Label(bar, text="─", bg=BAR, fg=FG_BAR,
                        font=ui_font(9), cursor="hand2")
        mini.pack(side="right", padx=(0, 4))
        mini.bind("<Button-1>", lambda e: self.minimize())
        if on_settings is not None:
            gear = tk.Label(bar, text="⚙", bg=BAR, fg=FG_BAR,
                            font=ui_font(9), cursor="hand2")
            gear.pack(side="right", padx=(0, 4))
            gear.bind("<Button-1>", lambda e: on_settings())
        self._status_label = tk.Label(bar, text="", bg=BAR, fg=FG_BAR,
                                      font=ui_font(8), anchor="e")
        self._status_label.pack(side="right", padx=6, pady=(_BAR_TEXT_NUDGE, 0))
        draggable = [bar, self._title_label, self._status_label]
        if self._bar_icon is not None:
            bar_icon = tk.Label(bar, image=self._bar_icon, bg=BAR)
            bar_icon.pack(side="left", padx=(6, 4))
            draggable.append(bar_icon)
        self._title_label.pack(side="left", padx=(0 if self._bar_icon else 6, 6),
                               pady=(_BAR_TEXT_NUDGE, 0))
        for w in draggable:
            w.bind("<Motion>", lambda e: self._edge_motion(e, "fleur"))
            w.bind("<ButtonPress-1>", self._bar_press)
            w.bind("<B1-Motion>", self._bar_drag)
            w.bind("<ButtonRelease-1>", self._bar_release)

    def _build_message_area(self) -> None:
        """內容區：可捲動的訊息列表、細捲軸、空狀態提示（橫幅事後才 pack 進來）。"""
        # 先建 _selection：本方法稍後綁定的 <Configure> 可能在事件迴圈中提早觸發
        # _on_canvas_configure（其內會呼叫 self._selection.redraw()），屬性要先存在。
        self._selection = Selection()
        # 內容區：錯誤橫幅（固定在下，不隨捲動）+ 可滾動訊息區
        self._frame = tk.Frame(self._win, bg=BG)
        self._frame.pack(side="top", fill="both", expand=True)

        scroll_area = tk.Frame(self._frame, bg=BG)
        scroll_area.pack(side="top", fill="both", expand=True)
        # 錯誤橫幅是事後才建立的，pack 時要指名排在捲動區之前（見 set_error）
        self._scroll_area = scroll_area
        self._canvas = tk.Canvas(scroll_area, bg=BG, highlightthickness=0)
        self._scrollbar = ThinScrollbar(scroll_area, command=self._user_scroll)
        self._scrollbar.bind("<MouseWheel>", self._on_wheel)  # 游標壓在捲軸上也能滾
        self._canvas.configure(yscrollcommand=self._scrollbar.set)
        # 底部讓出縮放把手的高度：把手 place 在視窗右下角，捲軸鋪到底會被它壓住
        self._scrollbar.pack(side="right", fill="y", padx=(0, EDGE),
                             pady=(0, _GRIP_SIZE))
        self._canvas.pack(side="left", fill="both", expand=True)
        self._inner = tk.Frame(self._canvas, bg=BG)
        self._inner_id = self._canvas.create_window((0, 0), window=self._inner, anchor="nw")
        self._inner.bind(
            "<Configure>",
            lambda e: self._canvas.configure(scrollregion=self._canvas.bbox("all")),
        )
        self._canvas.bind("<Configure>", self._on_canvas_configure)
        self._canvas.bind("<Enter>", lambda e: self._canvas.bind_all("<MouseWheel>", self._on_wheel))
        self._canvas.bind("<Leave>", lambda e: self._canvas.unbind_all("<MouseWheel>"))

        # 空狀態提示：沒有訊息時把目前狀態大字置中顯示
        self._placeholder = tk.Label(scroll_area, text="", bg=BG, fg=FG_BAR,
                                     font=ui_font(11))
        self._placeholder.place(relx=0.5, rely=0.5, anchor="center")

    def _build_resize_handles(self) -> None:
        """右下角把手與四邊縮放的事件接線。"""
        # 右下角縮放把手只畫斜線、背景留透明色鍵，才不會在遊戲畫面上多出一塊方形。
        # 代價是只有線條的實心像素接得到滑鼠（透明色鍵像素在 Windows 下點不穿），
        # 所以線畫粗一點撐回抓取範圍；亮色配描邊，任何背景上都有對比。
        grip = tk.Canvas(self._win, width=_GRIP_SIZE, height=_GRIP_SIZE, bg=BG,
                         highlightthickness=0, bd=0, cursor="size_nw_se")
        for inset in (4, 9, 14):
            ends = (_GRIP_SIZE - inset, _GRIP_SIZE - 2, _GRIP_SIZE - 2, _GRIP_SIZE - inset)
            grip.create_line(*ends, fill=OUTLINE, width=4)
            grip.create_line(*ends, fill=FG_BAR, width=2)
        grip.place(relx=1.0, rely=1.0, anchor="se")
        grip.bind("<ButtonPress-1>", lambda e: self._resize_start(e, "se"))
        grip.bind("<B1-Motion>", self._edge_drag)
        grip.bind("<ButtonRelease-1>", self._edge_release)

        # 四邊縮放：本體的透明背景像素點得穿，左／右／下三邊與下方兩角的事件落到底板上。
        # 上緣被標題列擋住，由標題列自己的按下事件分流（見 _bar_press）——在標題列上
        # 鋪感應細條會蓋掉標題文字的頂端。
        self._backdrop.bind("<Motion>", self._edge_motion)
        self._backdrop.bind("<ButtonPress-1>", self._edge_press)
        self._backdrop.bind("<B1-Motion>", self._edge_drag)
        self._backdrop.bind("<ButtonRelease-1>", self._edge_release)

    def _attach_to_shell(self, on_close) -> None:
        """OS 層的收尾：owner 關係、工作列按鈕、關閉協定、疊序。"""
        self._win.title(app_name())  # 工作列按鈕顯示的名稱
        # 用 Win32 直接建立 OS 擁有關係：owned window 永遠疊在 owner 之上，點擊／啟用
        # 都不會反轉（Tk 的 master 參數實測不會設定 GW_OWNER）。
        # 必須排在 enable_taskbar_button 之前：改 owner 會讓 shell 撤掉已建好的工作列
        # 按鈕，而 WS_EX_APPWINDOW 要等下一次 hide→show 才重新生效——順序反過來，
        # 首次啟動就沒有工作列按鈕。
        try:
            self._win.update_idletasks()
            win32gui.SetWindowLong(root_hwnd(self._win), win32con.GWL_HWNDPARENT,
                                   root_hwnd(self._backdrop))
        except Exception as exc:
            log(f"[ui] owner setup failed: {exc}")
        enable_taskbar_button(self._win)  # 文字層不透明，不需重設 alpha
        # 有工作列按鈕就關得掉（Alt+F4、工作列右鍵都送 WM_DELETE_WINDOW）：tkinter 預設
        # 接成 destroy 這個 Toplevel，只拆掉文字層、留下 backdrop 孤兒，主迴圈還照跑噴錯。
        # 接回 ✕ 的乾淨關閉，兩條路徑才一致。
        if on_close is not None:   # None（測試直接建視窗）時維持 Tk 預設行為
            self._win.protocol("WM_DELETE_WINDOW", on_close)
        self._backdrop.lower(self._win)  # 疊序保險：底板壓在文字層之下
        # Caps Lock 開著時 Tk 送的是 <Control-C>，兩個都要接。不用 bind_all——
        # 那會連設定視窗的輸入框一起攔截。
        self._win.bind("<Control-c>", self.copy_selection)
        self._win.bind("<Control-C>", self.copy_selection)
        # 右鍵與左鍵一樣要雙路由：選取區以外的空白處按右鍵，事件會穿透到 backdrop
        self._backdrop.bind("<Button-3>", self._selection_menu)
        self._popup = Popup(self._win, self.copy_selection)
        # Esc 綁在本體而非選單上：選單刻意不取鍵盤焦點（取走的話 Ctrl+C 就收不到），
        # 使用者按 Esc 時焦點在本體上（框選起手時 _focus_for_copy 已經拿過來了）。
        self._win.bind("<Escape>", lambda e: self._popup.hide())

    # --- 縮小成泡泡 ---
    @property
    def minimized(self) -> bool:
        """是否處於縮小（泡泡）狀態。"""
        return self._minimized

    def unread_count(self) -> int:
        """縮小期間累積的未讀訊息數（展開歸零）。"""
        return self._unread

    def minimize(self) -> None:
        """縮小成浮動泡泡：隱藏本體（訊息照常累積），點泡泡展開、拖曳移動。"""
        if self._minimized:
            return
        self._selection.clear("minimized to bubble")
        self._popup.hide()
        self._win.update_idletasks()
        if self._bubble_pos.get("x") is None:
            # 無記憶位置：預設出現在 overlay 右上角（縮小按鈕附近），視覺上「收進泡泡」
            self._bubble_pos = {
                "x": self._win.winfo_x() + self._win.winfo_width() - BUBBLE_SIZE,
                "y": self._win.winfo_y(),
            }
        self._minimized = True
        self._unread = 0
        self._win.withdraw()
        self._backdrop.withdraw()
        self._bubble = Bubble(self._win, self._bubble_pos["x"], self._bubble_pos["y"],
                              bubble_alpha(self._alpha), on_click=self.expand,
                              on_move=self._bubble_moved, on_close=self._on_close)
        # 泡泡剛建立時可能已經是前景（enable_taskbar_button 的 deiconify 會啟用它），
        # 拿當下的前景當基準才不會第一輪就誤判成「使用者切回本工具」
        self._prev_foreground = self._foreground_window()
        self._watch_job = self._win.after(_FOREGROUND_POLL_MS, self._watch_foreground)
        log(f"[ui] minimized to bubble ({self._scroll_debug()})")

    def expand(self) -> None:
        """從泡泡展開回完整視窗，未讀歸零。"""
        if not self._minimized:
            return
        if self._watch_job is not None:
            self._win.after_cancel(self._watch_job)
            self._watch_job = None
        self._minimized = False
        self._unread = 0
        if self._bubble is not None:
            self._bubble.destroy()
            self._bubble = None
        self._backdrop.deiconify()
        self._backdrop.attributes("-topmost", True)
        self._backdrop.attributes("-alpha", self._alpha)
        self._win.deiconify()
        self._win.attributes("-topmost", True)
        self._backdrop.lower(self._win)  # 疊序保險：底板永遠壓在文字層之下
        # 重新對齊排到 idle：deiconify 只是送出顯示要求，這一行執行時畫布仍是隱藏的，
        # 當場重算會量到（也擺放到）舊值。等 map 真的完成後再做，才對得齊。
        self._win.after_idle(self._settle_after_expand)

    def _settle_after_expand(self) -> None:
        """視窗重新顯示後接回捲動狀態：泡泡期間的訊息是在隱藏狀態下排版的，必須自己
        重算——deiconify 幾何沒變就不會帶來 <Configure>。"""
        self._refresh_scroll()
        log(f"[ui] expanded ({self._scroll_debug()})")

    def _foreground_window(self) -> int:
        try:
            return win32gui.GetForegroundWindow()
        except Exception as exc:
            log(f"[ui] foreground lookup failed: {exc}")
            return 0

    def _watch_foreground(self) -> None:
        """泡泡狀態下輪詢前景視窗，本工具被切回前景就展開。
        overrideredirect 視窗不是 OS 意義上的最小化，收不到還原通知，
        點工作列按鈕／Alt+Tab 只會把泡泡切成前景，只能自己輪詢察覺。"""
        self._watch_job = None
        if not self._minimized or self._bubble is None:
            return
        fg = self._foreground_window()
        expand = should_auto_expand(fg, self._prev_foreground, self._bubble.hwnd,
                                    self._bubble.pointer_over())
        self._prev_foreground = fg
        if expand:
            log(f"[ui] auto-expand: bubble brought to foreground, fg=0x{fg:x}")
            self.expand()
            return
        self._watch_job = self._win.after(_FOREGROUND_POLL_MS, self._watch_foreground)

    def _bubble_moved(self, x: int, y: int) -> None:
        self._bubble_pos = {"x": x, "y": y}
        if self._on_bubble_move is not None:
            self._on_bubble_move(x, y)

    def set_alpha(self, alpha: float) -> None:
        """套用新的視窗不透明度（半透明底板與泡泡即時生效；文字層恆為不透明）。"""
        self._alpha = alpha
        self._backdrop.attributes("-alpha", alpha)
        if self._bubble is not None:
            self._bubble.attributes("-alpha", bubble_alpha(alpha))

    # --- 幾何 ---
    def _apply_geometry(self, x: int, y: int, w: int, h: int) -> None:
        self._w, self._h = w, h
        self._wrap = w - 40
        self._win.geometry(f"{w}x{h}+{x}+{y}")
        self._backdrop.geometry(f"{w}x{h}+{x}+{y}")

    def _emit_geometry(self) -> None:
        if self._on_geometry_change is None:
            return
        self._win.update_idletasks()
        self._on_geometry_change(self._win.winfo_x(), self._win.winfo_y(),
                                 self._win.winfo_width(), self._win.winfo_height())

    def _on_canvas_configure(self, e) -> None:
        # 內層寬度跟著畫布寬，文字才會依視窗寬換行
        self._canvas.itemconfigure(self._inner_id, width=e.width)
        self._wrap = max(80, e.width - 12)
        # 既有訊息與錯誤橫幅的換行寬度也要同步更新，否則縮小視窗後右緣被切
        for entry in self._messages:
            for child in entry.row.winfo_children():
                child.itemconfigure("txt", width=self._wrap)
                _fit_line_height(child)
        self._selection.redraw()
        if self._error_label is not None:
            self._error_label.configure(wraplength=self._wrap)
        if self._update_label is not None:
            self._update_label.configure(wraplength=self._wrap)
        # 排到 idle 再貼底，不在事件處理中直接 update_idletasks()——那會讓下一個
        # Configure 事件重入本函式；此時排版也尚未完成，量到的高度是舊的。
        self._win.after_idle(self._refresh_scroll)

    def _on_wheel(self, e) -> None:
        self._canvas.yview_scroll(int(-e.delta / 120), "units")
        self._note_scroll()

    def _user_scroll(self, *args) -> None:
        """捲軸拖曳入口：捲完記下使用者是否仍要跟隨底部。"""
        self._canvas.yview(*args)
        self._note_scroll()

    def _scroll_debug(self) -> str:
        """捲動狀態快照，供 log 定位「訊息看不到／捲不到底」；region_h 與 content_h
        對不上就代表捲動範圍是舊的。"""
        region = self._canvas.cget("scrollregion").split()
        bbox = self._canvas.bbox("all")
        return (f"messages={len(self._messages)} follow={self._follow} "
                f"yview_bottom={self._canvas.yview()[1]:.4f} "
                f"region_h={region[3] if len(region) == 4 else '?'} "
                f"content_h={bbox[3] if bbox else '?'} "
                f"canvas_h={self._canvas.winfo_height()}")

    def _note_scroll(self) -> None:
        """使用者主動捲動後重新判定是否繼續跟隨底部：往上捲＝正在讀歷史，
        新訊息不該把畫面搶走；捲回底部則恢復跟隨。"""
        follow = should_stick_to_bottom(self._canvas.yview()[1])
        if follow != self._follow:
            self._follow = follow
            log(f"[ui] auto-follow {'enabled' if follow else 'disabled'} "
                f"by user scroll ({self._scroll_debug()})")

    def _view_anchor(self) -> tuple["tk.Misc", int] | None:
        """視圖目前對齊到的內容位置：(最新一則的列, 它相對視口頂端的偏移)。

        畫布記的是像素原點而非「看到哪一則」，清掉上方舊訊息或改變某列高度時底下
        內容會整段滑動、正在讀的行就跳掉——內容變動前取錨、變動後交給
        `_refresh_scroll` 復位。跟隨底部時直接貼底不需要錨；最新一則不會被上方的
        清除移走，拿它當錨最穩。拖曳框選中即使處於跟隨狀態也要給錨——prune 把上方
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
                f"({self._scroll_debug()})")
            return
        target = max(bbox[1], row.winfo_y() - offset)
        self._canvas.yview_moveto((target - bbox[1]) / (bbox[3] - bbox[1]))

    def _refresh_scroll(self, anchor: tuple["tk.Misc", int] | None = None) -> None:
        """重算捲動範圍；跟隨模式貼回底部，否則依 anchor 維持原位。

        任何改變畫布內容或幾何的動作都要呼叫（新增／更新／清除訊息、縮放、橫幅進出）。
        漏呼叫的後果是視圖從此停在舊位置——`_follow` 仍為真卻沒人貼底，之後每則
        新訊息都落在畫面外，看起來就像訊息漏掉了。
        框選拖曳期間不貼底：畫面被新訊息拉走的話，游標下的字會整個換掉。"""
        self._canvas.update_idletasks()
        # 視窗隱藏（縮成泡泡、工作列收合）期間畫布不重繪，內嵌容器與捲動帳目脫節——
        # yview 回報已在底部，畫面卻少了最後幾則、往下也捲不動。重設一次座標（值不變）
        # 即可要求畫布重新擺放它。
        self._canvas.coords(self._inner_id, 0, 0)
        self._canvas.configure(scrollregion=self._canvas.bbox("all"))
        if self._follow and not self._selection.dragging:
            self._canvas.yview_moveto(1.0)
        elif anchor is not None:
            self._restore_anchor(*anchor)

    def _move_start(self, e) -> None:
        self._drag = (e.x_root, e.y_root, self._win.winfo_x(), self._win.winfo_y())

    def _move_drag(self, e) -> None:
        sx, sy, ox, oy = self._drag
        nx, ny = moved_to(ox, oy, e.x_root - sx, e.y_root - sy)
        self._win.geometry(f"{self._w}x{self._h}+{nx}+{ny}")
        self._backdrop.geometry(f"{self._w}x{self._h}+{nx}+{ny}")

    def _edge_under(self, e) -> str:
        """游標（螢幕座標）目前壓在視窗的哪條邊／哪個角。"""
        return edge_at(e.x_root, e.y_root, self._win.winfo_x(), self._win.winfo_y(),
                       self._w, self._h)

    def _edge_motion(self, e, default: str = "") -> None:
        e.widget.configure(cursor=_EDGE_CURSORS.get(self._edge_under(e), default))

    def _edge_press(self, e) -> None:
        edge = self._edge_under(e)
        if edge:
            self._resize_start(e, edge)
        else:
            self._selection_press(e)   # 透明背景區的點擊落到這裡，交給框選

    def _bar_press(self, e) -> None:
        """標題列按下：壓在上緣（含上方兩角）＝縮放，其餘＝拖曳移動。"""
        edge = self._edge_under(e)
        if edge:
            self._resize_start(e, edge)
        else:
            self._move_start(e)

    def _bar_drag(self, e) -> None:
        if self._resize is not None:
            self._edge_drag(e)
        else:
            self._move_drag(e)

    def _bar_release(self, e) -> None:
        if self._resize is not None:
            self._edge_release(e)
        else:
            self._emit_geometry()

    def _resize_start(self, e, edge: str) -> None:
        """記下拖曳起點與起始幾何：拖曳期間一律以起點換算，避免逐次累加的誤差。"""
        self._resize = (e.x_root, e.y_root, self._win.winfo_x(), self._win.winfo_y(),
                        self._w, self._h, edge)
        log(f"[ui] overlay resize start edge={edge} geometry="
            f"{self._w}x{self._h}+{self._win.winfo_x()}+{self._win.winfo_y()}")

    def _edge_drag(self, e) -> None:
        if self._resize is None:
            self._selection_drag(e)   # 這次按下不在邊上：可能正在框選
            return
        sx, sy, ox, oy, ow, oh, edge = self._resize
        self._apply_geometry(*resized_edge(edge, ox, oy, ow, oh,
                                           e.x_root - sx, e.y_root - sy,
                                           MIN_WIDTH, MIN_HEIGHT))

    def _edge_release(self, e) -> None:
        if self._resize is None:
            self._selection_release(e)
            return
        edge = self._resize[6]
        self._resize = None
        log(f"[ui] overlay resize end edge={edge} geometry="
            f"{self._w}x{self._h}+{self._win.winfo_x()}+{self._win.winfo_y()}")
        self._emit_geometry()

    # --- 框選 ---
    def _in_message_area(self, x_root: int, y_root: int) -> bool:
        """螢幕座標是否落在可捲動的訊息視口內。捲出視野的列仍有幾何位置，不先擋一道
        的話，點在把手或標題列附近會選到看不見的訊息。"""
        c = self._canvas
        return point_in_rect(x_root, y_root, c.winfo_rootx(), c.winfo_rooty(),
                             c.winfo_width(), c.winfo_height())

    def _selection_press(self, e) -> None:
        """框選起手。訊息列的底色是本體的透明色鍵，只有文字墨跡接得到滑鼠、其餘落到
        backdrop，兩條路徑都導進這裡，一律用螢幕座標。"""
        self._popup.hide()   # 點到疊加視窗任何一處就收起選單（取代原生選單的 grab）
        if not self._in_message_area(e.x_root, e.y_root):
            self._selection.clear("press outside the message area")
            return
        if self._selection.begin(e.x_root, e.y_root):
            self._focus_for_copy()

    def _selection_drag(self, e) -> None:
        self._selection.extend(e.x_root, e.y_root)

    def _selection_release(self, e) -> None:
        self._selection.finish()

    def _focus_for_copy(self) -> None:
        """把鍵盤焦點交給本體，Ctrl+C 才收得到——backdrop 帶 WS_EX_NOACTIVATE，
        從它起手的選取不會給焦點。"""
        try:
            log("[ui] forcing keyboard focus")
            self._win.focus_force()
            log("[ui] selection took keyboard focus")
        except tk.TclError as exc:
            log(f"[ui] selection focus failed: {exc}")

    def copy_selection(self, _event=None) -> None:
        """把目前選取的文字寫進系統剪貼簿。沒有選取就什麼都不做——寫入空字串會把
        使用者原本的剪貼簿內容清掉。"""
        text = self._selection.text()
        if not text:
            return
        self._win.clipboard_clear()
        self._win.clipboard_append(text)
        # Windows 下要 flush 過，內容才真的落進系統剪貼簿；這會連帶清空 after 佇列，
        # add_message／prune 可能在這裡重入執行，但 text 已存成區域變數，無害
        self._win.update()
        log(f"[ui] copied selection chars={len(text)}")

    def _selection_menu(self, e) -> None:
        """有選取時才彈出右鍵選單；沒選取就不彈，不做灰掉的空選單。
        文字每次現取，介面語言換了就跟著換。"""
        if not self._selection.active:
            return
        self._popup.show(e.x_root, e.y_root, t("menu.copy"))

    # --- 訊息 ---
    def _drop_row(self, entry: _Message) -> None:
        """移除一則訊息的畫面元件。銷毀列與解除選取登記必須成對——漏掉任一處，
        選取就會指向已銷毀的 widget。"""
        self._selection.forget(entry.row)
        entry.row.destroy()

    def add_message(self, original: str, translated: str, now: float | None = None,
                    msg_id: int | None = None, pending: bool = False,
                    color: str | None = None) -> None:
        """加入一則訊息。pending＝譯文欄位目前是佔位字樣，以較暗的顏色標示，
        待 update_message 填入真正的譯文時才恢復正常顏色。
        color＝該則在遊戲內的顯示色：譯文直接用它、原文用調暗版；None 退回預設配色。"""
        anchor = self._view_anchor()
        row = tk.Frame(self._inner, bg=BG)
        original_line = _outlined_line(row, original,
                                       dimmed(color) if color else FG_ORIGINAL,
                                       ui_font(9), self._wrap)
        original_line.pack(fill="x")
        translated_line = _outlined_line(row, translated,
                                         FG_PENDING if pending else (color or FG_TRANSLATED),
                                         ui_font(11), self._wrap)
        translated_line.pack(fill="x")
        row.pack(side="top", fill="x", pady=2)  # 最新在最下
        self._selection.register(row, (original_line, translated_line))
        for line in (original_line, translated_line):
            line.bind("<ButtonPress-1>", self._selection_press)
            line.bind("<B1-Motion>", self._selection_drag)
            line.bind("<ButtonRelease-1>", self._selection_release)
            line.bind("<Button-3>", self._selection_menu)
        self._messages.append(_Message(now if now is not None else time.time(),
                                       original, translated, row, msg_id, color))
        while len(self._messages) > self._max:
            self._drop_row(self._messages.pop(0))

        self._refresh_placeholder()
        self._refresh_scroll(anchor)
        if self._minimized and self._bubble is not None:
            self._unread += 1
            self._bubble.set_unread(self._unread)

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
            self._refresh_scroll(anchor)
            return

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
            self._refresh_scroll(anchor)

    def prune(self, now: float | None = None) -> None:
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
        self._refresh_scroll(anchor)

    def set_status(self, state: str) -> None:
        """更新狀態指示：標題列右側小字；視窗還沒有任何訊息時，同步大字置中顯示。
        存的是 state key 而非文字——語言切換後 refresh_labels() 才能重新翻譯。"""
        self._status_state = state
        text, color = t(f"status.{state}"), STATUS_COLORS[state]
        self._status_label.configure(text=text, fg=color)
        self._placeholder.configure(text=text, fg=color)
        self._refresh_placeholder()

    def _refresh_placeholder(self) -> None:
        if self._messages:
            self._placeholder.place_forget()
        else:
            self._placeholder.place(relx=0.5, rely=0.5, anchor="center")
            self._placeholder.lift()

    def set_error(self, key: str) -> None:
        """顯示錯誤橫幅（傳入文案 key，顯示時才翻譯）。"""
        self.clear_error()
        self._error_key = key
        # justify＝換行後每一行都靠左：tk.Label 多行預設置中，anchor="w" 只擺放整塊
        # 文字、管不到行內對齊，較長的橫幅（如版本不相容）換行後會歪成階梯狀。
        self._error_label = tk.Label(self._frame, text=t(key), bg=BG, fg=FG_ERROR,
                                     font=ui_font(10, "bold"), anchor="w",
                                     justify="left", wraplength=self._wrap)
        # before＝捲動區：pack 依宣告順序分配空間，橫幅排在 expand=True 的捲動區
        # 之後就會在視窗被縮小時被擠掉——而「遊戲未就緒」正是最該看到的訊息。
        self._error_label.pack(side="bottom", fill="x", pady=2,
                               before=self._scroll_area)

    def clear_error(self) -> None:
        self._error_key = None
        if self._error_label is not None:
            self._error_label.destroy()
            self._error_label = None

    def set_update(self, release) -> None:
        """顯示更新橫幅：整列可點（開瀏覽器到下載頁），右側 ✕ 只關掉這一次。
        與錯誤橫幅各佔一列、互不覆蓋——兩者可能同時該被看到。release 存起來，
        換語言時才重繪得出來。"""
        self.clear_update()
        self._update_release = release
        row = tk.Frame(self._frame, bg=BG_UPDATE)
        close = tk.Label(row, text="✕", bg=BG_UPDATE, fg=FG_UPDATE, font=ui_font(9),
                         cursor="hand2")
        # ✕ 先 pack：expand=True 的文字若先宣告會吃光整列寬度，把它擠出畫面
        close.pack(side="right", padx=(4, 6))
        close.bind("<Button-1>", lambda e: self._dismiss_update())
        label = tk.Label(row, text=t("update.available", version=release.version),
                         bg=BG_UPDATE, fg=FG_UPDATE, font=ui_font(10, "bold"),
                         anchor="w", justify="left", cursor="hand2",
                         wraplength=self._wrap)
        label.pack(side="left", fill="x", expand=True)
        label.bind("<Button-1>", lambda e: self._open_update_link(release))
        # before＝捲動區：同錯誤橫幅。右側讓出縮放把手的寬度：把手底色是透明色鍵，
        # 疊在這條不透明橫幅上會挖出缺口並蓋掉半個 ✕（捲軸讓出把手高度同理）。
        row.pack(side="bottom", fill="x", pady=2, padx=(0, _GRIP_SIZE),
                 before=self._scroll_area)
        self._update_row = row
        self._update_label = label
        log(f"[update] banner shown for {release.version}")

    def _open_update_link(self, release) -> None:
        log(f"[update] banner clicked version={release.version}")
        webbrowser.open(release.url)

    def _dismiss_update(self) -> None:
        if self._update_release is not None:
            log(f"[update] banner dismissed version={self._update_release.version}")
        self.clear_update()

    def clear_update(self) -> None:
        self._update_release = None
        if self._update_row is not None:
            self._update_row.destroy()
            self._update_row = None
            self._update_label = None

    def refresh_labels(self) -> None:
        """介面語言變更後重繪常駐文字（標題列、狀態、錯誤橫幅）與字型。
        已經印在畫面上的訊息不回溯改寫——那是聊天內容，不是介面文字。"""
        self._title_label.configure(text=app_name(), font=ui_font(8))
        self._status_label.configure(font=ui_font(8))
        self._placeholder.configure(font=ui_font(11))
        self._win.title(app_name())
        if self._bubble is not None:
            self._bubble.title(app_name())
        if self._status_state is not None:
            self.set_status(self._status_state)
        if self._error_key is not None:
            self.set_error(self._error_key)
        if self._update_release is not None:
            self.set_update(self._update_release)

    # --- 測試/除錯輔助 ---
    def visible_messages(self) -> list[tuple[str, str]]:
        return [(m.original, m.translated) for m in self._messages]

    def error_text(self) -> str | None:
        return self._error_label.cget("text") if self._error_label else None

    def update_text(self) -> str | None:
        return self._update_label.cget("text") if self._update_label else None

    def status_text(self) -> str:
        return self._status_label.cget("text")

    def placeholder_visible(self) -> bool:
        return self._placeholder.winfo_manager() == "place"
