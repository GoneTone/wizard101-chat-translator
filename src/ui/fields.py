"""精靈與設定視窗共用的欄位元件與純邏輯：
服務商選擇、API 欄位、測試連線、熱鍵捕捉、語言選擇。"""
import queue
import sys
import threading
import tkinter as tk
import webbrowser

import keyboard
from dataclasses import dataclass
from tkinter import ttk

from src.i18n import LANGUAGES, SOURCE_LANGUAGE, current_language, t
from src.translator import (TranslatorConfigError, TranslatorNoModelList,
                            TranslatorOffline, list_models, test_translate)
from src.ui.fonts import ui_font
from src.ui.responsive import bind_wrap


@dataclass(frozen=True)
class Provider:
    label_key: str
    needs_base_url: bool
    key_url: str | None = None


PROVIDERS: dict[str, Provider] = {
    # 前兩家是品牌名，不進語言檔；只有「自訂端點」需要翻譯。
    "openai": Provider(label_key="provider.openai", needs_base_url=False,
                       key_url="https://platform.openai.com/api-keys"),
    "claude": Provider(label_key="provider.claude", needs_base_url=False,
                       key_url="https://console.anthropic.com/settings/keys"),
    "custom": Provider(label_key="provider.custom", needs_base_url=True),
}

# 翻譯目標語言的常用選項：各語言的 endonym，任何介面語言下都不翻譯。
COMMON_LANGUAGES = ["繁體中文（台灣）", "简体中文（中国）", "English", "日本語",
                    "한국어", "Español", "Português", "Deutsch", "Français"]

# 介面語言 → 翻譯目標語言的預設值：首次設定時讓兩者一致，之後互不干涉。
DEFAULT_TARGET_LANGUAGE = {
    "zh-TW": "繁體中文（台灣）",
    "zh-CN": "简体中文（中国）",
    "en": "English",
}


def validate_endpoint_fields(api: dict) -> list[str]:
    """檢查連上端點所需的欄位（不含模型），回傳錯誤文案 key 列表（空＝通過）。
    取模型清單時模型欄本來就還沒填，故與 validate_api_form 分開。"""
    errors = []
    provider = PROVIDERS[api["provider"]]
    if not provider.needs_base_url and not api["api_key"].strip():
        errors.append("error.need_api_key")
    if provider.needs_base_url and not api["base_url"].strip():
        errors.append("error.need_base_url")
    return errors


def validate_api_form(api: dict) -> list[str]:
    """檢查 API 表單必填欄位，回傳錯誤文案 key 列表（空＝通過）。"""
    errors = [] if api["model"].strip() else ["error.need_model"]
    return errors + validate_endpoint_fields(api)


def filter_models(models: list[str], query: str) -> list[str]:
    """依關鍵字篩選模型清單（不分大小寫子字串比對）；關鍵字為空白＝不篩選。"""
    keyword = query.strip().lower()
    return [m for m in models if keyword in m.lower()] if keyword else list(models)


def poll_queue(widget, result_queue: queue.Queue, on_result, interval_ms: int = 100):
    """輪詢背景執行緒放進 queue 的結果，取到就在主執行緒交給 on_result。
    tkinter 的 after 不保證跨執行緒安全：worker 只放 queue，由主執行緒輪詢取用。
    等待期間視窗被關閉即停止輪詢、結果丟棄。"""
    try:
        if not widget.winfo_exists():
            return
    except tk.TclError:
        return
    try:
        result = result_queue.get_nowait()
    except queue.Empty:
        widget.after(interval_ms,
                     lambda: poll_queue(widget, result_queue, on_result, interval_ms))
        return
    on_result(result)


def friendly_error(exc: Exception) -> tuple[str, dict]:
    """把翻譯例外轉成（文案 key， format 變數）。

    帶變數的兩種錯誤（HTTP 狀態碼、未預期例外）光靠 key 表達不了，故回傳
    tuple；顯示端一律 `t(key, **kwargs)`。"""
    if isinstance(exc, TranslatorConfigError):
        if exc.status in (401, 403):
            return "error.bad_key", {}
        if exc.status == 404:
            return "error.model_not_found", {}
        return "error.api_http", {"status": exc.status}
    if isinstance(exc, TranslatorOffline):
        return "error.offline", {}
    return "error.unexpected", {"error": exc}




class ModelField(ttk.Frame):
    """模型欄位：可自行輸入的下拉選單 ＋「重新整理」向端點取得可用模型清單。
    清單不落地也不快取（每次按才抓）；端點沒有模型清單 API 時退化成純文字輸入。"""

    def __init__(self, parent, model_var: tk.StringVar, api_getter):
        super().__init__(parent)
        self._var = model_var
        self._api_getter = api_getter
        self._all_models: list[str] = []
        self._queue: queue.Queue = queue.Queue()
        self._outside_click_id: str | None = None

        # grid 而非 pack：說明文字要與輸入框（而不是「模型」標籤）切齊同一欄。
        self.columnconfigure(1, weight=1)
        ttk.Label(self, text=t("field.model"), width=14).grid(row=0, column=0, sticky="w")
        self._combo = ttk.Combobox(self, textvariable=self._var, values=[])
        self._combo.grid(row=0, column=1, sticky="ew", pady=2)
        self._btn = ttk.Button(self, text=t("button.refresh"), width=9,
                               command=self._start_refresh)
        self._btn.grid(row=0, column=2, padx=(4, 0))
        self._status = ttk.Label(self, text=t("hint.model_idle"), foreground="#888888",
                                 justify="left")
        self._status.grid(row=1, column=1, columnspan=2, sticky="ew")
        bind_wrap(self._status)

        self._combo.bind("<KeyRelease>", self._on_type)
        self._combo.bind("<FocusOut>", lambda e: self.after_idle(self._unpost_if_left))
        self._combo.bind("<Escape>", lambda e: self.unpost_options())
        for key in ("<Down>", "<Up>"):
            self._combo.bind(key, self._on_arrow)
        self._combo.bind("<<ComboboxSelected>>", lambda e: self.unpost_options())
        # 從箭頭展開的清單也要解除 modal（原生 Press binding 攔不到，改在事件後補）。
        self._combo.bind("<ButtonRelease-1>",
                         lambda e: self.after_idle(self._make_popdown_modeless))

    # --- 狀態查詢（測試與呼叫端用） ---
    def options(self) -> list[str]:
        return list(self._combo.cget("values"))

    def status(self) -> str:
        return self._status.cget("text")

    # --- 下拉清單 ---
    def is_posted(self) -> bool:
        """下拉清單目前是否展開著。"""
        return bool(self._combo.tk.call("winfo", "ismapped", self._popdown()))

    def post_options(self) -> None:
        """展開下拉清單，並把鍵盤焦點留在輸入框，以便邊看清單邊改關鍵字。"""
        if not self.is_posted():
            self._combo.tk.call("ttk::combobox::Post", self._combo)
        self._make_popdown_modeless()
        self._watch_outside_click()
        self._combo.focus_set()

    def unpost_options(self) -> None:
        if not self._combo.winfo_exists():
            return  # 關窗時仍可能有延遲的收合回呼落到這裡
        self._combo.tk.call("ttk::combobox::Unpost", self._combo)
        if self._outside_click_id is not None:
            self.winfo_toplevel().unbind("<Button-1>", self._outside_click_id)
            self._outside_click_id = None

    def _watch_outside_click(self) -> None:
        """展開期間監看整個視窗的點擊，點到別的控件就收起清單。
        原生是靠 global grab 攔下所有點擊才做到「點哪都關」，而 grab 已為了讓
        使用者能邊看清單邊打字而拆掉（見 _make_popdown_modeless），只好自己補。
        清單本身是另一個 toplevel，它的點擊不會傳到這裡，不會誤收。"""
        if self._outside_click_id is None:
            self._outside_click_id = self.winfo_toplevel().bind(
                "<Button-1>", self._on_click_elsewhere, add="+")

    def _on_click_elsewhere(self, event) -> None:
        if event.widget is not self._combo:
            self.unpost_options()

    def _popdown(self) -> str:
        return self._combo.tk.eval(f"ttk::combobox::PopdownWindow {self._combo}")

    def _listbox(self) -> str:
        return self._popdown() + ".f.l"

    def _make_popdown_modeless(self) -> None:
        """拆掉展開清單的 modal 行為，讓它能與輸入框並存。

        ttk 原生的清單是 modal 的：grab 住滑鼠、一 map 就搶走鍵盤焦點、一失焦就
        自動收合。於是點回輸入框改關鍵字時，那一下點擊只會把清單關掉，使用者
        永遠沒辦法邊看清單邊篩選。三者都拆掉後，焦點留在輸入框、收合時機改由
        本元件自己掌握（失焦、Esc、選取）。

        這些行為都來自 ttk 的類別 binding（ComboboxPopdown／ComboboxListbox），
        只有實例 binding 才蓋得過去，且腳本要以 break 收尾才會中止類別 binding。
        清單 map 時原生會重新抓一次 global grab，所以連 <Map> 都得改寫——只留下
        「輸入框顯示按下狀態」，把 grab 拿掉。"""
        try:
            popdown = self._popdown()
            self._combo.tk.call("grab", "release", popdown)
            self._combo.tk.call("bind", popdown, "<Map>",
                                "[winfo parent %W] state pressed; break")
            for event in ("<Map>", "<FocusOut>"):
                self._combo.tk.call("bind", self._listbox(), event, "break")
        except tk.TclError:
            pass  # 清單尚未建立：沒有 grab 也沒有 binding 要拆

    def _unpost_if_left(self) -> None:
        """輸入框失焦後收起清單；焦點只是移進清單本身則不算離開。"""
        if not self._combo.winfo_exists():
            return
        if not str(self._combo.tk.call("focus")).startswith(self._popdown()):
            self.unpost_options()

    def _on_arrow(self, event) -> str:
        """方向鍵：展開清單並把鍵盤交給它，之後的上下移動與 Enter 選取都走原生。"""
        if not self._all_models:
            return "break"
        if not self.is_posted():
            self.post_options()
        self._combo.tk.call("focus", self._listbox())
        return "break"

    def _on_type(self, event) -> None:
        """輸入即篩選，並讓清單保持展開顯示篩選結果。
        方向鍵／Enter／Esc／Tab 交回原生鍵盤操作，不在這裡攔截。"""
        if event.keysym in ("Up", "Down", "Return", "Escape", "Tab"):
            return
        self.refresh_options()
        if self.options():
            self.post_options()

    # --- 清單呈現 ---
    def refresh_options(self) -> None:
        """依模型欄目前的輸入篩選下拉選單；還沒抓過清單就不動作。"""
        if not self._all_models:
            return
        matched = filter_models(self._all_models, self._var.get())
        self._combo.configure(values=matched)
        if not matched:
            self.unpost_options()  # 沒有相符項目時清單只會剩一個空白小框
        elif self.is_posted():
            self._reload_posted_list()

    def _reload_posted_list(self) -> None:
        """把新的 values 灌進「已經展開」的清單。
        ttk 只在展開當下填一次內容，之後改 values 畫面不會跟著變——邊打字邊篩選
        就得自己重填，並依新的項數重算清單高度與位置。"""
        self._combo.tk.call("ttk::combobox::ConfigureListbox", self._combo)
        self._combo.update_idletasks()  # 幾何要先傳播，重新定位才量得到新高度
        self._combo.tk.call("ttk::combobox::PlacePopdown", self._combo, self._popdown())

    def show_models(self, models: list[str]) -> None:
        self._all_models = list(models)
        self._combo.configure(values=self._all_models)
        self._set_status(t("hint.model_found", count=len(models)) if models
                         else t("hint.model_empty"))

    def show_error(self, exc: Exception) -> None:
        self._all_models = []
        self._combo.configure(values=[])
        if isinstance(exc, TranslatorNoModelList):
            self._set_status(t("hint.model_no_list"))
        else:
            key, kwargs = friendly_error(exc)
            self._set_status(t(key, **kwargs), error=True)

    def _set_status(self, text: str, error: bool = False) -> None:
        self._status.configure(text=text, foreground="#cc3333" if error else "#888888")

    # --- 取得清單 ---
    def _start_refresh(self) -> None:
        api = self._api_getter()
        errors = validate_endpoint_fields(api)
        if errors:
            self._set_status(t("sep.errors").join(t(e) for e in errors), error=True)
            return
        self._btn.configure(state="disabled", text=t("button.loading"))
        self._set_status(t("hint.model_idle"))
        threading.Thread(target=self._refresh_worker, args=(api,), daemon=True).start()
        poll_queue(self, self._queue, self._on_refreshed)

    def _refresh_worker(self, api: dict) -> None:
        try:
            models = list_models(api)
        except TranslatorNoModelList as exc:
            print(f"[settings] model list unsupported (provider={api['provider']}, "
                  f"base_url={api['base_url']}): {exc}", file=sys.stderr)
            self._queue.put(exc)
            return
        except Exception as exc:
            print(f"[settings] model list failed (provider={api['provider']}, "
                  f"base_url={api['base_url']}): {exc}", file=sys.stderr)
            self._queue.put(exc)
            return
        print(f"[settings] model list fetched (provider={api['provider']}, "
              f"count={len(models)})", file=sys.stderr)
        self._queue.put(models)

    def _on_refreshed(self, result) -> None:
        self._btn.configure(state="normal", text=t("button.refresh"))
        if isinstance(result, Exception):
            self.show_error(result)
        else:
            self.show_models(result)


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
        self._test_result = ttk.Label(test_row, text="")
        self._test_result.pack(side="left", fill="x", expand=True, padx=8)
        bind_wrap(self._test_result)

        self._target_language_fn = lambda: DEFAULT_TARGET_LANGUAGE[current_language()]
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
            # 模型 ID 跨服務商不通用（已抓的清單隨模型欄重建一起消失）。
            self._model.set("")
        prov = PROVIDERS[self._provider.get()]
        if prov.needs_base_url:
            self._labeled_entry(t("field.base_url"), self._base_url)
            hint = ttk.Label(self._fields, text=t("hint.custom_endpoint"),
                             foreground="#888888", justify="left")
            hint.pack(fill="x", padx=(20, 0))
            bind_wrap(hint)
            self._model_row()
            self._labeled_entry(t("field.api_key_optional"), self._api_key, secret=True)
            self._thinking_row()
        else:
            self._labeled_entry(t("field.api_key"), self._api_key, secret=True)
            self._model_row()
            if self._provider.get() == "openai":
                # ChatGPT 官方端點也可關思考（只送 reasoning_effort，見 translator）；
                # Claude 維持模型預設（adaptive），不提供開關。
                self._thinking_row()
            link = ttk.Label(self._fields, text=t("link.get_key"), foreground="#4a7ddc",
                             cursor="hand2")
            link.pack(anchor="w", pady=(2, 0))
            link.bind("<Button-1>", lambda e: webbrowser.open(prov.key_url))
        self._last_provider = self._provider.get()
        self._invalidate_test()
        if self._on_change:
            self._on_change()

    def _model_row(self) -> None:
        """模型欄（三家共用）：每次重建都是新元件，切換服務商時已抓的清單自然清空。"""
        self._model_field = ModelField(self._fields, self._model, self.get_values)
        self._model_field.pack(fill="x")

    def _thinking_row(self) -> None:
        """思考開關＋為何建議關閉的說明（支援思考開關的服務商共用）。"""
        ttk.Checkbutton(self._fields, text=t("field.thinking"),
                        variable=self._thinking).pack(anchor="w", pady=(2, 0))
        hint = ttk.Label(self._fields, text=t("hint.thinking"), foreground="#888888",
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
            btn = ttk.Button(row, text=t("button.show"), width=5,
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
            self._show_test_result(False, t("sep.errors").join(t(e) for e in errors))
            return
        self._test_btn.configure(state="disabled", text=t("button.testing"))
        self._test_result.configure(text="")
        target = self._target_language_fn()
        threading.Thread(target=self._test_worker, args=(api, target),
                         daemon=True).start()
        poll_queue(self, self._queue, self._on_tested)

    def _test_worker(self, api: dict, target_language: str) -> None:
        try:
            sample = test_translate(api, target_language)
        except Exception as exc:
            print(f"[settings] test connection failed (provider={api['provider']}, "
                  f"model={api['model']}): {exc}", file=sys.stderr)
            key, kwargs = friendly_error(exc)
            self._queue.put((False, t(key, **kwargs)))
            return
        print(f"[settings] test connection ok (provider={api['provider']}, "
              f"model={api['model']})", file=sys.stderr)
        self._queue.put((True, t("test.success", sample=sample)))

    def _on_tested(self, result) -> None:
        ok, message = result
        self._test_btn.configure(state="normal", text=t("button.test"))
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
        self._btn = ttk.Button(self, text=t("button.change"), width=14,
                               command=self._capture)
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
        self._btn.configure(text=t("button.press_key"), state="disabled")
        threading.Thread(target=self._capture_worker, daemon=True).start()
        poll_queue(self, self._queue, self._on_captured)

    def _capture_worker(self) -> None:
        try:
            combo = keyboard.read_hotkey(suppress=False)
        except Exception as exc:
            print(f"[settings] hotkey capture failed: {exc}", file=sys.stderr)
            combo = None
        self._queue.put(combo)

    def _on_captured(self, combo) -> None:
        self._capturing = False
        self._btn.configure(text=t("button.change"), state="normal")
        if combo and combo != "esc":
            self.set_value(combo)


class LanguageField(ttk.Frame):
    """翻譯目標語言：常用語言下拉 + 可自行輸入。"""

    def __init__(self, parent, initial: str):
        super().__init__(parent)
        self._var = tk.StringVar(value=initial)
        combo = ttk.Combobox(self, textvariable=self._var, values=COMMON_LANGUAGES)
        combo.pack(fill="x")
        hint = ttk.Label(self, text=t("hint.language"), foreground="#888888",
                         justify="left")
        hint.pack(fill="x", pady=(2, 0))
        bind_wrap(hint)

    def value(self) -> str:
        return self._var.get().strip()

    def set_value(self, s: str) -> None:
        self._var.set(s)


class UiLanguageField(ttk.Frame):
    """介面語言：固定三個選項的唯讀下拉。

    顯示 endonym（各語言自稱），對外進出的是語言碼——與 LanguageField
    （翻譯目標語言，可自由輸入任何語言名稱）是不同用途的兩個欄位。"""

    def __init__(self, parent, initial: str, on_change=None):
        super().__init__(parent)
        self._on_change = on_change
        self._names = list(LANGUAGES.values())
        self._codes = list(LANGUAGES)
        self._var = tk.StringVar(value=LANGUAGES.get(initial, LANGUAGES[SOURCE_LANGUAGE]))
        combo = ttk.Combobox(self, textvariable=self._var, values=self._names,
                             state="readonly")
        combo.pack(fill="x")
        combo.bind("<<ComboboxSelected>>", lambda e: self._notify())

    def value(self) -> str:
        """目前選到的語言碼。"""
        name = self._var.get()
        return self._codes[self._names.index(name)] if name in self._names \
            else SOURCE_LANGUAGE

    def set_value(self, code: str) -> None:
        self._var.set(LANGUAGES.get(code, LANGUAGES[SOURCE_LANGUAGE]))

    def _notify(self) -> None:
        if self._on_change is not None:
            self._on_change(self.value())
