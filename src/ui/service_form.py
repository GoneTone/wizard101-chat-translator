"""單筆翻譯服務的編輯表單：名稱、服務商、該家的欄位、測試連線。
首次精靈（內嵌）與設定視窗的新增／編輯對話框共用同一份。"""
import tkinter as tk
from tkinter import ttk

from src.i18n import current_language, language_name, t
from src.log import log
from src.services import (
    API_EFFORTS,
    API_PROFILE_FIELDS,
    EFFORT_AUTO,
    PROVIDERS,
    is_auto_name,
    validate_service,
)
from src.translation.translator import test_translate
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
from src.ui.provider_picker import open_provider_picker
from src.ui.richtext import RichLabel, ttk_background


class ServiceForm(ttk.Frame):
    """一筆服務的編輯表單；`values()` 回傳可直接存進清單的服務 dict。"""

    def __init__(self, parent, service: dict, on_change=None):
        super().__init__(parent)
        self._on_change = on_change
        self.test_passed = False
        self._id = service["id"]
        self._provider = service["provider"]
        self._name_var = tk.StringVar(value=service.get("name", ""))
        self._api_key = tk.StringVar(value=service.get("api_key", ""))
        self._model = tk.StringVar(value=service.get("model", ""))
        self._base_url = tk.StringVar(value=service.get("base_url", ""))
        self._thinking = tk.BooleanVar(value=service.get("thinking", False))
        self._effort = tk.StringVar(value=service.get("effort", EFFORT_AUTO))
        for var in (self._api_key, self._model, self._base_url, self._effort):
            var.trace_add("write", lambda *_: self._invalidate_test())

        self._name_row()
        self._provider_row()
        self._fields = ttk.Frame(self)
        self._fields.pack(fill="x")
        self._test_row()
        # 精靈階段目標語言還沒選：退到介面語言的自稱（與 main.bootstrap_language 同一套預設）
        self._target_language_fn = lambda: language_name(current_language())
        self._rebuild_fields()

    # --- 值存取 ---
    def values(self) -> dict:
        """完整的一筆服務：id、名稱、服務商與該家的欄位。"""
        return {"id": self._id, "name": self._resolved_name(),
                "provider": self._provider, **self._field_values()}

    def api_values(self) -> dict:
        """攤平成 Translator／測試連線吃的形狀（去掉 id 與名稱）。"""
        return {"provider": self._provider, **self._field_values()}

    def set_name(self, name: str) -> None:
        self._name_var.set(name)

    def set_provider(self, provider: str) -> None:
        """換服務商：名稱跟著走、欄位清空重建，先前的測試結果作廢。"""
        if provider == self._provider:
            return
        previous = self._provider
        self._provider = provider
        self._provider_label.configure(text=t(PROVIDERS[provider].label_key))
        # 名稱還是上一家的自動值就跟著換；使用者取過名字就不覆蓋
        if is_auto_name(self._name_var.get().strip(), PROVIDERS[previous].short_name):
            self._name_var.set(PROVIDERS[provider].short_name)
        for var in (self._api_key, self._model, self._base_url, self._effort):
            var.set("")
        self._thinking.set(False)
        self._effort.set(EFFORT_AUTO)
        self._test_result.set("")
        log(f"[settings] service form provider switched {previous} -> {provider}")
        self._rebuild_fields()

    def set_target_language_fn(self, fn) -> None:
        """測試連線時取得目標語言的 callback（精靈階段語言還沒選，用預設）。"""
        self._target_language_fn = fn

    def _resolved_name(self) -> str:
        return self._name_var.get().strip() or PROVIDERS[self._provider].short_name

    def _field_values(self) -> dict:
        values = {"api_key": self._api_key.get().strip(),
                  "model": self._model.get().strip(),
                  "base_url": self._base_url.get().strip(),
                  "thinking": self._thinking.get(),
                  "effort": self._effort.get()}
        return {key: values[key] for key in API_PROFILE_FIELDS[self._provider]}

    # --- 版面 ---
    def _name_row(self) -> None:
        row = ttk.Frame(self)
        row.pack(fill="x", pady=(0, 6))
        ttk.Label(row, text=t("field.service_name"), width=LABEL_WIDTH).pack(side="left")
        ttk.Entry(row, textvariable=self._name_var).pack(side="left", fill="x", expand=True)

    def _provider_row(self) -> None:
        row = ttk.Frame(self)
        row.pack(fill="x", pady=(0, 6))
        ttk.Label(row, text=t("field.provider"), width=LABEL_WIDTH).pack(side="left")
        # 〔更改〕先 pack：後宣告會在視窗變窄時被 expand=True 的名稱標籤擠掉
        ttk.Button(row, text=t("button.change"), width=7,
                   command=self._change_provider).pack(side="right", padx=(4, 0))
        self._provider_label = ttk.Label(
            row, text=t(PROVIDERS[self._provider].label_key))
        self._provider_label.pack(side="left", fill="x", expand=True)

    def _change_provider(self) -> None:
        picked = open_provider_picker(self)
        if picked is not None:
            self.set_provider(picked)

    def _test_row(self) -> None:
        row = ttk.Frame(self)
        row.pack(fill="x", pady=(8, 0))
        self._test_btn = ttk.Button(row, text=t("button.test"), command=self._start_test)
        self._test_btn.pack(side="left")
        # 測試還在跑時改了欄位，舊結果回來時要丟掉，不能把新設定標成「已測過」
        self._test_task = BackgroundButton(self._test_btn, "test connection")
        self._test_result = RichLabel(row, fg=ERROR_COLOR, bg=ttk_background(self),
                                      font="TkDefaultFont")
        self._test_result.pack(side="left", fill="x", expand=True, padx=8)

    # --- 動態欄位 ---
    def _rebuild_fields(self) -> None:
        for widget in self._fields.winfo_children():
            widget.destroy()
        prov = PROVIDERS[self._provider]
        if prov.needs_base_url:
            # 依填寫順序排：網址→金鑰→模型。模型清單要靠前兩者才取得到。
            self._labeled_entry(t("field.base_url"), self._base_url)
            self._field_hint(t("hint.custom_endpoint"))
            self._labeled_entry(t("field.api_key_optional"), self._api_key, secret=True)
            self._model_row()
            self._thinking_row()
        else:
            self._labeled_entry(t("field.api_key"), self._api_key, secret=True)
            # 取金鑰的連結緊貼金鑰欄：它是這一欄的輔助說明
            link_label(self._fields, t("link.get_key"), prov.key_url).pack(
                anchor="w", pady=(2, 0))
            self._model_row()
            if prov.has_field("thinking"):
                self._thinking_row()
            if prov.has_field("effort"):
                self._effort_row()
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
        self._model_field = ModelField(self._fields, self._model, self.api_values)
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

    def _start_test(self) -> None:
        api = self.api_values()
        errors = validate_service(api)
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
