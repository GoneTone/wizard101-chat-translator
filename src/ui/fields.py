"""精靈與設定視窗共用的欄位群：服務商選擇＋API 欄位＋測試連線、熱鍵捕捉、
翻譯目標語言與介面語言選擇。小元件在 form.py、模型欄位在 model_field.py、
服務商資料與表單驗證在 providers.py。"""
import copy
import tkinter as tk
from tkinter import ttk

import keyboard

from src.config import API_EFFORTS, API_PROFILE_FIELDS, API_PROVIDERS, EFFORT_AUTO
from src.i18n import (
    DEFAULT_LANGUAGE,
    available_languages,
    current_language,
    language_name,
    t,
)
from src.log import log
from src.translation.translator import (
    test_translate,
)
from src.ui.form import (
    ERROR_COLOR,
    LABEL_WIDTH,
    BackgroundButton,
    friendly_error,
    hint_label,
    link_label,
    show_outcome,
)
from src.ui.model_field import ModelField
from src.ui.providers import PROVIDERS, validate_api_form
from src.ui.richtext import RichLabel, ttk_background

# 翻譯目標語言的常用選項：各語言的 endonym，任何介面語言下都不翻譯。
COMMON_LANGUAGES = ["繁體中文（台灣）", "简体中文（中国）", "English", "日本語",
                    "한국어", "Español", "Português", "Deutsch", "Français"]


class ApiFields(ttk.Frame):
    """API 設定欄位群：服務商 radio + 動態欄位 + 測試連線。"""

    def __init__(self, parent, initial: dict, on_change=None):
        super().__init__(parent)
        self._on_change = on_change
        self.test_passed = False
        # 每家一份設定都留在手上：切換服務商時只是換一份填進欄位，值不會互相蓋掉。
        self._profiles = {name: dict(initial[name]) for name in API_PROVIDERS}
        self._last_provider = initial["provider"]
        self._provider = tk.StringVar(value=initial["provider"])
        self._api_key = tk.StringVar()
        self._model = tk.StringVar()
        self._base_url = tk.StringVar()
        self._thinking = tk.BooleanVar()
        self._effort = tk.StringVar()
        self._load_profile(initial["provider"])
        for var in (self._api_key, self._model, self._base_url, self._effort):
            var.trace_add("write", lambda *_: self._invalidate_test())

        radio_row = ttk.Frame(self)
        radio_row.pack(fill="x", pady=(0, 6))
        for key, prov in PROVIDERS.items():
            ttk.Radiobutton(radio_row, text=t(prov.label_key), value=key,
                            variable=self._provider,
                            command=self._rebuild_fields).pack(anchor="w")

        self._fields = ttk.Frame(self)
        self._fields.pack(fill="x")

        test_row = ttk.Frame(self)
        test_row.pack(fill="x", pady=(8, 0))
        self._test_btn = ttk.Button(test_row, text=t("button.test"),
                                    command=self._start_test)
        self._test_btn.pack(side="left")
        # 測試還在跑時切了服務商或改了欄位，舊結果回來時要丟掉，不能把新的設定標成「已測過」
        self._test_task = BackgroundButton(self._test_btn, "test connection")
        self._test_result = RichLabel(test_row, fg=ERROR_COLOR, bg=ttk_background(self),
                                      font="TkDefaultFont")
        self._test_result.pack(side="left", fill="x", expand=True, padx=8)

        # 精靈階段目標語言還沒選：退到介面語言的自稱（與 main.bootstrap_language 同一套預設）
        self._target_language_fn = lambda: language_name(current_language())
        self._rebuild_fields()

    # --- 值存取 ---
    def get_values(self) -> dict:
        """完整的 api 區塊（provider ＋ 每家各一份）：存檔用。
        欄位上的值先歸還目前這家，其餘服務商的設定原樣帶著走。"""
        self._profiles[self._last_provider] = self._field_values(self._last_provider)
        return {"provider": self._provider.get(), **copy.deepcopy(self._profiles)}

    def active_values(self) -> dict:
        """目前這家的設定（扁平，含 provider）：表單驗證、取模型清單與測試連線用。"""
        provider = self._provider.get()
        return {"provider": provider, **self._field_values(provider)}

    def _field_values(self, provider: str) -> dict:
        """欄位上的值，只取這家有的那幾個（見 config.API_PROFILE_FIELDS）。
        provider 要明講：換家的當下欄位裡放的還是上一家的值。"""
        values = {"api_key": self._api_key.get().strip(),
                  "model": self._model.get().strip(),
                  "base_url": self._base_url.get().strip(),
                  "thinking": self._thinking.get(),
                  "effort": self._effort.get()}
        return {key: values[key] for key in API_PROFILE_FIELDS[provider]}

    def _load_profile(self, provider: str) -> None:
        profile = self._profiles[provider]
        self._api_key.set(profile["api_key"])
        self._model.set(profile["model"])
        self._base_url.set(profile.get("base_url", ""))
        self._thinking.set(profile.get("thinking", False))
        self._effort.set(profile.get("effort", EFFORT_AUTO))

    def set_target_language_fn(self, fn) -> None:
        """測試連線時取得目標語言的 callback（精靈階段語言還沒選，用預設）。"""
        self._target_language_fn = fn

    def _switch_profile(self) -> None:
        """換服務商：欄位上的值歸還上一家，再把新這家自己存的值填回欄位。
        連線測試結果不跟著搬（那是對上一家端點測出來的），已抓的模型清單隨欄位重建消失。"""
        self._profiles[self._last_provider] = self._field_values(self._last_provider)
        target = self._provider.get()
        self._load_profile(target)
        self._test_result.set("")
        profile = self._profiles[target]
        log(f"[settings] provider switched {self._last_provider} -> {target} "
            f"(model={profile['model'] or '-'}, has_key={bool(profile['api_key'])})")

    # --- 動態欄位 ---
    def _rebuild_fields(self) -> None:
        for w in self._fields.winfo_children():
            w.destroy()
        if self._provider.get() != self._last_provider:
            self._switch_profile()
        prov = PROVIDERS[self._provider.get()]
        if prov.needs_base_url:
            # 依填寫順序排：網址→金鑰→模型。模型清單要靠前兩者才取得到，
            # 且這樣與官方端點那一支（金鑰在模型之前）一致，切換服務商時欄位不跳動。
            self._labeled_entry(t("field.base_url"), self._base_url)
            self._field_hint(t("hint.custom_endpoint"))
            self._labeled_entry(t("field.api_key_optional"), self._api_key, secret=True)
            self._model_row()
            self._thinking_row()
        else:
            self._labeled_entry(t("field.api_key"), self._api_key, secret=True)
            # 取金鑰的連結緊貼金鑰欄：它是這一欄的輔助說明，擺到最後會與要幫的欄位分家
            link_label(self._fields, t("link.get_key"), prov.key_url).pack(
                anchor="w", pady=(2, 0))
            self._model_row()
            if prov.has_field("thinking"):
                # ChatGPT 官方端點可關思考（只送 reasoning_effort，見 translator）。
                self._thinking_row()
            if prov.has_field("effort"):
                self._effort_row()
        self._last_provider = self._provider.get()
        self._invalidate_test()
        if self._on_change:
            self._on_change()

    def _field_hint(self, text: str) -> None:
        """欄位下方的說明：左邊留一個與標籤等寬的空位，讓文字左緣對齊輸入框，
        與模型欄（走 grid，說明本來就落在輸入框那一欄）的觀感一致。"""
        row = ttk.Frame(self._fields)
        row.pack(fill="x")
        ttk.Label(row, width=LABEL_WIDTH).pack(side="left")
        hint_label(row, text).pack(side="left", fill="x", expand=True)

    def _model_row(self) -> None:
        """模型欄（三家共用）：每次重建都是新元件，切換服務商時已抓的清單自然清空。"""
        self._model_field = ModelField(self._fields, self._model, self.active_values)
        self._model_field.pack(fill="x")

    def _effort_row(self) -> None:
        """思考深度（Claude）：選項是「自動／精簡」而不是開關 —— Claude 沒有完全不
        思考這個選項，做成與另兩家一樣的勾選只會讓人以為關得掉。
        下拉顯示的是譯文，存回設定的是 API_EFFORTS 的代碼。"""
        row = ttk.Frame(self._fields)
        row.pack(fill="x", pady=2)
        ttk.Label(row, text=t("field.effort"), width=LABEL_WIDTH).pack(side="left")
        names = [t(f"effort.{code}") for code in API_EFFORTS]
        current = self._effort.get() if self._effort.get() in API_EFFORTS else EFFORT_AUTO
        # 顯示用的變數要留在 self 上：只被 Combobox 參照的話會被 GC，欄位就空掉。
        self._effort_shown = tk.StringVar(value=t(f"effort.{current}"))
        combo = ttk.Combobox(row, textvariable=self._effort_shown, values=names,
                             state="readonly")
        combo.pack(side="left", fill="x", expand=True)
        combo.bind("<<ComboboxSelected>>", lambda e: self._effort.set(
            API_EFFORTS[names.index(self._effort_shown.get())]))
        self._field_hint(t("hint.effort"))

    def _thinking_row(self) -> None:
        """思考開關＋為何建議關閉的說明（支援思考開關的服務商共用）。"""
        ttk.Checkbutton(self._fields, text=t("field.thinking"),
                        variable=self._thinking).pack(anchor="w", pady=(2, 0))
        hint_label(self._fields, t("hint.thinking")).pack(fill="x", padx=(20, 0))

    def _labeled_entry(self, label: str, var: tk.StringVar, secret: bool = False):
        row = ttk.Frame(self._fields)
        row.pack(fill="x", pady=2)
        ttk.Label(row, text=label, width=LABEL_WIDTH).pack(side="left")
        entry = ttk.Entry(row, textvariable=var, show="●" if secret else "")
        if secret:
            # 「顯示」先 pack：後宣告會在視窗變窄時被 expand=True 的輸入框擠掉
            btn = ttk.Button(row, text=t("button.show"), width=5,
                             command=lambda: entry.configure(
                                 show="" if entry.cget("show") else "●"))
            btn.pack(side="right", padx=(4, 0))
        entry.pack(side="left", fill="x", expand=True)

    # --- 測試連線 ---
    def _invalidate_test(self) -> None:
        self.test_passed = False
        self._test_task.invalidate()

    def clear_test_result(self) -> None:
        """作廢已顯示的測試結果：那句譯文是用當時的目標語言翻的，語言一改就不算數。"""
        self._invalidate_test()
        self._test_result.set("")
        if self._on_change:
            self._on_change()

    def _start_test(self) -> None:
        api = self.active_values()
        errors = validate_api_form(api)
        if errors:
            self._show_test_result(False, t("sep.errors").join(t(e) for e in errors))
            return
        self._test_result.set("")
        target = self._target_language_fn()
        self._test_task.start(lambda: test_translate(api, target),
                              lambda result: self._on_tested(result, api),
                              t("button.testing"))

    def _on_tested(self, result, api: dict) -> None:
        """測試連線的結果：譯出的樣句，或拋出的例外。"""
        if isinstance(result, Exception):
            log(f"[settings] test connection failed (provider={api['provider']}, "
                f"model={api['model']}): {result}")
            key, kwargs = friendly_error(result)
            self._show_test_result(False, t(key, **kwargs))
            return
        log(f"[settings] test connection ok (provider={api['provider']}, "
            f"model={api['model']})")
        self._show_test_result(True, t("test.success", sample=result))

    def _show_test_result(self, ok: bool, message: str) -> None:
        self.test_passed = ok
        show_outcome(self._test_result, ok, message)
        if self._on_change:
            self._on_change()


class HotkeyField(ttk.Frame):
    """熱鍵欄位：顯示目前值，點「更改」後按下組合鍵即設定（Esc 取消）。
    捕捉用 keyboard 套件的低階鍵盤鉤子而非 tk 事件：Ctrl+Space 等組合會先被
    輸入法（IME）或系統攔截、tk 收不到；低階鉤子在 IME 之前就能看到按鍵，
    且與實際註冊熱鍵走同一條路 —— 捕捉得到就保證註冊得到。"""

    def __init__(self, parent, initial: str):
        super().__init__(parent)
        self._value = initial
        self._label = ttk.Label(self, text=initial)
        self._label.pack(side="left")
        self._btn = ttk.Button(self, text=t("button.change"), width=14,
                               command=self._capture)
        self._btn.pack(side="left", padx=8)
        self._task = BackgroundButton(self._btn, "hotkey capture")

    def value(self) -> str:
        return self._value

    def set_value(self, s: str) -> None:
        self._value = s
        self._label.configure(text=s)

    def _capture(self) -> None:
        self._task.start(lambda: keyboard.read_hotkey(suppress=False), self._on_captured,
                         t("button.press_key"))

    def _on_captured(self, combo) -> None:
        if isinstance(combo, Exception):
            log(f"[settings] hotkey capture failed: {combo}")
            return
        if combo and combo != "esc":
            self.set_value(combo)


class LanguageField(ttk.Frame):
    """翻譯目標語言：常用語言下拉 + 可自行輸入。"""

    def __init__(self, parent, initial: str, on_change=None):
        super().__init__(parent)
        self._var = tk.StringVar(value=initial)
        if on_change is not None:
            # 下拉選取與手動輸入都要通知：測試連線的譯文是用某個目標語言翻出來的，
            # 語言一改那句譯文就過期了（見 ApiFields.clear_test_result）。
            self._var.trace_add("write", lambda *_: on_change())
        combo = ttk.Combobox(self, textvariable=self._var, values=COMMON_LANGUAGES)
        combo.pack(fill="x")
        hint_label(self, t("hint.language")).pack(fill="x", pady=(2, 0))

    def value(self) -> str:
        return self._var.get().strip()

    def set_value(self, s: str) -> None:
        self._var.set(s)


class UiLanguageField(ttk.Frame):
    """介面語言：唯讀下拉，顯示 endonym、對外進出語言碼 —— 與 LanguageField
    （翻譯目標語言，可自由輸入）是不同用途的兩個欄位。"""

    def __init__(self, parent, initial: str, on_change=None):
        super().__init__(parent)
        self._on_change = on_change
        languages = available_languages()
        self._names = list(languages.values())
        self._codes = list(languages)
        self._var = tk.StringVar(value=language_name(
            initial if initial in languages else DEFAULT_LANGUAGE))
        combo = ttk.Combobox(self, textvariable=self._var, values=self._names,
                             state="readonly")
        combo.pack(fill="x")
        combo.bind("<<ComboboxSelected>>", lambda e: self._notify())

    def value(self) -> str:
        """目前選到的語言碼。"""
        name = self._var.get()
        return self._codes[self._names.index(name)] if name in self._names \
            else DEFAULT_LANGUAGE

    def set_value(self, code: str) -> None:
        self._var.set(language_name(
            code if code in available_languages() else DEFAULT_LANGUAGE))

    def _notify(self) -> None:
        if self._on_change is not None:
            self._on_change(self.value())
