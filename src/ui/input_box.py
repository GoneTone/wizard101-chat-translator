"""翻譯輸入框：打字（任何語言）→ Enter 翻成遊戲語言、Esc 關閉。
遊戲開聊天輸入框時自動呼出並貼在它正下方（熱鍵呼出則沿用上次的位置）。
翻譯跑背景執行緒，結果經 ui_queue 回主執行緒。"""
import queue
import threading
import tkinter as tk

import win32api
import win32con
import win32gui

from src.composer.paste import force_foreground
from src.config import app_name
from src.i18n import t
from src.log import log
from src.ui.fonts import ui_font
from src.ui.geometry import anchored_position
from src.ui.palette import FG_ERROR, FG_UPDATE
from src.ui.richtext import RichLabel
from src.ui.winstyle import root_hwnd, visible_chrome

BG = "#1a1a24"
FG = "#f2f2f7"
HINT_FG = "#9a9aa8"
GAME_INPUT_MAX_CHARS = 80  # 遊戲聊天輸入框的長度上限（實測）
DEFAULT_WIDTH = 460  # 沒有錨點時的寬度；有錨點就跟遊戲輸入框同寬
MIN_WIDTH = 320  # 遊戲輸入框再窄也不跟：提示文字會擠成一長條，且輸入欄放不下一句話
_INITIAL_HEIGHT = 84  # 開窗時的占位高度；建好內容後隨即由 _fit_height 貼合
ANCHOR_GAP = 4  # 與遊戲輸入框的垂直間距（px）
FALLBACK_X, FALLBACK_Y = 200, 200  # 沒有任何錨點（遊戲沒連上就按熱鍵）時的位置


def work_area_at(x: int, y: int) -> tuple[int, int, int, int]:
    """含 (x, y) 那顆螢幕的工作區 (x, y, w, h)（去掉工作列）；多螢幕時貼齊用的邊界要跟著
    遊戲所在的螢幕走，不能用主螢幕尺寸。"""
    monitor = win32api.MonitorFromPoint((x, y), win32con.MONITOR_DEFAULTTONEAREST)
    left, top, right, bottom = win32api.GetMonitorInfo(monitor)["Work"]
    return left, top, right - left, bottom - top


class InputBox:
    """翻譯輸入框視窗：`show(anchor)` 呼出（記住當下的前景視窗，鍵入時要切回去），
    anchor＝遊戲輸入框的螢幕矩形，視窗貼在它正下方、與它同寬；沒給就沿用上一次的錨點。
    Enter 把文字交給 `translate_fn`，譯文經 `on_translated(text, hwnd)` 送進遊戲。
    每次開關 `_session` +1，背景執行緒的結果對不上號就丟掉。"""

    def __init__(self, root: tk.Tk, translate_fn, ui_queue: queue.Queue, on_translated):
        self._root = root
        self._translate = translate_fn
        self._queue = ui_queue
        self._on_translated = on_translated
        self._anchor: tuple[int, int, int, int] | None = None
        self._width = DEFAULT_WIDTH  # 本次開窗的 client 寬度（見 show）
        self._above_anchor = False   # 本次是否放在錨點上方（高度變化要往上長）
        self._win: tk.Toplevel | None = None
        self._entry: tk.Entry | None = None
        self._status: RichLabel | None = None
        self._target_hwnd: int | None = None
        self._session = 0

    @property
    def is_open(self) -> bool:
        return self._win is not None

    def show(self, anchor: tuple[int, int, int, int] | None = None) -> None:
        """呼出輸入框並貼在 anchor（遊戲輸入框的螢幕矩形）正下方、與它同寬；
        已開著就只是重新對焦。"""
        if self._win is not None:
            self._force_focus()
            return
        if anchor is not None:
            self._anchor = anchor
        self._session += 1
        self._target_hwnd = win32gui.GetForegroundWindow()
        log(f"[input] box opened (target_hwnd={self._target_hwnd:#x}, "
            f"anchor={self._anchor})")
        self._win = tk.Toplevel(self._root)
        # 先藏著把內容建好、量出實際高度再定位：下方放不放得下要看真實高度（提示文字
        # 隨 DPI 與寬度換行），而且不會先閃在占位處再跳到錨點旁
        self._win.withdraw()
        self._win.title(app_name())
        # 尺寸由錨點與內容決定，不開放手動縮放；標題列仍可拖動（當次有效、不記錄）
        self._win.resizable(False, False)
        self._win.attributes("-topmost", True)
        self._win.configure(bg=BG)
        self._win.geometry(f"{DEFAULT_WIDTH}x{_INITIAL_HEIGHT}")
        # 看得見的外框要與遊戲輸入框同寬：client 寬先扣掉可見邊框，定位時再補回隱形邊框。
        # 要先讓 Tk 把外框樣式套到 HWND 上（update_idletasks）再量，否則量到的邊框全是 0
        self._win.update_idletasks()
        left_inset, top_inset, extra_w, extra_h = visible_chrome(root_hwnd(self._win))
        self._width = (max(MIN_WIDTH, self._anchor[2] - extra_w) if self._anchor is not None
                       else DEFAULT_WIDTH)
        self._win.geometry(f"{self._width}x{_INITIAL_HEIGHT}")
        self._entry = tk.Entry(self._win, bg="#262636", fg=FG, insertbackground=FG,
                               font=ui_font(12))
        self._entry.pack(fill="x", padx=8, pady=(10, 4))
        # RichLabel：翻譯失敗訊息裡的網址要能點；它自己依寬度換行，
        # 行數變了才重算視窗高度（值沒變不動，避免回圈）
        self._status = RichLabel(self._win, fg=HINT_FG, bg=BG, font=ui_font(9),
                                 link_fg=FG_UPDATE, on_height_change=self._fit_height)
        self._status.set(t("input.hint"))
        self._status.pack(fill="x", padx=8)
        self._entry.bind("<Return>", self._on_enter)
        self._win.bind("<Escape>", lambda e: self.close())
        self._win.protocol("WM_DELETE_WINDOW", self.close)
        self._fit_height()
        px, py = self._position(self._width + extra_w, self._win.winfo_reqheight() + extra_h)
        self._win.geometry(f"+{px - left_inset}+{py - top_inset}")
        self._force_focus()

    def _position(self, width: int, height: int) -> tuple[int, int]:
        """可見外框（width×height）的左上角：貼在錨點下方（放不下翻到上方、夾在遊戲所在
        螢幕的工作區內）；沒有錨點（遊戲沒連上就按熱鍵）退回固定位置。"""
        self._above_anchor = False
        if self._anchor is None:
            return FALLBACK_X, FALLBACK_Y
        area = work_area_at(self._anchor[0], self._anchor[1])
        x, y = anchored_position(self._anchor, width, height, area, gap=ANCHOR_GAP)
        self._above_anchor = y < self._anchor[1]
        return x, y

    def _force_focus(self) -> None:
        """把輸入框搶到前景並對焦輸入欄。從全域熱鍵開啟時遊戲仍是前景視窗，
        Windows 前景鎖會擋掉單純的 focus，故用 AttachThreadInput + SetForegroundWindow 奪取。"""
        self._win.deiconify()
        self._win.lift()
        self._win.attributes("-topmost", True)
        self._win.focus_force()
        try:
            self._win.update_idletasks()
            force_foreground(root_hwnd(self._win))
        except Exception:
            pass  # 奪取前景失敗：仍有 topmost + focus_force，退回讓使用者點一下輸入框
        self._entry.focus_force()

    def close(self) -> None:
        """關閉輸入框並把前景還給呼出時的視窗。位置與寬度都不記：下次呼出重新貼齊遊戲輸入框。"""
        if self._win is not None:
            self._win.destroy()
            self._win = None
            self._entry = None
            self._status = None
            self._session += 1
            # 前景還給呼出當下的視窗（遊戲）：關窗後 Windows 有時會把焦點交給別的視窗
            force_foreground(self._target_hwnd)

    def _on_enter(self, _event) -> None:
        # 壓縮所有空白（含貼上夾帶的換行）：輸入端也守住單行保證
        text = " ".join(self._entry.get().split())
        if not text:
            self.close()  # 空白按 Enter＝關閉（等同 Esc），快速讓開回到遊戲
            return
        self._entry.configure(state="disabled")
        self._status.set(t("input.translating"), HINT_FG)
        hwnd = self._target_hwnd
        session = self._session
        threading.Thread(target=self._worker, args=(text, hwnd, session), daemon=True).start()

    def _worker(self, text: str, hwnd: int | None, session: int) -> None:
        try:
            translated = self._translate(text)
        except Exception as exc:
            log(f"[input] outgoing translation failed (chars={len(text)}): "
                f"{type(exc).__name__}: {exc}")
            # 先把訊息綁成區域變數：lambda 延後在主執行緒執行，屆時 except 的 exc 已被刪除
            msg = t("input.failed", error=exc)
            self._queue.put(lambda: self._show_error(msg, session))
            return
        self._queue.put(lambda: self._finish(translated, hwnd, session))

    def _show_error(self, message: str, session: int) -> None:
        if session != self._session:
            return  # 過期回呼：輸入框已關閉或重開
        if self._entry is None:
            return
        self._entry.configure(state="normal")
        self._status.set(message, FG_ERROR)
        self._fit_height()

    def _fit_height(self) -> None:
        """依內容自動調整視窗高度（寬度不動）：提示／錯誤文字換行行數會隨
        DPI 縮放、視窗寬度與訊息長度變動，固定高度會把文字切在下緣。寬度一律用開窗時
        算好的值：尚未顯示時 winfo_width() 對不上 geometry 請求，不能讀回。
        放在錨點上方時底邊釘住、往上長：顯示後才量得準的換行若往下長會蓋到遊戲輸入框。"""
        if self._win is None:
            return
        self._win.update_idletasks()
        height = self._win.winfo_reqheight()
        current = self._win.winfo_height()
        if height == current:
            return
        if self._above_anchor and self._win.winfo_viewable():
            x, y = self._win.winfo_x(), self._win.winfo_y() - (height - current)
            self._win.geometry(f"{self._width}x{height}+{x}+{y}")
        else:
            self._win.geometry(f"{self._width}x{height}")

    def _finish(self, translated: str, hwnd: int | None, session: int) -> None:
        if session != self._session:
            return  # 過期回呼：輸入框已關閉或重開
        if len(translated) > GAME_INPUT_MAX_CHARS:
            log(f"[input] translation too long for the game "
                f"(chars={len(translated)}, limit={GAME_INPUT_MAX_CHARS})")
            # 超過遊戲輸入上限：不鍵入、不關窗，讓使用者刪減原文後重送
            self._show_error(t("input.too_long", count=len(translated),
                               limit=GAME_INPUT_MAX_CHARS), session)
            return
        self.close()
        self._on_translated(translated, hwnd)
