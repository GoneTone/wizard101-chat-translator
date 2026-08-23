"""熱鍵呼出的翻譯輸入框：打字（任何語言）→ Enter 翻成遊戲語言、Esc 關閉。
翻譯跑背景執行緒，結果經 ui_queue 回主執行緒。"""
import ctypes
import queue
import threading
import tkinter as tk

import win32gui

from src.config import APP_NAME

BG = "#1a1a24"
FG = "#f2f2f7"


class InputBox:
    def __init__(self, root: tk.Tk, translate_fn, ui_queue: queue.Queue, on_translated,
                 position: dict | None = None, on_move=None):
        self._root = root
        self._translate = translate_fn
        self._queue = ui_queue
        self._on_translated = on_translated
        self._pos = position or {"x": None, "y": None}
        self._on_move = on_move
        self._win: tk.Toplevel | None = None
        self._entry: tk.Entry | None = None
        self._status: tk.Label | None = None
        self._target_hwnd: int | None = None
        self._session = 0

    def show(self) -> None:
        if self._win is not None:  # 已開著就聚焦
            self._force_focus()
            return
        self._session += 1
        self._target_hwnd = win32gui.GetForegroundWindow()
        self._win = tk.Toplevel(self._root)
        self._win.title(APP_NAME)
        self._win.resizable(False, False)  # 高度依內容自適應，手動縮放會切到文字
        self._win.attributes("-topmost", True)
        self._win.configure(bg=BG)
        px = self._pos["x"] if self._pos.get("x") is not None else 200
        py = self._pos["y"] if self._pos.get("y") is not None else 200
        self._win.geometry(f"460x84+{px}+{py}")
        self._entry = tk.Entry(self._win, bg="#262636", fg=FG, insertbackground=FG,
                               font=("Microsoft JhengHei", 12))
        self._entry.pack(fill="x", padx=8, pady=(10, 4))
        self._status = tk.Label(self._win, text="輸入訊息，Enter 翻譯並輸入遊戲（不會自動送出），Esc 關閉",
                                bg=BG, fg="#9a9aa8", font=("Microsoft JhengHei", 9),
                                anchor="w", justify="left", wraplength=436)
        self._status.pack(fill="x", padx=8)
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
            hwnd = win32gui.GetAncestor(self._win.winfo_id(), 2)  # GA_ROOT
            user32 = ctypes.windll.user32
            fg_thread = user32.GetWindowThreadProcessId(win32gui.GetForegroundWindow(), None)
            this_thread = ctypes.windll.kernel32.GetCurrentThreadId()
            user32.AttachThreadInput(this_thread, fg_thread, True)
            try:
                user32.SetForegroundWindow(hwnd)
            finally:
                user32.AttachThreadInput(this_thread, fg_thread, False)
        except Exception:
            pass  # 奪取前景失敗：仍有 topmost + focus_force，退回讓使用者點一下輸入框
        self._entry.focus_force()

    def close(self) -> None:
        if self._win is not None:
            self._remember_position()
            self._win.destroy()
            self._win = None
            self._entry = None
            self._status = None
            self._session += 1

    def _remember_position(self) -> None:
        """記住輸入框目前位置，供下次開啟還原（關閉前呼叫）。"""
        try:
            x, y = self._win.winfo_x(), self._win.winfo_y()
        except Exception:
            return
        self._pos = {"x": x, "y": y}
        if self._on_move is not None:
            self._on_move(x, y)

    def _on_enter(self, _event) -> None:
        text = self._entry.get().strip()
        if not text:
            self.close()  # 空白按 Enter＝關閉（等同 Esc），快速讓開回到遊戲
            return
        self._entry.configure(state="disabled")
        self._status.configure(text="翻譯中…", fg="#9a9aa8")
        hwnd = self._target_hwnd
        session = self._session
        threading.Thread(target=self._worker, args=(text, hwnd, session), daemon=True).start()

    def _worker(self, text: str, hwnd: int | None, session: int) -> None:
        try:
            translated = self._translate(text)
        except Exception as exc:
            # 先把訊息綁成區域變數：lambda 延後在主執行緒執行，屆時 except 的 exc 已被刪除
            msg = f"翻譯失敗：{exc}"
            self._queue.put(lambda: self._show_error(msg, session))
            return
        self._queue.put(lambda: self._finish(translated, hwnd, session))

    def _show_error(self, message: str, session: int) -> None:
        if session != self._session:
            return  # stale/cancelled
        if self._entry is None:
            return
        self._entry.configure(state="normal")
        self._status.configure(text=message, fg="#ff5f5f")
        self._fit_height()

    def _fit_height(self) -> None:
        """依內容自動調整視窗高度（位置不動）：提示／錯誤文字換行行數會隨
        DPI 縮放與訊息長度變動，固定高度會把文字切在下緣。"""
        self._win.update_idletasks()
        self._win.geometry(f"460x{max(84, self._win.winfo_reqheight())}")

    def _finish(self, translated: str, hwnd: int | None, session: int) -> None:
        if session != self._session:
            return  # stale/cancelled
        self.close()
        self._on_translated(translated, hwnd)
