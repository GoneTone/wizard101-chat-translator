"""首次設定精靈：API 設定（選服務商 → 填 API → 測試連線）→ 偏好設定，
兩步完成寫入 cfg。中途關閉＝取消（不留半套設定），run_wizard 回傳 False。"""
import tkinter as tk
from tkinter import ttk

from src.config import APP_NAME
from src.ui.fields import (AUTO_INPUT_LABEL, ApiFields, HotkeyField, LanguageField,
                           validate_api_form)
from src.ui.responsive import bind_wrap

STEP_API, STEP_PREFS = 0, 1
_TITLES = ["API 設定", "偏好設定"]

_INTRO = ("本工具會即時翻譯 Wizard101 的遊戲聊天，並可用熱鍵輸入你的語言、"
          "翻成英文送進遊戲。\n請選擇翻譯服務並填好設定，按「測試連線」確認可用"
          "（會實際翻譯一句測試文字）。")


def can_advance(step: int, api_test_passed: bool, api_errors: list[str]) -> bool:
    """該步驟是否允許按「下一步」。
    api_test_passed 已含「使用者明示略過測試」的情形（見 SetupWizard._refresh_nav）。"""
    if step == STEP_API:
        return not api_errors and api_test_passed
    return True


class SetupWizard:
    """精靈視窗本體。completed 屬性表示是否走完全部步驟。"""

    def __init__(self, root: tk.Tk, cfg: dict):
        self._cfg = cfg
        self.completed = False
        self._step = STEP_API
        self._skip_test = False

        self._win = tk.Toplevel(root)
        self._win.title(f"{APP_NAME} — 首次設定")
        # 高度留給第一步：說明＋服務商＋欄位＋思考說明＋測試列已達 460px，
        # 測試結果訊息（尤其多行錯誤）還會再撐高，太緊會把「略過測試」擠出畫面。
        win_w, win_h = 540, 540
        x = (self._win.winfo_screenwidth() - win_w) // 2
        y = (self._win.winfo_screenheight() - win_h) // 2
        self._win.geometry(f"{win_w}x{win_h}+{x}+{y}")
        self._win.resizable(True, True)
        self._win.minsize(win_w, win_h)  # 下限＝預設尺寸：再窄會把「略過測試」與測試結果擠出畫面
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
        self._api_fields = ApiFields(self._body, cfg["api"], on_change=self._on_api_change)
        self._language = LanguageField(self._body, cfg["target_language"])
        self._hotkey = HotkeyField(self._body, cfg["hotkey"])
        self._auto_input = tk.BooleanVar(value=cfg["auto_show_input"])
        self._show_step()

    # --- 導航 ---
    def _show_step(self) -> None:
        # 跨步驟保留的元件只收起來；每步臨時建立的說明文字等直接銷毀，
        # 避免來回導航時在 body 底下累積孤兒 widget。
        persistent = {self._api_fields, self._language, self._hotkey}
        for w in self._body.winfo_children():
            if w in persistent:
                w.pack_forget()
            else:
                w.destroy()
        self._indicator.configure(text="  ".join(
            "●" if i <= self._step else "○" for i in range(len(_TITLES))))
        self._title.configure(text=_TITLES[self._step])

        if self._step == STEP_API:
            intro = ttk.Label(self._body, text=_INTRO, justify="left")
            intro.pack(fill="x")
            bind_wrap(intro)
            self._api_fields.pack(fill="x", pady=(10, 0))
            skip = ttk.Label(self._body, text="略過測試", foreground="#888888",
                             cursor="hand2", font=("Microsoft JhengHei", 8))
            skip.pack(anchor="e", pady=(6, 0))
            skip.bind("<Button-1>", lambda e: self._do_skip_test())
            self._next_btn.configure(text="下一步")
        else:  # STEP_PREFS
            ttk.Label(self._body, text="翻譯目標語言（收到的訊息翻成什麼語言）").pack(anchor="w")
            self._language.pack(fill="x", pady=(2, 12))
            ttk.Label(self._body, text="呼出輸入框的熱鍵").pack(anchor="w")
            self._hotkey.pack(anchor="w", pady=(2, 0))
            ttk.Checkbutton(self._body, text=AUTO_INPUT_LABEL,
                            variable=self._auto_input).pack(anchor="w", pady=(14, 0))
            self._next_btn.configure(text="完成")
        self._back_btn.configure(
            state="normal" if self._step > STEP_API else "disabled")
        self._refresh_nav()

    def _on_api_change(self) -> None:
        # API 欄位有任何變動就取消先前的「略過測試」：改過設定應重新測試（或再次明示略過）。
        self._skip_test = False
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
        if self._step == STEP_PREFS:
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
        self._cfg["auto_show_input"] = self._auto_input.get()
        self.completed = True
        self._win.destroy()

    def _cancel(self) -> None:
        self._win.destroy()


def run_wizard(root: tk.Tk, cfg: dict) -> bool:
    """顯示首次設定精靈並等待關閉；完成回 True（結果已寫入 cfg，呼叫端負責存檔）。"""
    wizard = SetupWizard(root, cfg)
    root.wait_window(wizard._win)
    return wizard.completed
