"""熱鍵呼出的繁中輸入框:Enter 翻譯、Esc 關閉。翻譯跑背景執行緒,結果經 ui_queue 回主執行緒。"""
import queue
import threading
import tkinter as tk

import win32gui

BG = "#1a1a24"
FG = "#f2f2f7"


class InputBox:
    def __init__(self, root: tk.Tk, translate_fn, ui_queue: queue.Queue, on_translated):
        self._root = root
        self._translate = translate_fn
        self._queue = ui_queue
        self._on_translated = on_translated
        self._win: tk.Toplevel | None = None
        self._entry: tk.Entry | None = None
        self._status: tk.Label | None = None
        self._target_hwnd: int | None = None

    def show(self) -> None:
        if self._win is not None:  # 已開著就聚焦
            self._win.lift()
            self._entry.focus_force()
            return
        self._target_hwnd = win32gui.GetForegroundWindow()
        self._win = tk.Toplevel(self._root)
        self._win.title("翻譯輸入")
        self._win.attributes("-topmost", True)
        self._win.configure(bg=BG)
        self._win.geometry("460x84+200+200")
        self._entry = tk.Entry(self._win, bg="#262636", fg=FG, insertbackground=FG,
                               font=("Microsoft JhengHei", 12))
        self._entry.pack(fill="x", padx=8, pady=(10, 4))
        self._status = tk.Label(self._win, text="打繁中,Enter 翻譯並貼進遊戲(不會自動送出),Esc 關閉",
                                bg=BG, fg="#9a9aa8", font=("Microsoft JhengHei", 9), anchor="w")
        self._status.pack(fill="x", padx=8)
        self._entry.bind("<Return>", self._on_enter)
        self._win.bind("<Escape>", lambda e: self.close())
        self._win.protocol("WM_DELETE_WINDOW", self.close)
        self._entry.focus_force()

    def close(self) -> None:
        if self._win is not None:
            self._win.destroy()
            self._win = None
            self._entry = None
            self._status = None

    def _on_enter(self, _event) -> None:
        text = self._entry.get().strip()
        if not text:
            return
        self._entry.configure(state="disabled")
        self._status.configure(text="翻譯中…", fg="#9a9aa8")
        hwnd = self._target_hwnd
        threading.Thread(target=self._worker, args=(text, hwnd), daemon=True).start()

    def _worker(self, text: str, hwnd: int | None) -> None:
        try:
            english = self._translate(text)
        except Exception as exc:
            self._queue.put(lambda: self._show_error(f"翻譯失敗:{exc}"))
            return
        self._queue.put(lambda: self._finish(english, hwnd))

    def _show_error(self, message: str) -> None:
        if self._entry is None:
            return
        self._entry.configure(state="normal")
        self._status.configure(text=message, fg="#ff5f5f")

    def _finish(self, english: str, hwnd: int | None) -> None:
        self.close()
        self._on_translated(english, hwnd)
