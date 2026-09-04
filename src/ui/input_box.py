"""熱鍵呼出的翻譯輸入框：打字（任何語言）→ Enter 翻成遊戲語言、Esc 關閉。
翻譯跑背景執行緒，結果經 ui_queue 回主執行緒。"""
import queue
import threading
import tkinter as tk

import win32gui

from src.composer.paste import force_foreground
from src.config import app_name
from src.i18n import t
from src.ui.fonts import ui_font
from src.ui.responsive import HINT_TRAILING, apply_wrap, bind_wrap
from src.ui.winstyle import root_hwnd

BG = "#1a1a24"
FG = "#f2f2f7"
GAME_INPUT_MAX_CHARS = 80  # 遊戲聊天輸入框的長度上限（實測）
DEFAULT_WIDTH = 460
MIN_WIDTH = 320  # 再窄會把提示文字擠成一長條，且輸入欄放不下一句話
_INITIAL_HEIGHT = 84  # 開窗時的占位高度；建好內容後隨即由 _fit_height 貼合


class InputBox:
    def __init__(self, root: tk.Tk, translate_fn, ui_queue: queue.Queue, on_translated,
                 position: dict | None = None, width: int = DEFAULT_WIDTH,
                 on_geometry_change=None):
        self._root = root
        self._translate = translate_fn
        self._queue = ui_queue
        self._on_translated = on_translated
        self._pos = position or {"x": None, "y": None}
        self._width = max(MIN_WIDTH, width)
        self._on_geometry_change = on_geometry_change
        self._win: tk.Toplevel | None = None
        self._entry: tk.Entry | None = None
        self._status: tk.Label | None = None
        self._target_hwnd: int | None = None
        self._session = 0

    def show(self) -> None:
        if self._win is not None:
            self._force_focus()
            return
        self._session += 1
        self._target_hwnd = win32gui.GetForegroundWindow()
        self._win = tk.Toplevel(self._root)
        self._win.title(app_name())
        # 只放開寬度：高度由 _fit_height 依內容自適應，手動拉高會露出一片空白
        self._win.resizable(True, False)
        # 高度不受下限拘束：完全交給 _fit_height 依內容決定（拉寬後行數變少要能縮回去）
        self._win.minsize(MIN_WIDTH, 1)
        self._win.attributes("-topmost", True)
        self._win.configure(bg=BG)
        px = self._pos["x"] if self._pos.get("x") is not None else 200
        py = self._pos["y"] if self._pos.get("y") is not None else 200
        self._win.geometry(f"{self._width}x{_INITIAL_HEIGHT}+{px}+{py}")
        self._entry = tk.Entry(self._win, bg="#262636", fg=FG, insertbackground=FG,
                               font=ui_font(12))
        self._entry.pack(fill="x", padx=8, pady=(10, 4))
        self._status = tk.Label(self._win, text=t("input.hint"),
                                bg=BG, fg="#9a9aa8", font=ui_font(9),
                                anchor="w", justify="left")
        self._status.pack(fill="x", padx=8)
        # 拉寬視窗 → 提示文字重新換行 → 行數變了才重算高度（值沒變不動，避免回圈）
        bind_wrap(self._status, container=self._win, trailing=HINT_TRAILING,
                  on_change=self._fit_height)
        # 視窗還沒 map 時量不到寬度，先用目標寬度套一次；否則開窗高度會先窄一格再跳
        apply_wrap(self._status, self._width, HINT_TRAILING)
        self._entry.bind("<Return>", self._on_enter)
        self._win.bind("<Escape>", lambda e: self.close())
        self._win.protocol("WM_DELETE_WINDOW", self.close)
        self._fit_height()
        self._force_focus()

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
        if self._win is not None:
            self._remember_geometry()
            self._win.destroy()
            self._win = None
            self._entry = None
            self._status = None
            self._session += 1
            # 前景還給呼出當下的視窗（遊戲）：關窗後 Windows 有時會把焦點交給別的視窗
            force_foreground(self._target_hwnd)

    def _remember_geometry(self) -> None:
        """記住位置與寬度供下次開啟還原（關閉前呼叫）；高度不記，依內容自適應。"""
        try:
            self._win.update_idletasks()
            x, y, width = self._win.winfo_x(), self._win.winfo_y(), self._current_width()
        except Exception:
            return
        self._pos = {"x": x, "y": y}
        self._width = width
        if self._on_geometry_change is not None:
            self._on_geometry_change(x, y, self._width)

    def _on_enter(self, _event) -> None:
        # 壓縮所有空白（含貼上夾帶的換行）：輸入端也守住單行保證
        text = " ".join(self._entry.get().split())
        if not text:
            self.close()  # 空白按 Enter＝關閉（等同 Esc），快速讓開回到遊戲
            return
        self._entry.configure(state="disabled")
        self._status.configure(text=t("input.translating"), fg="#9a9aa8")
        hwnd = self._target_hwnd
        session = self._session
        threading.Thread(target=self._worker, args=(text, hwnd, session), daemon=True).start()

    def _worker(self, text: str, hwnd: int | None, session: int) -> None:
        try:
            translated = self._translate(text)
        except Exception as exc:
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
        self._status.configure(text=message, fg="#ff5f5f")
        self._fit_height()

    def _current_width(self) -> int:
        """目前視窗寬度；尚未 map 時 winfo_width() 回 1，退回記憶中的寬度。"""
        width = self._win.winfo_width()
        return max(MIN_WIDTH, width if width > 1 else self._width)

    def _fit_height(self) -> None:
        """依內容自動調整視窗高度（位置與寬度不動）：提示／錯誤文字換行行數會隨
        DPI 縮放、視窗寬度與訊息長度變動，固定高度會把文字切在下緣。"""
        if self._win is None:
            return
        self._win.update_idletasks()
        height = self._win.winfo_reqheight()
        if height != self._win.winfo_height():
            self._win.geometry(f"{self._current_width()}x{height}")

    def _finish(self, translated: str, hwnd: int | None, session: int) -> None:
        if session != self._session:
            return  # 過期回呼：輸入框已關閉或重開
        if len(translated) > GAME_INPUT_MAX_CHARS:
            # 超過遊戲輸入上限：不鍵入、不關窗，讓使用者刪減原文後重送
            self._show_error(t("input.too_long", count=len(translated),
                               limit=GAME_INPUT_MAX_CHARS), session)
            return
        self.close()
        self._on_translated(translated, hwnd)
