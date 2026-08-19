"""疊加視窗:無邊框、置頂、半透明、滑鼠穿透;顯示原文+繁中譯文。"""
import time
import tkinter as tk

import win32con
import win32gui

BG = "#101018"
FG_ORIGINAL = "#9a9aa8"
FG_TRANSLATED = "#f2f2f7"
FG_ERROR = "#ff5f5f"


class OverlayWindow:
    def __init__(self, root: tk.Tk, x: int | None, y: int | None,
                 max_messages: int = 8, fade_seconds: int = 180):
        self._max = max_messages
        self._fade = fade_seconds
        self._messages: list[tuple[float, str, str, tk.Frame]] = []
        self._error_label: tk.Label | None = None

        self._win = tk.Toplevel(root)
        self._win.overrideredirect(True)
        self._win.attributes("-topmost", True)
        self._win.attributes("-alpha", 0.85)
        self._win.configure(bg=BG)
        self._win.geometry(f"+{x or 40}+{y or 40}")
        self._frame = tk.Frame(self._win, bg=BG)
        self._frame.pack(fill="both", expand=True, padx=6, pady=4)
        self._win.update_idletasks()
        self._make_click_through()

    def _make_click_through(self) -> None:
        hwnd = win32gui.GetParent(self._win.winfo_id()) or self._win.winfo_id()
        styles = win32gui.GetWindowLong(hwnd, win32con.GWL_EXSTYLE)
        win32gui.SetWindowLong(
            hwnd, win32con.GWL_EXSTYLE,
            styles | win32con.WS_EX_LAYERED | win32con.WS_EX_TRANSPARENT | win32con.WS_EX_NOACTIVATE,
        )

    def add_message(self, original: str, translated: str, now: float | None = None) -> None:
        row = tk.Frame(self._frame, bg=BG)
        tk.Label(row, text=original, bg=BG, fg=FG_ORIGINAL,
                 font=("Microsoft JhengHei", 9), anchor="w", justify="left",
                 wraplength=420).pack(fill="x")
        tk.Label(row, text=translated, bg=BG, fg=FG_TRANSLATED,
                 font=("Microsoft JhengHei", 11), anchor="w", justify="left",
                 wraplength=420).pack(fill="x")
        row.pack(fill="x", pady=2)
        self._messages.append((now if now is not None else time.time(), original, translated, row))
        while len(self._messages) > self._max:
            _, _, _, old_row = self._messages.pop(0)
            old_row.destroy()

    def prune(self, now: float | None = None) -> None:
        cutoff = (now if now is not None else time.time()) - self._fade
        keep = []
        for entry in self._messages:
            if entry[0] <= cutoff:
                entry[3].destroy()
            else:
                keep.append(entry)
        self._messages = keep

    def set_error(self, text: str) -> None:
        self.clear_error()
        self._error_label = tk.Label(self._frame, text=text, bg=BG, fg=FG_ERROR,
                                     font=("Microsoft JhengHei", 10, "bold"), anchor="w")
        self._error_label.pack(fill="x", pady=2)

    def clear_error(self) -> None:
        if self._error_label is not None:
            self._error_label.destroy()
            self._error_label = None

    # --- 測試/除錯輔助 ---
    def visible_messages(self) -> list[tuple[str, str]]:
        return [(orig, trans) for _, orig, trans, _ in self._messages]

    def error_text(self) -> str | None:
        return self._error_label.cget("text") if self._error_label else None
