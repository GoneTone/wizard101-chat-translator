"""首次設定精靈：介面語言 → API 設定（選服務商 → 填 API → 測試連線）→ 偏好設定，
三步完成寫入 cfg。中途關閉＝取消（不留半套設定），run_wizard 回傳 False。"""
import sys
import tkinter as tk
from tkinter import ttk

from src.config import app_name
from src.i18n import current_language, language_name, set_language, t
from src.ui.fields import (HINT_COLOR, ApiFields, HotkeyField, LanguageField,
                           UiLanguageField, validate_api_form)
from src.ui.fonts import ui_font
from src.ui.responsive import bind_wrap
from src.ui.scrollable import ScrollableFrame

STEP_LANG, STEP_API, STEP_PREFS = 0, 1, 2
_STEP_KEYS = ["wizard.step.language", "wizard.step.api", "wizard.step.prefs"]

MIN_HEIGHT = 380  # 視窗高度下限：步驟內容可捲動，只需容得下步驟標題與導覽列


def can_advance(step: int, api_test_passed: bool, api_errors: list[str]) -> bool:
    """該步驟是否允許按「下一步」。
    api_test_passed 已含「使用者明示略過測試」的情形（見 SetupWizard._refresh_nav）。"""
    if step == STEP_API:
        return not api_errors and api_test_passed
    return True


class SetupWizard:
    """精靈視窗本體。completed 屬性表示是否走完全部步驟；restart 表示語言頁換過語言，
    需要以新語言重建整個精靈（見 run_wizard）。"""

    def __init__(self, root: tk.Tk, cfg: dict):
        self._cfg = cfg
        self.completed = False
        self.restart = False
        self._step = STEP_LANG
        self._skip_test = False

        self._win = tk.Toplevel(root)
        self._win.title(t("wizard.title", app=app_name()))
        # 預設高度留給第一步：說明＋服務商＋欄位＋思考說明＋測試列已達 460px，
        # 測試結果訊息（尤其多行錯誤）還會再撐高。
        win_w, win_h = 540, 540
        x = (self._win.winfo_screenwidth() - win_w) // 2
        y = (self._win.winfo_screenheight() - win_h) // 2
        self._win.geometry(f"{win_w}x{win_h}+{x}+{y}")
        self._win.resizable(True, True)
        # 寬度下限維持開窗值（再窄是橫向擠壓，捲動救不了）；高度放寬，步驟內容可捲動
        self._win.minsize(win_w, MIN_HEIGHT)
        self._win.protocol("WM_DELETE_WINDOW", self._cancel)

        self._indicator = ttk.Label(self._win, text="")
        self._indicator.pack(pady=(10, 0))
        self._title = ttk.Label(self._win, font=ui_font(13, "bold"))
        self._title.pack(pady=(2, 8))

        # 導覽列先 pack：後宣告會在視窗變矮時被 expand=True 的內容區擠掉
        nav = ttk.Frame(self._win, padding=8)
        nav.pack(side="bottom", fill="x")
        self._back_btn = ttk.Button(nav, text=t("button.back"), command=self._back)
        self._back_btn.pack(side="left")
        self._next_btn = ttk.Button(nav, text=t("button.next"), command=self._next)
        self._next_btn.pack(side="right")

        self._body_scroll = ScrollableFrame(self._win, padding=16)
        self._body = self._body_scroll.body   # 步驟內容都放這裡，捲動由外層處理
        self._body_scroll.pack(fill="both", expand=True)

        # 跨步驟保留的欄位元件（建一次，切步驟時搬進／搬出 body）
        self._ui_language = UiLanguageField(self._body, current_language(),
                                            on_change=self._on_language_change)
        self._api_fields = ApiFields(self._body, cfg["api"], on_change=self._on_api_change)
        self._language = LanguageField(self._body, cfg["target_language"])
        self._hotkey = HotkeyField(self._body, cfg["hotkey"])
        self._auto_input = tk.BooleanVar(value=cfg["auto_show_input"])
        self._show_step()

    # --- 導航 ---
    def _show_step(self) -> None:
        # 跨步驟保留的元件只收起來；每步臨時建立的說明等直接銷毀，免得來回導航累積孤兒
        persistent = {self._ui_language, self._api_fields, self._language, self._hotkey}
        for w in self._body.winfo_children():
            if w in persistent:
                w.pack_forget()
            else:
                w.destroy()
        self._indicator.configure(text="  ".join(
            "●" if i <= self._step else "○" for i in range(len(_STEP_KEYS))))
        self._title.configure(text=t(_STEP_KEYS[self._step]))

        if self._step == STEP_LANG:
            hint = ttk.Label(self._body, text=t("wizard.language_hint"), justify="left")
            hint.pack(fill="x", pady=(0, 8))
            bind_wrap(hint)
            self._ui_language.pack(fill="x")
            self._next_btn.configure(text=t("button.next"))
        elif self._step == STEP_API:
            intro = ttk.Label(self._body, text=t("wizard.intro"), justify="left")
            intro.pack(fill="x")
            bind_wrap(intro)
            self._api_fields.pack(fill="x", pady=(10, 0))
            skip = ttk.Label(self._body, text=t("wizard.skip_test"), foreground=HINT_COLOR,
                             cursor="hand2", font=ui_font(8))
            skip.pack(anchor="e", pady=(6, 0))
            skip.bind("<Button-1>", lambda e: self._do_skip_test())
            self._next_btn.configure(text=t("button.next"))
        else:  # STEP_PREFS
            ttk.Label(self._body, text=t("wizard.target_language")).pack(anchor="w")
            self._language.pack(fill="x", pady=(2, 12))
            ttk.Label(self._body, text=t("settings.hotkey")).pack(anchor="w")
            self._hotkey.pack(anchor="w", pady=(2, 0))
            ttk.Checkbutton(self._body, text=t("field.auto_input"),
                            variable=self._auto_input).pack(anchor="w", pady=(14, 0))
            self._next_btn.configure(text=t("button.finish"))
        self._back_btn.configure(
            state="normal" if self._step > STEP_LANG else "disabled")
        self._refresh_nav()

    def _on_language_change(self, code: str) -> None:
        """語言一改就整個精靈重建：跨步驟保留的欄位元件已帶著舊語言的標籤，
        逐一刷新容易漏掉，重建最保險（代價是 API 測試狀態要重測）。"""
        if code == current_language():
            return
        old_default = language_name(current_language())
        self._collect_into_cfg()
        set_language(code)
        self._cfg["ui_language"] = code
        # 使用者還沒動過翻譯目標語言時，讓它跟著介面語言走；動過就不覆蓋。
        if self._cfg["target_language"] == old_default:
            self._cfg["target_language"] = language_name(code)
        self.restart = True
        print(f"[ui] wizard restarting with language {code}", file=sys.stderr)
        # after_idle：此處在 <<ComboboxSelected>> 事件內，ttk 類別 binding 還在處理同一事件，
        # 立即 destroy() 會讓它收尾時碰到已死的 widget（TclError: invalid command name）
        self._win.after_idle(self._win.destroy)

    def _collect_into_cfg(self) -> None:
        """把目前填在欄位裡的值寫回 cfg（重建精靈與完成精靈共用）。"""
        self._cfg["api"] = self._api_fields.get_values()
        if self._language.value():
            self._cfg["target_language"] = self._language.value()
        self._cfg["hotkey"] = self._hotkey.value()
        self._cfg["auto_show_input"] = self._auto_input.get()

    def _on_api_change(self) -> None:
        # API 欄位一改就取消先前的「略過測試」：改過設定應重測（或再次明示略過）
        self._skip_test = False
        self._refresh_nav()

    def _refresh_nav(self) -> None:
        if not hasattr(self, "_api_fields"):
            return  # ApiFields 建構中觸發的第一次 on_change：欄位元件尚未掛上 self，略過
        api = self._api_fields.active_values()
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
        self._collect_into_cfg()
        self._cfg["ui_language"] = current_language()
        self.completed = True
        self._win.destroy()

    def _cancel(self) -> None:
        self._win.destroy()


def run_wizard(root: tk.Tk, cfg: dict) -> bool:
    """顯示首次設定精靈並等待關閉；完成回 True（結果已寫入 cfg，呼叫端負責存檔）。
    使用者在語言頁換語言時，精靈會以新語言重建（見 SetupWizard._on_language_change）。"""
    while True:
        wizard = SetupWizard(root, cfg)
        root.wait_window(wizard._win)
        if not wizard.restart:
            return wizard.completed
