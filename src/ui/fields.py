"""精靈與設定視窗共用的欄位元件與純邏輯：
服務商選擇、API 欄位、測試連線、熱鍵捕捉、語言選擇。"""
import copy
import queue
import sys
import threading
import tkinter as tk
import webbrowser

import keyboard
from dataclasses import dataclass
from tkinter import ttk

from src.config import (API_EFFORTS, API_PROFILE_FIELDS, API_PROVIDERS,
                        EFFORT_AUTO, needs_base_url)
from src.i18n import (DEFAULT_LANGUAGE, available_languages, current_language,
                      language_name, t)
from src.translation.translator import (TranslatorConfigError, TranslatorNoModelList,
                            TranslatorOffline, list_models, test_translate)
from src.ui.responsive import bind_wrap


@dataclass(frozen=True)
class Provider:
    """服務商的 UI 資料。該畫哪些欄位一律問 config 的欄位表（has_field），
    這裡只補純 UI 的部分：顯示名稱與申請金鑰的連結。"""
    key: str
    label_key: str
    key_url: str | None = None

    def has_field(self, name: str) -> bool:
        return name in API_PROFILE_FIELDS[self.key]

    @property
    def needs_base_url(self) -> bool:
        return needs_base_url(self.key)


PROVIDERS: dict[str, Provider] = {p.key: p for p in (
    # 前兩家是品牌名，不進語言檔；只有「自訂端點」需要翻譯。
    Provider(key="openai", label_key="provider.openai",
             key_url="https://platform.openai.com/api-keys"),
    Provider(key="claude", label_key="provider.claude",
             key_url="https://console.anthropic.com/settings/keys"),
    Provider(key="custom", label_key="provider.custom"),
)}

# 欄位標籤欄的字元寬：標籤、模型欄與欄位說明共用同一個值才對得齊
LABEL_WIDTH = 14

# 精靈、設定視窗與輸入框共用的字色：可點連結、欄位說明的灰、成功綠、失敗紅
LINK_COLOR = "#4a7ddc"
HINT_COLOR = "#888888"
OK_COLOR = "#2e8b57"
ERROR_COLOR = "#cc3333"

# 翻譯目標語言的常用選項：各語言的 endonym，任何介面語言下都不翻譯。
COMMON_LANGUAGES = ["繁體中文（台灣）", "简体中文（中国）", "English", "日本語",
                    "한국어", "Español", "Português", "Deutsch", "Français"]

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


def show_outcome(label: ttk.Label, ok: bool, message: str) -> None:
    """把一次操作的結果寫進標籤：成功「✓ 」綠字、失敗「✗ 」紅字
    （測試連線與檢查更新共用同一種呈現）。"""
    label.configure(text=("✓ " if ok else "✗ ") + message,
                    foreground=OK_COLOR if ok else ERROR_COLOR)


def link_label(parent, text: str, url: str) -> ttk.Label:
    """藍字可點的連結標籤：點擊以系統瀏覽器開啟 url。"""
    label = ttk.Label(parent, text=text, foreground=LINK_COLOR, cursor="hand2")
    label.bind("<Button-1>", lambda e: webbrowser.open(url))
    return label


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
    """把翻譯例外轉成（文案 key，format 變數）；顯示端一律 `t(key, **kwargs)`。
    HTTP 狀態碼與未預期例外光靠 key 表達不了，故帶變數。"""
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
        ttk.Label(self, text=t("field.model"), width=LABEL_WIDTH).grid(
            row=0, column=0, sticky="w")
        self._combo = ttk.Combobox(self, textvariable=self._var, values=[])
        self._combo.grid(row=0, column=1, sticky="ew", pady=2)
        self._btn = ttk.Button(self, text=t("button.refresh"), width=9,
                               command=self._start_refresh)
        self._btn.grid(row=0, column=2, padx=(4, 0))
        self._status = ttk.Label(self, text=t("hint.model_idle"), foreground=HINT_COLOR,
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
        """展開期間監看整個視窗的點擊，點到別的控件就收起清單。原生靠 global grab
        才做到「點哪都關」，而 grab 已為了邊看清單邊打字拆掉（見 _make_popdown_modeless）。
        清單本身是另一個 toplevel，其點擊不會傳到這裡，不會誤收。"""
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
        """拆掉展開清單的 modal 行為，讓它與輸入框並存。

        ttk 原生清單會 grab 滑鼠、一 map 就搶鍵盤焦點、一失焦就收合，點回輸入框改
        關鍵字那一下只會把清單關掉。三者都拆掉後焦點留在輸入框，收合時機由本元件
        掌握（失焦、Esc、選取）。這些行為來自 ttk 類別 binding（ComboboxPopdown／
        ComboboxListbox），只有實例 binding 以 break 收尾才蓋得過；清單 map 時原生會
        重抓 global grab，所以 <Map> 也得改寫——只留「輸入框顯示按下狀態」。"""
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
        """把新的 values 灌進已展開的清單：ttk 只在展開當下填一次內容，之後改 values
        畫面不會跟著變，得自己重填並依新項數重算清單高度與位置。"""
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
        self._status.configure(text=text, foreground=ERROR_COLOR if error else HINT_COLOR)

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
        self._test_result = ttk.Label(test_row, text="")
        self._test_result.pack(side="left", fill="x", expand=True, padx=8)
        bind_wrap(self._test_result)

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

    def set_values(self, api: dict) -> None:
        """整份 api 區塊換掉（設定視窗重建時復原 draft 用）。"""
        self._profiles = {name: dict(api[name]) for name in API_PROVIDERS}
        # 先對齊 _last_provider，_rebuild_fields 才不會把剛載入的值當成上一家的而歸還回去
        self._last_provider = api["provider"]
        self._provider.set(api["provider"])
        self._load_profile(api["provider"])
        self._rebuild_fields()

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
        self._test_result.configure(text="")
        profile = self._profiles[target]
        print(f"[settings] provider switched {self._last_provider} -> {target} "
              f"(model={profile['model'] or '-'}, has_key={bool(profile['api_key'])})",
              file=sys.stderr)

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
        hint = ttk.Label(row, text=text, foreground=HINT_COLOR, justify="left")
        hint.pack(side="left", fill="x", expand=True)
        bind_wrap(hint)

    def _model_row(self) -> None:
        """模型欄（三家共用）：每次重建都是新元件，切換服務商時已抓的清單自然清空。"""
        self._model_field = ModelField(self._fields, self._model, self.active_values)
        self._model_field.pack(fill="x")

    def _effort_row(self) -> None:
        """思考深度（Claude）：選項是「自動／精簡」而不是開關——Claude 沒有完全不
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
        hint = ttk.Label(self._fields, text=t("hint.thinking"), foreground=HINT_COLOR,
                         justify="left")
        hint.pack(fill="x", padx=(20, 0))
        bind_wrap(hint)

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

    def clear_test_result(self) -> None:
        """作廢已顯示的測試結果：那句譯文是用當時的目標語言翻的，語言一改就不算數。"""
        self._invalidate_test()
        self._test_result.configure(text="")
        if self._on_change:
            self._on_change()

    def _start_test(self) -> None:
        api = self.active_values()
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
        show_outcome(self._test_result, ok, message)
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

    def __init__(self, parent, initial: str, on_change=None):
        super().__init__(parent)
        self._var = tk.StringVar(value=initial)
        if on_change is not None:
            # 下拉選取與手動輸入都要通知：測試連線的譯文是用某個目標語言翻出來的，
            # 語言一改那句譯文就過期了（見 ApiFields.clear_test_result）。
            self._var.trace_add("write", lambda *_: on_change())
        combo = ttk.Combobox(self, textvariable=self._var, values=COMMON_LANGUAGES)
        combo.pack(fill="x")
        hint = ttk.Label(self, text=t("hint.language"), foreground=HINT_COLOR,
                         justify="left")
        hint.pack(fill="x", pady=(2, 0))
        bind_wrap(hint)

    def value(self) -> str:
        return self._var.get().strip()

    def set_value(self, s: str) -> None:
        self._var.set(s)


class UiLanguageField(ttk.Frame):
    """介面語言：唯讀下拉，顯示 endonym、對外進出語言碼——與 LanguageField
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
