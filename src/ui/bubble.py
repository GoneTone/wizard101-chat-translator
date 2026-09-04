"""疊加視窗縮小後的浮動泡泡：應用程式 icon 的圓形視窗＋未讀數徽章。

點一下展開（on_click）、拖曳移動（放開時回報 on_move）；有工作列按鈕，Alt+F4／
工作列關閉走 on_close。OverlayWindow 在 minimize() 建立、expand() 銷毀；自動展開
的時機由 overlay 輪詢前景視窗、以 should_auto_expand 決定。"""
import sys
import tkinter as tk

from src.config import app_name
from src.ui.fonts import ui_font
from src.ui.geometry import is_click, moved_to, point_in_rect
from src.ui.icons import load_icon
from src.ui.palette import BAR, GRIP
from src.ui.winstyle import enable_taskbar_button, root_hwnd

BUBBLE_SIZE = 48
_TRANSPARENT = "#010101"  # 泡泡視窗的透明色鍵（方形視窗只露出圓形）
# 未讀數底圓相對文字外框的外擴。跟著泡泡尺寸走，否則泡泡縮小後徽章會相對膨脹，
# 「99+」直接橫跨半顆泡泡蓋掉圖案。
_BADGE_PAD = max(2, round(BUBBLE_SIZE * 0.065))
_BADGE_FONT_SIZE = 7   # 配合 BUBBLE_SIZE：太大會擠掉底下的 101
# 未讀數徽章的垂直位置：對齊 icon 右下角「101」的中心線（量圖得來的比例），
# 兩者才不會看起來一高一低。
_BADGE_BASELINE = 0.82
# 泡泡該比主視窗更低調：在使用者設定的不透明度上再打折，但留下限，免得
# overlay_alpha 調到最低時泡泡幾乎看不見、找不回來。
BUBBLE_ALPHA_FACTOR = 0.8
BUBBLE_ALPHA_FLOOR = 0.30


def bubble_alpha(alpha: float) -> float:
    """泡泡的不透明度：跟著主視窗的設定走，但再透一些（不低於下限）。"""
    return max(BUBBLE_ALPHA_FLOOR, alpha * BUBBLE_ALPHA_FACTOR)


def should_auto_expand(foreground: int, previous: int, bubble_hwnd: int,
                       cursor_on_bubble: bool) -> bool:
    """泡泡被切成前景（點工作列按鈕／Alt+Tab）時是否該自動展開回完整視窗。

    只認「這一輪才變成前景」的轉換，持續在前景時不重複觸發；游標壓在泡泡上
    代表使用者正直接操作泡泡，交給既有的按下／拖曳／放開邏輯處理——否則按下
    的瞬間就展開，泡泡再也拖不動。bubble_hwnd 取不到（0）時一律不觸發，
    避免與 GetForegroundWindow() 的 0（無前景視窗）誤判成相等。"""
    if not bubble_hwnd or foreground != bubble_hwnd or previous == bubble_hwnd:
        return False
    return not cursor_on_bubble


class Bubble(tk.Toplevel):
    """泡泡視窗本體。`hwnd` 是根視窗的 HWND（取不到為 0，見 should_auto_expand）。"""

    def __init__(self, master: tk.Misc, x: int, y: int, alpha: float,
                 on_click, on_move, on_close=None):
        super().__init__(master)
        self._on_click = on_click
        self._on_move = on_move
        # 按下的起點；None＝這次放開沒有對應的按下（見 _release）
        self._drag_state: tuple[int, int, int, int] | None = None
        self.overrideredirect(True)
        self.attributes("-topmost", True)
        self.attributes("-transparentcolor", _TRANSPARENT)
        self.attributes("-alpha", alpha)
        self.configure(bg=_TRANSPARENT)
        self.geometry(f"{BUBBLE_SIZE}x{BUBBLE_SIZE}+{x}+{y}")
        c = tk.Canvas(self, width=BUBBLE_SIZE, height=BUBBLE_SIZE,
                      bg=_TRANSPARENT, highlightthickness=0)
        c.pack()

        def px(f: float) -> int:
            return round(BUBBLE_SIZE * f)

        # 泡泡就是應用程式 icon 本身；載不進來時退回深色圓底，至少還看得見、抓得住。
        self._icon = load_icon(self, BUBBLE_SIZE)
        if self._icon is not None:
            c.create_image(BUBBLE_SIZE // 2, BUBBLE_SIZE // 2, image=self._icon)
        else:
            c.create_oval(2, 2, BUBBLE_SIZE - 2, BUBBLE_SIZE - 2,
                          fill=BAR, outline=GRIP, width=2)

        # 未讀數擺左下：icon 右上是「文A」徽章、右下是 101，只剩左下留白；底下墊深色圓
        # 才有對比，數字直接壓在彩色螺旋上讀不出來。這是徽章的基準位置（set_unread
        # 先歸位再量），水平盡量貼左緣、只留底圓不被裁掉的餘裕。
        self._badge_home = (px(0.18), px(_BADGE_BASELINE))
        bx, by = self._badge_home
        self._badge_dot = c.create_oval(bx, by, bx, by,   # 大小交給 set_unread 依文字重算
                                        fill=BAR, outline=GRIP, state="hidden")
        self._badge = c.create_text(bx, by, text="", fill="#ff9090",
                                    font=ui_font(_BADGE_FONT_SIZE, "bold"))
        self._canvas = c
        c.bind("<ButtonPress-1>", self._press)
        c.bind("<B1-Motion>", self._drag)
        c.bind("<ButtonRelease-1>", self._release)
        self.title(app_name())
        enable_taskbar_button(self)
        # 被 Alt+F4 就地 destroy 的話主視窗仍是隱藏狀態，使用者會完全找不到這支程式，
        # 所以一樣接到乾淨關閉。
        if on_close is not None:
            self.protocol("WM_DELETE_WINDOW", on_close)
        try:
            self.hwnd = root_hwnd(self)
        except Exception as exc:
            self.hwnd = 0
            print(f"[ui] bubble hwnd lookup failed: {exc}", file=sys.stderr)

    def destroy(self) -> None:
        super().destroy()
        # icon 圖片在主執行緒上釋放：留給循環 GC 的話，PhotoImage.__del__ 可能在翻譯
        # worker 執行緒裡觸發，對 Tk 的呼叫會炸「main thread is not in main loop」
        # （測試實測踩過，只在 GC 剛好落在別的執行緒時出現）。
        self._icon = None

    def set_unread(self, count: int) -> None:
        """更新未讀數徽章；0 則隱藏。"""
        text = "99+" if count > 99 else (str(count) if count else "")
        c = self._canvas
        c.itemconfigure(self._badge, text=text)
        if not text:
            c.itemconfigure(self._badge_dot, state="hidden")
            return
        # 先把文字放回基準位置再量：上一輪若因 99+ 往右推過，不歸位會越漂越右
        c.coords(self._badge, *self._badge_home)
        x0, y0, x1, y1 = c.bbox(self._badge)
        # 全部取整再畫：create_oval 高度為 2*half_h+1（奇數），圓心才落在像素正中央、
        # 與數字墨跡（高度同為奇數）對齊；留浮點的話圓心卡在像素邊界，數字永遠差半格。
        half_h = round((y1 - y0) / 2 + _BADGE_PAD)
        # 單一數字的 bbox 又窄又高，等量外擴會成直立橢圓——水平半徑至少拉齊成圓，
        # 位數多了才讓它往左右長成橫橢圓。
        half_w = max(round((x1 - x0) / 2 + _BADGE_PAD), half_h)
        cx, cy = round((x0 + x1) / 2), round((y0 + y1) / 2)
        # 徽章貼著左下角，位數一多底圓會戳出畫布：把圓心推回來，文字跟著走才同心
        cx = max(cx, half_w + 1)
        cy = min(cy, BUBBLE_SIZE - half_h - 1)
        c.coords(self._badge, cx, cy)
        c.coords(self._badge_dot, cx - half_w, cy - half_h, cx + half_w, cy + half_h)
        c.itemconfigure(self._badge_dot, state="normal")

    def pointer_over(self) -> bool:
        """游標目前是否壓在泡泡上。測不到就當作是——寧可不自動展開，也不要在
        使用者正按著泡泡時把它抽走（見 should_auto_expand）。"""
        try:
            px, py = self.winfo_pointerxy()
            return point_in_rect(px, py, self.winfo_x(), self.winfo_y(),
                                 BUBBLE_SIZE, BUBBLE_SIZE)
        except Exception as exc:
            print(f"[ui] bubble hit-test failed: {exc}", file=sys.stderr)
            return True

    def _press(self, e) -> None:
        self._drag_state = (e.x_root, e.y_root, self.winfo_x(), self.winfo_y())

    def _drag(self, e) -> None:
        if self._drag_state is None:
            return
        sx, sy, ox, oy = self._drag_state
        nx, ny = moved_to(ox, oy, e.x_root - sx, e.y_root - sy)
        self.geometry(f"+{nx}+{ny}")

    def _release(self, e) -> None:
        # 點 ─ 縮小時 overlay 會 withdraw 掉正被按住的視窗、隱式 grab 因此斷掉，
        # 放開滑鼠的事件落到剛出現在游標下的泡泡上——沒有對應的按下，當作沒發生
        if self._drag_state is None:
            return
        sx, sy, _, _ = self._drag_state
        self._drag_state = None
        if is_click(e.x_root - sx, e.y_root - sy):
            self._on_click()
            return
        self._on_move(self.winfo_x(), self.winfo_y())
