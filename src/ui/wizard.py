"""首次設定精靈：介面語言 → API 設定（選服務商 → 填 API → 測試連線）→ 偏好設定，
三步完成寫入 cfg。中途關閉＝取消（不留半套設定），run_wizard 回傳 False。

第二步有兩種樣態：還沒有服務可編輯時先出服務商卡片，選過之後才建出表單。
`_service_form is None` 就是「還在卡片」的唯一依據。"""
import copy
import tkinter as tk
from tkinter import messagebox, ttk

from src.config import app_name
from src.i18n import current_language, language_name, set_language, t
from src.log import log
from src.services import (
    PROVIDERS,
    SLOTS,
    find,
    is_auto_name,
    new_service,
    unique_name,
    validate_service,
)
from src.ui.fields import HotkeyField, LanguageField, UiLanguageField
from src.ui.fonts import ui_font
from src.ui.form import HINT_COLOR, help_translate_link, translators_row
from src.ui.geometry import centered_position
from src.ui.provider_picker import ProviderCards
from src.ui.responsive import bind_wrap
from src.ui.scrollable import ScrollableFrame
from src.ui.service_form import ServiceForm

STEP_LANG, STEP_API, STEP_PREFS = 0, 1, 2
_STEP_KEYS = ["wizard.step.language", "wizard.step.service", "wizard.step.prefs"]

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
        x, y = centered_position(self._win.winfo_screenwidth(),
                                 self._win.winfo_screenheight(), win_w, win_h)
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
        # 編輯既有的預設服務而不是每次開一張新的：換語言會重建整個精靈
        # （見 _on_language_change），而設定壞掉時也是從這裡救回來的
        existing = find(cfg, cfg["default_service"])
        if existing is None and cfg["services"]:
            existing = cfg["services"][0]
        # 有服務可編輯就直接進表單，服務商早就選過了；沒有才從卡片問起
        self._service_form = None
        if existing is not None:
            self._build_service_form(copy.deepcopy(existing))
        self._language = LanguageField(self._body, cfg["target_language"])
        self._hotkey = HotkeyField(self._body, cfg["hotkey"])
        self._auto_input = tk.BooleanVar(value=cfg["auto_show_input"])
        self._region_hotkey = HotkeyField(self._body, cfg["region_hotkey"])
        self._show_step()

    # --- 導航 ---
    def _show_step(self) -> None:
        # 跨步驟保留的元件只收起來；每步臨時建立的說明等直接銷毀，免得來回導航累積孤兒
        persistent = {self._ui_language, self._language, self._hotkey,
                      self._region_hotkey, self._service_form}
        for w in self._body.winfo_children():
            if w in persistent:
                w.pack_forget()
            else:
                w.destroy()
        self._indicator.configure(text="  ".join(
            "●" if i <= self._step else "○" for i in range(len(_STEP_KEYS))))
        self._title.configure(text=t(_STEP_KEYS[self._step]))
        # 剛剛連同其他臨時元件一起被銷毀了
        self._translators_row = self._help_translate_link = None

        if self._step == STEP_LANG:
            hint = ttk.Label(self._body, text=t("wizard.language_hint"), justify="left")
            hint.pack(fill="x", pady=(0, 8))
            bind_wrap(hint)
            self._ui_language.pack(fill="x")
            # 每次進這一步重建：離開本步驟時會連同其他臨時元件一起被銷毀
            self._translators_row = translators_row(self._body)
            if self._translators_row is not None:
                self._translators_row.pack(anchor="w", pady=(4, 0))
            self._help_translate_link = help_translate_link(self._body)
            self._help_translate_link.pack(anchor="w", pady=(4, 0))
            self._next_btn.configure(text=t("button.next"))
        elif self._step == STEP_API:
            intro = ttk.Label(self._body, text=t("wizard.intro"), justify="left")
            intro.pack(fill="x")
            bind_wrap(intro)
            if self._service_form is None:
                self._provider_step()
            else:
                self._form_step()
            self._next_btn.configure(text=t("button.next"))
        else:  # STEP_PREFS
            ttk.Label(self._body, text=t("wizard.target_language")).pack(anchor="w")
            self._language.pack(fill="x", pady=(2, 12))
            ttk.Label(self._body, text=t("settings.hotkey")).pack(anchor="w")
            self._hotkey.pack(anchor="w", pady=(2, 0))
            # 與設定視窗同一個順序：每把熱鍵底下緊接著它自己的選項
            ttk.Checkbutton(self._body, text=t("field.auto_input"),
                            variable=self._auto_input).pack(anchor="w", pady=(6, 0))
            ttk.Label(self._body, text=t("settings.region_hotkey")).pack(anchor="w",
                                                                         pady=(10, 0))
            self._region_hotkey.pack(anchor="w", pady=(2, 0))
            self._next_btn.configure(text=t("button.finish"))
        self._back_btn.configure(
            state="normal" if self._step > STEP_LANG else "disabled")
        self._refresh_nav()

    def _provider_step(self) -> None:
        """第二步的前半：還沒有服務時先選服務商（卡片是臨時元件，切步驟時銷毀）。"""
        ask = ttk.Label(self._body, text=t("wizard.pick_provider"), justify="left")
        ask.pack(fill="x", pady=(10, 4))
        bind_wrap(ask)
        ProviderCards(self._body, self._pick_provider).pack(fill="x")

    def _form_step(self) -> None:
        """第二步的後半：服務商選好了，編輯那一筆服務。"""
        # 填設定與測試連線要等表單真的畫出來才說得通，前半只有服務商卡片
        howto = ttk.Label(self._body, text=t("wizard.intro_form"), justify="left")
        howto.pack(fill="x", pady=(10, 0))
        bind_wrap(howto)
        self._service_form.pack(fill="x", pady=(10, 0))
        skip = ttk.Label(self._body, text=t("wizard.skip_test"), foreground=HINT_COLOR,
                         cursor="hand2", font=ui_font(8))
        skip.pack(anchor="e", pady=(6, 0))
        skip.bind("<Button-1>", lambda e: self._do_skip_test())

    def _pick_provider(self, provider: str) -> None:
        """選好服務商：建出表單並重畫本步驟，卡片隨其他臨時元件一起被銷毀。"""
        self._build_service_form(new_service(provider, self._cfg["services"]))
        log(f"[ui] wizard provider picked: {provider}")
        self._show_step()

    def _build_service_form(self, draft: dict) -> None:
        """建立第二步的服務表單（選過服務商之後才有）。"""
        self._service_form = ServiceForm(self._body, draft, self._cfg["services"],
                                         on_change=self._on_api_change)

    def _on_language_change(self, code: str) -> None:
        """語言一改就整個精靈重建：跨步驟保留的欄位元件已帶著舊語言的標籤，
        逐一刷新容易漏掉，重建最保險（代價是 API 測試狀態要重測）。"""
        if code == current_language():
            return
        old_default = language_name(current_language())
        self._collect_into_cfg()
        # 依 id 認人：清單裡不只精靈編輯的這一筆，位置不可假設
        service = (find(self._cfg, self._service_form.values()["id"])
                   if self._service_form is not None else None)
        old_short_name = PROVIDERS[service["provider"]].short_name if service else None
        set_language(code)
        self._cfg["ui_language"] = code
        # 使用者還沒動過翻譯目標語言時，讓它跟著介面語言走；動過就不覆蓋。
        if self._cfg["target_language"] == old_default:
            self._cfg["target_language"] = language_name(code)
        # 服務名稱同理：還是自動取的服務商短名就跟著換，使用者取過名字就不碰。
        if service is not None and is_auto_name(service["name"], old_short_name):
            # 一樣要過去重：清單裡可能已經有一筆叫新語言的短名
            service["name"] = unique_name(PROVIDERS[service["provider"]].short_name,
                                          self._cfg["services"],
                                          ignore_id=service["id"])
        self.restart = True
        log(f"[ui] wizard restarting with language {code}")
        # after_idle：此處在 <<ComboboxSelected>> 事件內，ttk 類別 binding 還在處理同一事件，
        # 立即 destroy() 會讓它收尾時碰到已死的 widget（TclError: invalid command name）
        self._win.after_idle(self._win.destroy)

    def _collect_into_cfg(self) -> None:
        """把目前填在欄位裡的值寫回 cfg（重建精靈與完成精靈共用）。
        還停在服務商卡片時沒有服務可寫，其餘欄位照寫。"""
        if self._service_form is not None:
            self._collect_service()
        if self._language.value():
            self._cfg["target_language"] = self._language.value()
        self._cfg["hotkey"] = self._hotkey.value()
        self._cfg["auto_show_input"] = self._auto_input.get()
        self._cfg["region_hotkey"] = self._region_hotkey.value()

    def _collect_service(self) -> None:
        """把編輯中的服務併回 cfg 的清單，並讓它成為預設服務。"""
        service = self._service_form.values()
        services = self._cfg["services"]
        # 清單裡永遠不會出現兩個一樣的名稱：三個分派下拉只顯示名稱，同名會讓人選錯
        service["name"] = unique_name(service["name"], services, ignore_id=service["id"])
        if not services:
            self._cfg["service_slots"] = {slot: None for slot in SLOTS}
        # 依 id 併回清單而不是整份取代：精靈也是設定壞掉時的救援路徑，
        # 不能順手把其他服務與它們的金鑰一起刪掉
        for index, existing in enumerate(services):
            if existing["id"] == service["id"]:
                services[index] = service
                break
        else:
            services.append(service)
        self._cfg["default_service"] = service["id"]

    def _on_api_change(self) -> None:
        # API 欄位一改就取消先前的「略過測試」：改過設定應重測（或再次明示略過）
        self._skip_test = False
        self._refresh_nav()

    def _refresh_nav(self) -> None:
        if self._service_form is None:
            # 還在選服務商（或表單正在建構中）：沒有服務可驗，第二步一律擋住。
            # 其餘步驟放行的前提是「沒有表單就只可能停在 STEP_API」—— 偏好設定頁只能
            # 經由被擋住的 _next 抵達；步驟順序若改動，這裡要重新檢查。
            self._next_btn.configure(
                state="disabled" if self._step == STEP_API else "normal")
            return
        api = self._service_form.api_values()
        ok = can_advance(self._step,
                         self._service_form.test_passed or self._skip_test,
                         validate_service(api))
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
        # 與設定視窗同一條驗證：兩把熱鍵相同會一起觸發，開輸入框的同時也開選取層
        if self._hotkey.value() == self._region_hotkey.value():
            messagebox.showwarning(t("dialog.incomplete_title"), t("error.hotkeys_same"),
                                   parent=self._win)
            return
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
