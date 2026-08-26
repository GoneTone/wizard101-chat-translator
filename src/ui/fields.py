"""精靈與設定視窗共用的欄位元件與純邏輯：
服務商選擇、API 欄位、測試連線、熱鍵捕捉、語言選擇。"""
import queue
import sys
import threading
import tkinter as tk
import webbrowser

import keyboard
from dataclasses import dataclass, field
from tkinter import ttk

from src.translator import TranslatorConfigError, TranslatorOffline, test_translate
from src.ui.responsive import bind_wrap


@dataclass(frozen=True)
class Provider:
    label: str
    needs_base_url: bool
    models: list[str] = field(default_factory=list)
    key_url: str | None = None


PROVIDERS: dict[str, Provider] = {
    "openai": Provider(label="ChatGPT（OpenAI）", needs_base_url=False,
                       models=["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"],
                       key_url="https://platform.openai.com/api-keys"),
    "claude": Provider(label="Claude（Anthropic）", needs_base_url=False,
                       models=["claude-opus-5", "claude-sonnet-5", "claude-haiku-4-5"],
                       key_url="https://console.anthropic.com/settings/keys"),
    "custom": Provider(label="自訂端點（進階）", needs_base_url=True),
}

COMMON_LANGUAGES = ["繁體中文（台灣）", "简体中文（中国）", "日本語", "한국어",
                    "Español", "Português", "Deutsch", "Français"]

CLAUDE_MODEL_HINT = "較快較省：claude-haiku-4-5"

LANGUAGE_HINT = "清單只是常用選項，也可以直接輸入任何語言名稱（例如 Italiano、ภาษาไทย）。"

THINKING_HINT = "建議關閉：開啟可能會讓每則翻譯慢上數秒、也更耗 Token。"

# 精靈與設定視窗共用同一句，避免兩邊文案走鐘
AUTO_INPUT_LABEL = "遊戲開啟聊天輸入框時自動呼出翻譯輸入（關閉時自動收回）"


def validate_api_form(api: dict) -> list[str]:
    """檢查 API 表單必填欄位，回傳錯誤訊息列表（空＝通過）。"""
    errors = []
    provider = PROVIDERS[api["provider"]]
    if not api["model"].strip():
        errors.append("請選擇或輸入模型")
    if not provider.needs_base_url and not api["api_key"].strip():
        errors.append("請輸入 API 金鑰")
    if provider.needs_base_url and not api["base_url"].strip():
        errors.append("請輸入伺服器網址")
    return errors


def friendly_error(exc: Exception) -> str:
    """把翻譯例外轉成一般使用者看得懂的錯誤訊息。"""
    if isinstance(exc, TranslatorConfigError):
        if exc.status in (401, 403):
            return "金鑰無效或過期，請確認 API 金鑰"
        if exc.status == 404:
            return "找不到模型，請確認模型名稱"
        return f"API 設定有誤（HTTP {exc.status}），請檢查各欄位"
    if isinstance(exc, TranslatorOffline):
        return "無法連線到伺服器，請檢查網址與網路"
    return f"發生錯誤：{exc}"




class ApiFields(ttk.Frame):
    """API 設定欄位群：服務商 radio + 動態欄位 + 測試連線。"""

    def __init__(self, parent, initial: dict, on_change=None):
        super().__init__(parent)
        self._on_change = on_change
        self.test_passed = False
        self._queue: queue.Queue = queue.Queue()  # 測試結果由背景執行緒送回主執行緒
        self._last_provider = initial["provider"]
        self._provider = tk.StringVar(value=initial["provider"])
        self._api_key = tk.StringVar(value=initial["api_key"])
        self._model = tk.StringVar(value=initial["model"])
        self._base_url = tk.StringVar(value=initial["base_url"])
        self._thinking = tk.BooleanVar(value=initial["thinking"])
        for var in (self._api_key, self._model, self._base_url):
            var.trace_add("write", lambda *_: self._invalidate_test())

        radio_row = ttk.Frame(self)
        radio_row.pack(fill="x", pady=(0, 6))
        for key, prov in PROVIDERS.items():
            ttk.Radiobutton(radio_row, text=prov.label, value=key,
                            variable=self._provider,
                            command=self._rebuild_fields).pack(anchor="w")

        self._fields = ttk.Frame(self)
        self._fields.pack(fill="x")

        test_row = ttk.Frame(self)
        test_row.pack(fill="x", pady=(8, 0))
        self._test_btn = ttk.Button(test_row, text="測試連線", command=self._start_test)
        self._test_btn.pack(side="left")
        self._test_result = ttk.Label(test_row, text="")
        self._test_result.pack(side="left", fill="x", expand=True, padx=8)
        bind_wrap(self._test_result)

        self._target_language_fn = lambda: "繁體中文（台灣）"
        self._rebuild_fields()

    # --- 值存取 ---
    def get_values(self) -> dict:
        return {"provider": self._provider.get(), "api_key": self._api_key.get().strip(),
                "model": self._model.get().strip(),
                "base_url": self._base_url.get().strip(),
                "thinking": self._thinking.get()}

    def set_values(self, api: dict) -> None:
        self._provider.set(api["provider"])
        self._api_key.set(api["api_key"])
        self._model.set(api["model"])
        self._base_url.set(api["base_url"])
        self._thinking.set(api["thinking"])
        self._rebuild_fields()

    def set_target_language_fn(self, fn) -> None:
        """測試連線時取得目標語言的 callback（精靈階段語言還沒選，用預設）。"""
        self._target_language_fn = fn

    # --- 動態欄位 ---
    def _rebuild_fields(self) -> None:
        for w in self._fields.winfo_children():
            w.destroy()
        provider_changed = self._provider.get() != self._last_provider
        if provider_changed:
            # 切換服務商後，上一家的「連線成功」殘留字樣不該繼續顯示。
            self._test_result.configure(text="")
        prov = PROVIDERS[self._provider.get()]
        if prov.needs_base_url:
            self._labeled_entry("伺服器網址", self._base_url)
            self._labeled_entry("模型名稱", self._model)
            self._labeled_entry("API 金鑰（選填）", self._api_key, secret=True)
            self._thinking_row()
            # custom 分支不清空模型欄：使用者原輸入（含跨服務商切回時）都保留。
        else:
            self._labeled_entry("API 金鑰", self._api_key, secret=True)
            row = ttk.Frame(self._fields)
            row.pack(fill="x", pady=2)
            ttk.Label(row, text="模型").pack(side="left")
            combo = ttk.Combobox(row, textvariable=self._model, values=prov.models)
            combo.pack(side="left", fill="x", expand=True, padx=(8, 0))
            # 切換服務商後模型欄若殘留上一家的模型 ID（不在新清單內），改填新服務商預設模型；
            # 模型欄原本是空的（如精靈初始狀態）也一併補上預設值。
            if (provider_changed and self._model.get() not in prov.models) \
                    or not self._model.get():
                self._model.set(prov.models[0])
            if self._provider.get() == "claude":
                ttk.Label(self._fields, text=CLAUDE_MODEL_HINT,
                          foreground="#888888").pack(anchor="w")
            if self._provider.get() == "openai":
                # ChatGPT 官方端點也可關思考（只送 reasoning_effort，見 translator）；
                # Claude 維持模型預設（adaptive），不提供開關。
                self._thinking_row()
            link = ttk.Label(self._fields, text="取得金鑰 ↗", foreground="#4a7ddc",
                             cursor="hand2")
            link.pack(anchor="w", pady=(2, 0))
            link.bind("<Button-1>", lambda e: webbrowser.open(prov.key_url))
        self._last_provider = self._provider.get()
        self._invalidate_test()
        if self._on_change:
            self._on_change()

    def _thinking_row(self) -> None:
        """思考開關＋為何建議關閉的說明（支援思考開關的服務商共用）。"""
        ttk.Checkbutton(self._fields, text="啟用模型思考（thinking）",
                        variable=self._thinking).pack(anchor="w", pady=(2, 0))
        hint = ttk.Label(self._fields, text=THINKING_HINT, foreground="#888888",
                         justify="left")
        hint.pack(fill="x", padx=(20, 0))
        bind_wrap(hint)

    def _labeled_entry(self, label: str, var: tk.StringVar, secret: bool = False):
        row = ttk.Frame(self._fields)
        row.pack(fill="x", pady=2)
        ttk.Label(row, text=label, width=14).pack(side="left")
        entry = ttk.Entry(row, textvariable=var, show="●" if secret else "")
        entry.pack(side="left", fill="x", expand=True)
        if secret:
            btn = ttk.Button(row, text="顯示", width=5,
                             command=lambda: entry.configure(
                                 show="" if entry.cget("show") else "●"))
            btn.pack(side="left", padx=(4, 0))

    # --- 測試連線 ---
    def _invalidate_test(self) -> None:
        self.test_passed = False

    def _start_test(self) -> None:
        api = self.get_values()
        errors = validate_api_form(api)
        if errors:
            self._show_test_result(False, "；".join(errors))
            return
        self._test_btn.configure(state="disabled", text="測試中…")
        self._test_result.configure(text="")
        target = self._target_language_fn()
        threading.Thread(target=self._test_worker, args=(api, target),
                         daemon=True).start()
        self._poll_result()

    def _test_worker(self, api: dict, target_language: str) -> None:
        try:
            sample = test_translate(api, target_language)
        except Exception as exc:
            print(f"[settings] test connection failed (provider={api['provider']}, "
                  f"model={api['model']}): {exc}", file=sys.stderr)
            self._queue.put((False, friendly_error(exc)))
            return
        print(f"[settings] test connection ok (provider={api['provider']}, "
              f"model={api['model']})", file=sys.stderr)
        self._queue.put((True, f"連線成功　範例：{sample}"))

    def _poll_result(self) -> None:
        # tkinter 的 after 不保證跨執行緒安全：worker 只放 queue，主執行緒輪詢取用
        try:
            if not self.winfo_exists():
                return  # 測試進行中視窗被關閉：停止輪詢，結果丟棄
        except tk.TclError:
            return
        try:
            ok, message = self._queue.get_nowait()
        except queue.Empty:
            self.after(100, self._poll_result)
            return
        self._test_btn.configure(state="normal", text="測試連線")
        self._show_test_result(ok, message)

    def _show_test_result(self, ok: bool, message: str) -> None:
        self.test_passed = ok
        prefix = "✓ " if ok else "✗ "
        color = "#2e8b57" if ok else "#cc3333"
        self._test_result.configure(text=prefix + message, foreground=color)
        if self._on_change:
            self._on_change()


class HotkeyField(ttk.Frame):
    """熱鍵欄位：顯示目前值，點「更改」後按下組合鍵即設定（Esc 取消）。
    捕捉用 keyboard 套件的低階鍵盤鉤子而非 tk 事件：Ctrl+Space 等組合會先被
    輸入法（IME）或系統攔截、tk 收不到；低階鉤子在 IME 之前就能看到按鍵，
    且與實際註冊熱鍵走同一條路——捕捉得到就保證註冊得到。"""

    def __init__(self, parent, initial: str):
        super().__init__(parent)
        self._value = initial
        self._queue: queue.Queue = queue.Queue()
        self._capturing = False
        self._label = ttk.Label(self, text=initial)
        self._label.pack(side="left")
        self._btn = ttk.Button(self, text="更改", width=14, command=self._capture)
        self._btn.pack(side="left", padx=8)

    def value(self) -> str:
        return self._value

    def set_value(self, s: str) -> None:
        self._value = s
        self._label.configure(text=s)

    def _capture(self) -> None:
        if self._capturing:
            return
        self._capturing = True
        self._btn.configure(text="請按鍵…（Esc 取消）", state="disabled")
        threading.Thread(target=self._capture_worker, daemon=True).start()
        self._poll_capture()

    def _capture_worker(self) -> None:
        try:
            combo = keyboard.read_hotkey(suppress=False)
        except Exception as exc:
            print(f"[settings] hotkey capture failed: {exc}", file=sys.stderr)
            combo = None
        self._queue.put(combo)

    def _poll_capture(self) -> None:
        # 與測試連線同模式:worker 只放 queue,主執行緒輪詢取用(tk 跨執行緒不安全)
        try:
            if not self.winfo_exists():
                return  # 捕捉期間視窗被關閉:結果丟棄
        except tk.TclError:
            return
        try:
            combo = self._queue.get_nowait()
        except queue.Empty:
            self.after(100, self._poll_capture)
            return
        self._capturing = False
        self._btn.configure(text="更改", state="normal")
        if combo and combo != "esc":
            self.set_value(combo)


class LanguageField(ttk.Frame):
    """翻譯目標語言：常用語言下拉 + 可自行輸入。"""

    def __init__(self, parent, initial: str):
        super().__init__(parent)
        self._var = tk.StringVar(value=initial)
        combo = ttk.Combobox(self, textvariable=self._var, values=COMMON_LANGUAGES)
        combo.pack(fill="x")
        hint = ttk.Label(self, text=LANGUAGE_HINT, foreground="#888888",
                         justify="left")
        hint.pack(fill="x", pady=(2, 0))
        bind_wrap(hint)

    def value(self) -> str:
        return self._var.get().strip()

    def set_value(self, s: str) -> None:
        self._var.set(s)
