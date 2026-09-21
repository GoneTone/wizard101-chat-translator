"""翻譯輸入框：打字（任何語言）→ Enter 翻成遊戲語言、Esc 關閉。
遊戲聊天輸入框開著就貼在它正下方、同寬；沒開（熱鍵呼出）就貼在游標處。
翻譯跑背景執行緒，結果經 ui_queue 回主執行緒。"""
import queue
import threading
import tkinter as tk

import win32gui

from src.composer.paste import force_foreground
from src.config import app_name
from src.i18n import t
from src.log import log
from src.translation.translator import RequestHandle, TranslatorCancelled
from src.ui.fonts import ui_font
from src.ui.geometry import anchored_position
from src.ui.monitors import work_area_at
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
ANCHOR_GAP = 4  # 與遊戲輸入框／游標的垂直間距（px）


def cursor_position() -> tuple[int, int]:
    """游標的螢幕座標。"""
    return win32gui.GetCursorPos()


class InputBox:
    """翻譯輸入框視窗：`show()` 呼出（記住當下的前景視窗，鍵入時要切回去）。
    `set_anchor(rect)`／`clear_anchor()` 跟著遊戲輸入框的開關走：有錨點就貼在它正下方、
    與它同寬，沒有就貼在游標處。Enter 把文字交給 `translate_fn`，譯文經
    `on_translated(text, hwnd)` 送進遊戲。每次開關 `_session` +1，背景執行緒的結果
    對不上號就丟掉；關窗時進行中的翻譯請求也經 `RequestHandle` 撤銷（`translate_fn(text,
    cancel)`），不然伺服器會把沒人要的譯文生成完、照樣計費。
    `close()` 是使用者主動關（Esc／X／空白 Enter）或已送出，未送出的文字一併丟掉；
    `hide()` 是遊戲關了聊天框被動收起，文字留到下次 `show()`。
    `describe_service()` 給失敗 log 指名發話走的是哪一組服務。"""

    def __init__(self, root: tk.Tk, translate_fn, ui_queue: queue.Queue, on_translated,
                 describe_service=lambda: "provider=?, model=?"):
        self._root = root
        self._translate = translate_fn
        self._describe_service = describe_service   # 失敗 log 的服務欄位
        self._queue = ui_queue
        self._on_translated = on_translated
        self._anchor: tuple[int, int, int, int] | None = None  # 遊戲輸入框的螢幕矩形
        self._width = DEFAULT_WIDTH  # 本次開窗的 client 寬度（見 show）
        self._above_anchor = False   # 本次是否放在錨點上方（高度變化要往上長）
        self._win: tk.Toplevel | None = None
        self._entry: tk.Entry | None = None
        self._status: RichLabel | None = None
        self._target_hwnd: int | None = None
        self._session = 0
        self._request: RequestHandle | None = None   # 進行中的翻譯請求，關窗時撤銷
        self._draft = ""  # hide() 收起時尚未送出的文字，下次 show() 還原

    @property
    def is_open(self) -> bool:
        return self._win is not None

    def set_anchor(self, rect: tuple[int, int, int, int]) -> None:
        """遊戲輸入框開了：記下它的螢幕矩形 (x, y, w, h)，下次 show() 貼齊它。"""
        self._anchor = rect

    def clear_anchor(self) -> None:
        """遊戲輸入框關了：之後 show() 改貼游標。"""
        self._anchor = None

    def show(self) -> None:
        """呼出輸入框：貼在遊戲輸入框正下方、與它同寬，沒有錨點就貼在游標處；
        已開著就只是重新對焦。"""
        if self._win is not None:
            self._force_focus()
            return
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
        if self._draft:
            log(f"[input] draft restored (chars={len(self._draft)})")
            self._entry.insert(0, self._draft)
            self._entry.icursor("end")
            self._draft = ""
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
        """可見外框（width×height）的左上角：貼在錨點下方，沒有錨點就把游標當成 0×0 的
        錨點貼在它右下方；放不下翻到上方、夾在錨點所在螢幕的工作區內。"""
        anchor = self._anchor
        if anchor is None:
            cx, cy = cursor_position()
            anchor = (cx, cy, 0, 0)
        area = work_area_at(anchor[0], anchor[1])
        x, y = anchored_position(anchor, width, height, area, gap=ANCHOR_GAP)
        self._above_anchor = y < anchor[1]
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

    def hide(self) -> None:
        """遊戲關了聊天框、被動收起：把尚未送出的文字留到下次 `show()`（翻譯中也算
        未送出，該次結果會因 session 對不上被丟掉，使用者重按 Enter 再翻）。"""
        if self._win is not None:
            self._draft = self._entry.get()
            if self._draft:
                log(f"[input] box hidden, draft kept (chars={len(self._draft)})")
            self._destroy()

    def close(self) -> None:
        """使用者主動關閉或已送出：丟掉未送出的文字，把前景還給呼出時的視窗。
        位置與寬度都不記：下次呼出重新貼齊遊戲輸入框。"""
        self._draft = ""
        if self._win is not None:
            self._destroy()

    def _destroy(self) -> None:
        self._win.destroy()
        self._win = None
        self._entry = None
        self._status = None
        self._session += 1
        if self._request is not None:
            self._request.cancel()
            self._request = None
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
        self._request = RequestHandle()
        threading.Thread(target=self._worker, args=(text, hwnd, session, self._request),
                         daemon=True).start()

    def _worker(self, text: str, hwnd: int | None, session: int,
                cancel: RequestHandle) -> None:
        try:
            translated = self._translate(text, cancel)
        except TranslatorCancelled:
            log(f"[input] outgoing translation cancelled (chars={len(text)})")
            return
        except Exception as exc:
            log(f"[input] outgoing translation failed "
                f"(chars={len(text)}, {self._describe_service()}): "
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
