"""疊加視窗：無邊框、置頂、半透明、可拖曳移動、可從四邊／四角縮放。
顯示原文 + 譯文的列表在 message_list.py（含捲動定位）；本模組負責視窗殼、標題列、
縮放、縮小成泡泡、橫幅，以及框選／複製的事件路由。
不滑鼠穿透 —— 視窗蓋住的區域點擊不會傳到遊戲，視窗永遠可互動。"""
import tkinter as tk
import webbrowser

import win32con
import win32gui

from src.config import app_name
from src.i18n import t
from src.log import log
from src.ui.bubble import BUBBLE_SIZE, Bubble, bubble_alpha, should_auto_expand
from src.ui.fonts import ui_font
from src.ui.geometry import centered_position, edge_at, moved_to, point_in_rect, resized_edge
from src.ui.icons import load_icon
from src.ui.message_list import MessageList
from src.ui.palette import (
    BAR,
    BG,
    BG_UPDATE,
    FG_BAR,
    FG_ERROR,
    FG_UPDATE,
    OUTLINE,
)
from src.ui.popup import Popup
from src.ui.richtext import RichLabel
from src.ui.selection import Selection
from src.ui.winstyle import enable_taskbar_button, make_non_activating, root_hwnd
from src.updater import is_newer

# 狀態指示的顏色（文字由 i18n 依 state key 取得）
STATUS_COLORS = {
    "locating": "#e0b050",
    "listening": "#7dc87d",
    "translating": "#6fa8dc",
    "waiting_game": "#9a9aa8",
    "access_denied": FG_ERROR,  # 要使用者動手（以管理員重開）才解得掉，用錯誤色
    "version_mismatch": FG_ERROR,  # 同上：等下去也不會好，要更新遊戲或本程式
}

MIN_WIDTH = 200
MIN_HEIGHT = 90
_BAR_HEIGHT = 20
_BAR_ICON = 16   # 標題列 icon：留 2px 上下邊給 _BAR_HEIGHT，且是 icon.ico 的原生尺寸
# 標題列文字下推的補償：Label 垂直置中按字型 linespace（含 descent）算，中文與數字
# 沒有下伸部、墨跡整體偏上，與精準置中的 icon 並排就對不齊。實測補 1px 最接近
# （剩下的 0.5px 是墨跡高度奇偶不匹配，補不掉）。
_BAR_TEXT_NUDGE = 1
_GRIP_SIZE = 16
_FOREGROUND_POLL_MS = 300
_AUTOSCROLL_MS = 40      # 框選拖到邊界外時的捲動節奏
_AUTOSCROLL_MIN = 2      # 每輪最少捲幾像素（剛越界時要慢，才停得準）
_AUTOSCROLL_MAX = 24
_EDGE_CURSORS = {"n": "size_ns", "s": "size_ns", "w": "size_we", "e": "size_we",
                 "nw": "size_nw_se", "se": "size_nw_se",
                 "ne": "size_ne_sw", "sw": "size_ne_sw"}


def autoscroll_pixels(y_root: int, top: int, bottom: int,
                      lo: int = _AUTOSCROLL_MIN, hi: int = _AUTOSCROLL_MAX) -> int:
    """框選拖到訊息區之外時，每一輪要捲動的像素（負值往上，落在區內回 0）。
    離邊界越遠捲越快，但設上限 —— 否則游標一滑出視窗就整段飛過去，選不準。"""
    if y_root < top:
        return -min(hi, lo + (top - y_root) // 4)
    if y_root > bottom:
        return min(hi, lo + (y_root - bottom) // 4)
    return 0


class OverlayWindow:
    """疊在遊戲上的譯文視窗：無邊框、常駐最上層、可拖曳縮放，訊息列表可捲動與框選複製。

    由三個 Toplevel 組成：不奪焦點的半透明底板、不透明的文字層、縮小後的泡泡
    （見 bubble.py）。橫幅（翻譯錯誤、更新提示）固定在訊息區下方。所有公開方法都
    必須在 Tk 主執行緒呼叫，背景執行緒經 ui_queue 排回來。"""

    def __init__(self, root: tk.Tk, x: int | None, y: int | None,
                 width: int = 640, height: int = 420,
                 max_messages: int = 50, fade_seconds: int = 180,
                 on_geometry_change=None, on_settings=None, on_close=None,
                 bubble_position: dict | None = None, on_bubble_move=None,
                 alpha: float = 0.80, on_region=None):
        self._alpha = alpha
        self._on_geometry_change = on_geometry_change
        self._on_bubble_move = on_bubble_move
        self._on_close = on_close   # 泡泡建立時要拿它接 WM_DELETE_WINDOW
        self._bubble_pos = dict(bubble_position) if bubble_position else {"x": None, "y": None}
        self._minimized = False
        self._unread = 0
        self._bubble: Bubble | None = None
        self._region_btn: tk.Label | None = None   # 有 on_region 才建立，供測試點擊
        self._prev_foreground = 0
        self._watch_job: str | None = None
        self._error_label: RichLabel | None = None
        self._status_state: str | None = None   # 目前狀態的 key，語言切換後重繪用
        self._error_key: str | None = None      # 目前橫幅的 key，同上
        self._error_kwargs: dict = {}
        self._update_row: tk.Frame | None = None
        self._update_label: tk.Label | None = None
        self._update_release = None   # 目前橫幅對應的 Release，語言切換後重繪用
        self._dismissed_version: str | None = None   # 使用者按 ✕ 關掉的版本，這次執行內不再自動跳出
        self._w = max(width, MIN_WIDTH)
        self._h = max(height, MIN_HEIGHT)
        self._drag = (0, 0, 0, 0)
        # 縮放中的起點與起始幾何；None＝目前沒有在縮放（見 _resize_start）
        self._resize: tuple[int, int, int, int, int, int, str] | None = None
        self._drag_point: tuple[int, int] | None = None   # 框選拖曳的最後座標（自動捲動要用）
        self._autoscroll_job: str | None = None

        self._build_backdrop(root)
        # master 用 root 而非 backdrop：Tk 的 master 連動 restack 會在點擊本體時
        # 把 backdrop 一起抬起、反而蓋過文字層（實測）；OS 擁有關係於下方另設。
        self._win = tk.Toplevel(root)
        self._win.overrideredirect(True)
        self._win.attributes("-topmost", True)
        self._win.attributes("-transparentcolor", BG)
        self._win.configure(bg=BG)
        # 未設定過位置（首次啟動）：擺螢幕正中央，比擺角落更容易被注意到
        cx, cy = centered_position(self._win.winfo_screenwidth(),
                                   self._win.winfo_screenheight(), self._w, self._h)
        self._apply_geometry(x if x is not None else cx, y if y is not None else cy,
                             self._w, self._h)
        self._build_title_bar(on_settings, on_close, on_region)
        self._build_message_area(max_messages, fade_seconds)
        self._build_resize_handles()
        self._attach_to_shell(on_close)

    def _build_backdrop(self, root: tk.Tk) -> None:
        """半透明底板：雙層視窗的下層（見內文）。"""
        # 雙層視窗：tk 的 -alpha 整窗生效、無法只透背景，故拆兩層 —— 下層 backdrop 承擔
        # 半透明底板（透明度設定作用於此），上層本體以 -transparentcolor 挖空背景色，
        # 文字與控制項保持完全不透明。本體由 backdrop 擁有（owned window），永遠疊在其上。
        self._backdrop = tk.Toplevel(root)
        self._backdrop.overrideredirect(True)
        self._backdrop.attributes("-topmost", True)
        self._backdrop.attributes("-alpha", self._alpha)
        self._backdrop.configure(bg=BG)
        # 底板攔截透明背景區的滑鼠事件（不穿透到遊戲），但點擊不奪焦點、不改疊序
        make_non_activating(self._backdrop)

    def _build_title_bar(self, on_settings, on_close, on_region=None) -> None:
        """標題列：icon、標題、狀態字與 ⚙／⛶／─／✕，整列可拖曳移動、上緣可縮放。"""
        bar = tk.Frame(self._win, bg=BAR, height=_BAR_HEIGHT, cursor="fleur")
        bar.pack(side="top", fill="x")
        bar.pack_propagate(False)
        # 不另放拖曳把手符號，可拖曳的暗示交給 bar 的 fleur 游標
        self._bar_icon = load_icon(self._win, _BAR_ICON)
        self._title_label = tk.Label(bar, text=app_name(), bg=BAR, fg=FG_BAR,
                                     font=ui_font(8), anchor="w")
        # side="right" 先 pack 者占最外側：由右到左為 ✕、⚙、⛶（開始框選）、狀態字。
        # 打包版沒有主控台，✕ 是唯一的正常關閉途徑，所以整組控制項都排在標題之前
        # pack —— 標題再長或視窗再窄，被裁掉的只會是標題。
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
        if on_region is not None:
            self._region_btn = tk.Label(bar, text="⛶", bg=BAR, fg=FG_BAR,
                                        font=ui_font(9), cursor="hand2")
            self._region_btn.pack(side="right", padx=(0, 4))
            self._region_btn.bind("<Button-1>", lambda e: on_region())
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

    def _build_message_area(self, max_messages: int, fade_seconds: int) -> None:
        """內容區：訊息列表（見 message_list.py）；橫幅事後才 pack 進來、排在列表之前。"""
        # 先建 _selection：列表綁定的 <Configure> 可能在事件迴圈中提早觸發、呼叫
        # self._selection.redraw()，物件要先存在。
        self._selection = Selection()
        self._frame = tk.Frame(self._win, bg=BG)
        self._frame.pack(side="top", fill="both", expand=True)
        self._list = MessageList(self._frame, self._selection,
                                 max_messages=max_messages, fade_seconds=fade_seconds,
                                 wrap=self._w - 40, bottom_inset=_GRIP_SIZE,
                                 bind_line=self._bind_line,
                                 on_wrap_change=self._on_wrap_change)
        # 底板空白處的滾輪也轉發給訊息區捲動
        self._backdrop.bind("<MouseWheel>", self._list.on_wheel)

    def _bind_line(self, line: tk.Canvas) -> None:
        """每一行文字 canvas 建好時接上框選與右鍵選單。"""
        line.bind("<ButtonPress-1>", self._selection_press)
        line.bind("<B1-Motion>", self._selection_drag)
        line.bind("<ButtonRelease-1>", self._selection_release)
        line.bind("<Button-3>", self._selection_menu)

    def _on_wrap_change(self, wrap: int) -> None:
        """換行寬度隨畫布寬更新後，更新橫幅也要跟著換行，否則縮小視窗後右緣被切。"""
        if self._update_label is not None:
            self._update_label.configure(wraplength=wrap)

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
        # 上緣被標題列擋住，由標題列自己的按下事件分流（見 _bar_press） —— 在標題列上
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
        # 按鈕，而 WS_EX_APPWINDOW 要等下一次 hide→show 才重新生效 —— 順序反過來，
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
        # Caps Lock 開著時 Tk 送的是 <Control-C>，兩個都要接。不用 bind_all ——
        # 那會連設定視窗的輸入框一起攔截。
        self._win.bind("<Control-c>", self.copy_selection)
        self._win.bind("<Control-C>", self.copy_selection)
        # 右鍵與左鍵一樣要雙路由：選取區以外的空白處按右鍵，事件會穿透到 backdrop
        self._backdrop.bind("<Button-3>", self._selection_menu)
        self._popup = Popup(self._win, self.copy_selection)
        # 點視窗任何一處都收起選單。綁在兩個 toplevel 上而不是各個控件上：Tk 的事件會
        # 沿 bindtags 傳到所屬的 toplevel，標題列、捲軸、右下把手因此一併涵蓋 —— 那些
        # 正是使用者會直覺點的「別的地方」，漏掉的話選單會賴著不走。
        for shell in (self._win, self._backdrop):
            shell.bind("<ButtonPress-1>", lambda e: self._popup.hide(), add="+")

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
        self._stop_autoscroll()
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
        log(f"[ui] minimized to bubble ({self._list.scroll_debug()})")

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
        重算 —— deiconify 幾何沒變就不會帶來 <Configure>。"""
        self._list.refresh_scroll()
        log(f"[ui] expanded ({self._list.scroll_debug()})")

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
        self._win.geometry(f"{w}x{h}+{x}+{y}")
        self._backdrop.geometry(f"{w}x{h}+{x}+{y}")

    def _emit_geometry(self) -> None:
        if self._on_geometry_change is None:
            return
        self._win.update_idletasks()
        self._on_geometry_change(self._win.winfo_x(), self._win.winfo_y(),
                                 self._win.winfo_width(), self._win.winfo_height())

    def _move_start(self, e) -> None:
        self._drag = (e.x_root, e.y_root, self._win.winfo_x(), self._win.winfo_y())

    def _move_drag(self, e) -> None:
        sx, sy, ox, oy = self._drag
        nx, ny = moved_to(ox, oy, e.x_root - sx, e.y_root - sy)
        self._apply_geometry(nx, ny, self._w, self._h)

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
        return point_in_rect(x_root, y_root, *self._list.viewport())

    def _selection_press(self, e) -> None:
        """框選起手。訊息列的底色是本體的透明色鍵，只有文字墨跡接得到滑鼠、其餘落到
        backdrop，兩條路徑都導進這裡，一律用螢幕座標。"""
        if not self._in_message_area(e.x_root, e.y_root):
            self._selection.clear("press outside the message area")
            return
        if self._selection.begin(e.x_root, e.y_root):
            self._focus_for_copy()

    def _selection_drag(self, e) -> None:
        self._drag_point = (e.x_root, e.y_root)
        self._selection.extend(e.x_root, e.y_root)
        if self._autoscroll_job is None:
            self._autoscroll_job = self._win.after(_AUTOSCROLL_MS, self._autoscroll)

    def _selection_release(self, e) -> None:
        self._stop_autoscroll()
        self._selection.finish()

    def _autoscroll(self) -> None:
        """拖到訊息區上下緣之外時持續捲動，選取才能延伸到畫面外的訊息。

        每捲一次都要用最後的游標座標重算一次 focus —— 內容在游標底下移動了，
        游標壓著的字跟著換人，不重算的話選取範圍會停在捲動前的位置。"""
        self._autoscroll_job = None
        if not self._selection.dragging or self._drag_point is None:
            return
        _, top, _, height = self._list.viewport()
        step = autoscroll_pixels(self._drag_point[1], top, top + height - 1)
        if step and self._list.scroll_by(step):
            self._selection.extend(*self._drag_point)
        self._autoscroll_job = self._win.after(_AUTOSCROLL_MS, self._autoscroll)

    def _stop_autoscroll(self) -> None:
        """取消排程。`_drag_point` 刻意留著 —— 它只被 `_autoscroll` 讀，而那裡本來就會
        先看 `dragging`；清掉反而讓「停掉排程」與「結束拖曳」兩件事糊在一起。"""
        if self._autoscroll_job is not None:
            self._win.after_cancel(self._autoscroll_job)
            self._autoscroll_job = None

    def _focus_for_copy(self) -> None:
        """把鍵盤焦點交給本體，Ctrl+C 才收得到 —— backdrop 帶 WS_EX_NOACTIVATE，
        從它起手的選取不會給焦點。"""
        try:
            log("[ui] forcing keyboard focus")
            self._win.focus_force()
            log("[ui] selection took keyboard focus")
        except tk.TclError as exc:
            log(f"[ui] selection focus failed: {exc}")

    def copy_selection(self, _event=None) -> None:
        """把目前選取的文字寫進系統剪貼簿。沒有選取就什麼都不做 —— 寫入空字串會把
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

    # --- 訊息（實作見 message_list.py） ---
    def add_message(self, original: str, translated: str, now: float | None = None,
                    msg_id: int | None = None, pending: bool = False,
                    color: str | None = None) -> None:
        """加入一則訊息（參數見 MessageList.add_message）；縮小成泡泡期間累計未讀數。"""
        self._list.add_message(original, translated, now, msg_id, pending, color)
        if self._minimized and self._bubble is not None:
            self._unread += 1
            self._bubble.set_unread(self._unread)

    def update_message(self, msg_id: int, translated: str,
                       failed: bool = False) -> None:
        """把某則佔位訊息的譯文就地填入（見 MessageList.update_message）。"""
        self._list.update_message(msg_id, translated, failed)

    def set_limits(self, max_messages: int, fade_seconds: int) -> None:
        """套用新的訊息上限與淡出秒數；超出上限的最舊訊息立即移除。"""
        self._list.set_limits(max_messages, fade_seconds)

    def prune(self, now: float | None = None) -> None:
        """移除已超過淡出秒數的訊息（fade_seconds <= 0 時不做）；`now` 供測試指定時刻。"""
        self._list.prune(now)

    def set_status(self, state: str) -> None:
        """更新狀態指示：標題列右側小字；視窗還沒有任何訊息時，同步大字置中顯示。
        存的是 state key 而非文字 —— 語言切換後 refresh_labels() 才能重新翻譯。"""
        self._status_state = state
        text, color = t(f"status.{state}"), STATUS_COLORS[state]
        self._status_label.configure(text=text, fg=color)
        self._list.set_placeholder(text, color)

    def set_error(self, key: str, **kwargs) -> None:
        """顯示錯誤橫幅（傳入文案 key 與 format 變數，顯示時才翻譯）。"""
        self.clear_error()
        self._error_key = key
        self._error_kwargs = kwargs
        # RichLabel：API 錯誤訊息裡的網址要能點；它自己依寬度換行、行數決定高度
        self._error_label = RichLabel(self._frame, fg=FG_ERROR, bg=BG,
                                      font=ui_font(10, "bold"), link_fg=FG_UPDATE)
        self._error_label.set(t(key, **kwargs))
        # before＝捲動區：pack 依宣告順序分配空間，橫幅排在 expand=True 的捲動區
        # 之後就會在視窗被縮小時被擠掉 —— 而「遊戲未就緒」正是最該看到的訊息。
        self._error_label.pack(side="bottom", fill="x", pady=2,
                               before=self._list.frame)

    def clear_error(self) -> None:
        """收起錯誤橫幅（翻譯恢復、遊戲重新連上時）。"""
        self._error_key = None
        if self._error_label is not None:
            self._error_label.destroy()
            self._error_label = None

    def offer_update(self, release) -> None:
        """自動檢查用的入口：使用者按 ✕ 關掉過的版本（或更舊的）不再跳出，
        只有比它新的版本才顯示橫幅。手動檢查與換語言重繪直接走 set_update。"""
        dismissed = self._dismissed_version
        if dismissed is not None and not is_newer(release.version, dismissed):
            log(f"[update] banner skipped: {release.version} not newer than "
                f"dismissed {dismissed}")
            return
        self.set_update(release)

    def set_update(self, release) -> None:
        """顯示更新橫幅：整列可點（開瀏覽器到下載頁），右側 ✕ 關掉並記下版本，
        自動檢查（offer_update）就不再為同版跳出。與錯誤橫幅各佔一列、互不覆蓋 ——
        兩者可能同時該被看到。release 存起來，換語言時才重繪得出來。"""
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
                         wraplength=self._list.wrap)
        label.pack(side="left", fill="x", expand=True)
        label.bind("<Button-1>", lambda e: self._open_update_link(release))
        # before＝捲動區：同錯誤橫幅。右側讓出縮放把手的寬度：把手底色是透明色鍵，
        # 疊在這條不透明橫幅上會挖出缺口並蓋掉半個 ✕（捲軸讓出把手高度同理）。
        row.pack(side="bottom", fill="x", pady=2, padx=(0, _GRIP_SIZE),
                 before=self._list.frame)
        self._update_row = row
        self._update_label = label
        log(f"[update] banner shown for {release.version}")

    def _open_update_link(self, release) -> None:
        log(f"[update] banner clicked version={release.version}")
        webbrowser.open(release.url)

    def _dismiss_update(self) -> None:
        if self._update_release is not None:
            self._dismissed_version = self._update_release.version
            log(f"[update] banner dismissed version={self._update_release.version}")
        self.clear_update()

    def clear_update(self) -> None:
        """收起更新提示橫幅。"""
        self._update_release = None
        if self._update_row is not None:
            self._update_row.destroy()
            self._update_row = None
            self._update_label = None

    def refresh_labels(self) -> None:
        """介面語言變更後重繪常駐文字（標題列、狀態、錯誤橫幅）與字型。
        已經印在畫面上的訊息不回溯改寫 —— 那是聊天內容，不是介面文字。"""
        self._title_label.configure(text=app_name(), font=ui_font(8))
        self._status_label.configure(font=ui_font(8))
        self._list.refresh_fonts()
        self._win.title(app_name())
        if self._bubble is not None:
            self._bubble.title(app_name())
        if self._status_state is not None:
            self.set_status(self._status_state)
        if self._error_key is not None:
            self.set_error(self._error_key, **self._error_kwargs)
        if self._update_release is not None:
            self.set_update(self._update_release)

    # --- 測試/除錯輔助 ---
    def visible_messages(self) -> list[tuple[str, str]]:
        return self._list.visible_messages()

    def error_text(self) -> str | None:
        return self._error_label.text() if self._error_label else None

    def update_text(self) -> str | None:
        return self._update_label.cget("text") if self._update_label else None

    def status_text(self) -> str:
        return self._status_label.cget("text")

    def placeholder_visible(self) -> bool:
        return self._list.placeholder_visible()
