"""首次設定精靈：選服務商 → 填 API → 測試連線 → 偏好設定，4 步完成寫入 cfg。
中途關閉＝取消（不留半套設定），run_wizard 回傳 False。"""
import tkinter as tk
from tkinter import ttk

from src.ui.fields import ApiFields, HotkeyField, LanguageField, validate_api_form

STEP_WELCOME, STEP_API, STEP_TEST, STEP_DONE = 0, 1, 2, 3
_TITLES = ["歡迎使用", "API 設定", "測試連線", "偏好設定"]


def can_advance(step: int, api_test_passed: bool, api_errors: list[str]) -> bool:
    """該步驟是否允許按「下一步」。"""
    if step == STEP_API:
        return not api_errors
    if step == STEP_TEST:
        return api_test_passed
    return True


class SetupWizard:
    """精靈視窗本體。completed 屬性表示是否走完全部步驟。"""

    def __init__(self, root: tk.Tk, cfg: dict):
        self._cfg = cfg
        self.completed = False
        self._step = STEP_WELCOME
        self._skip_test = False

        self._win = tk.Toplevel(root)
        self._win.title("Wizard101 聊天翻譯助手 — 首次設定")
        win_w, win_h = 520, 420
        x = (self._win.winfo_screenwidth() - win_w) // 2
        y = (self._win.winfo_screenheight() - win_h) // 2
        self._win.geometry(f"{win_w}x{win_h}+{x}+{y}")
        self._win.resizable(False, False)
        self._win.protocol("WM_DELETE_WINDOW", self._cancel)

        self._indicator = ttk.Label(self._win, text="")
        self._indicator.pack(pady=(10, 0))
        self._title = ttk.Label(self._win, font=("Microsoft JhengHei", 13, "bold"))
        self._title.pack(pady=(2, 8))
        self._body = ttk.Frame(self._win, padding=16)
        self._body.pack(fill="both", expand=True)

        nav = ttk.Frame(self._win, padding=8)
        nav.pack(side="bottom", fill="x")
        self._back_btn = ttk.Button(nav, text="上一步", command=self._back)
        self._back_btn.pack(side="left")
        self._next_btn = ttk.Button(nav, text="下一步", command=self._next)
        self._next_btn.pack(side="right")

        # 跨步驟保留的欄位元件（建一次，切步驟時搬進／搬出 body）
        self._api_fields = ApiFields(self._body, cfg["api"], on_change=self._refresh_nav)
        self._language = LanguageField(self._body, cfg["target_language"])
        self._hotkey = HotkeyField(self._body, cfg["hotkey"])
        self._show_step()

    # --- 導航 ---
    def _show_step(self) -> None:
        for w in self._body.winfo_children():
            w.pack_forget()
        self._indicator.configure(
            text="  ".join("●" if i <= self._step else "○" for i in range(4)))
        self._title.configure(text=_TITLES[self._step])

        if self._step == STEP_WELCOME:
            ttk.Label(self._body, wraplength=440, text=(
                "本工具會即時翻譯 Wizard101 的遊戲聊天，並可用熱鍵輸入你的語言、"
                "翻成英文送進遊戲。\n\n首先，請選擇要使用的翻譯服務：")).pack(anchor="w")
            self._api_fields.pack(fill="x", pady=(12, 0))
        elif self._step == STEP_API:
            self._api_fields.pack(fill="x")
        elif self._step == STEP_TEST:
            ttk.Label(self._body, wraplength=440, text=(
                "按「測試連線」確認設定可用（會實際翻譯一句測試文字）。")).pack(anchor="w")
            self._api_fields.pack(fill="x", pady=(12, 0))
            skip = ttk.Label(self._body, text="略過測試", foreground="#888888",
                             cursor="hand2", font=("Microsoft JhengHei", 8))
            skip.pack(anchor="e", pady=(6, 0))
            skip.bind("<Button-1>", lambda e: self._do_skip_test())
        else:  # STEP_DONE
            ttk.Label(self._body, text="翻譯目標語言（收到的訊息翻成什麼語言）").pack(anchor="w")
            self._language.pack(fill="x", pady=(2, 12))
            ttk.Label(self._body, text="呼出輸入框的熱鍵").pack(anchor="w")
            self._hotkey.pack(anchor="w", pady=(2, 0))
            self._next_btn.configure(text="完成")
        if self._step != STEP_DONE:
            self._next_btn.configure(text="下一步")
        self._back_btn.configure(
            state="normal" if self._step > STEP_WELCOME else "disabled")
        self._refresh_nav()

    def _refresh_nav(self) -> None:
        if not hasattr(self, "_api_fields"):
            return  # ApiFields 建構中觸發的第一次 on_change：欄位元件尚未掛上 self，略過
        api = self._api_fields.get_values()
        ok = can_advance(self._step,
                         self._api_fields.test_passed or self._skip_test,
                         validate_api_form(api))
        self._next_btn.configure(state="normal" if ok else "disabled")

    def _do_skip_test(self) -> None:
        self._skip_test = True
        self._refresh_nav()

    def _next(self) -> None:
        if self._step == STEP_DONE:
            self._finish()
            return
        self._step += 1
        self._show_step()

    def _back(self) -> None:
        self._step -= 1
        self._show_step()

    def _finish(self) -> None:
        self._cfg["api"] = self._api_fields.get_values()
        if self._language.value():
            self._cfg["target_language"] = self._language.value()
        self._cfg["hotkey"] = self._hotkey.value()
        self.completed = True
        self._win.destroy()

    def _cancel(self) -> None:
        self._win.destroy()


def run_wizard(root: tk.Tk, cfg: dict) -> bool:
    """顯示首次設定精靈並等待關閉；完成回 True（結果已寫入 cfg，呼叫端負責存檔）。"""
    wizard = SetupWizard(root, cfg)
    root.wait_window(wizard._win)
    return wizard.completed
