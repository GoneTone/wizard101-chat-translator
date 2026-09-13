"""模型欄位：可自行輸入的下拉選單，按「重新整理」向端點抓可用模型並即打即篩。
大半是 ttk combobox 下拉清單的 Tcl 層改寫（拆掉 modal 行為讓清單與輸入框並存），
自成一體，與其他欄位無關。
"""
import tkinter as tk
from tkinter import ttk

from src.i18n import t
from src.log import log
from src.translation.translator import TranslatorNoModelList, list_models
from src.ui.form import ERROR_COLOR, HINT_COLOR, LABEL_WIDTH, BackgroundButton, friendly_error
from src.ui.providers import validate_endpoint_fields
from src.ui.richtext import RichLabel, ttk_background


def filter_models(models: list[str], query: str) -> list[str]:
    """依關鍵字篩選模型清單（不分大小寫子字串比對）；關鍵字為空白＝不篩選。"""
    keyword = query.strip().lower()
    return [m for m in models if keyword in m.lower()] if keyword else list(models)


class ModelField(ttk.Frame):
    """模型欄位：可自行輸入的下拉選單 ＋「重新整理」向端點取得可用模型清單。
    清單不落地也不快取（每次按才抓）；端點沒有模型清單 API 時退化成純文字輸入。"""

    def __init__(self, parent, model_var: tk.StringVar, api_getter):
        super().__init__(parent)
        self._var = model_var
        self._api_getter = api_getter
        self._all_models: list[str] = []
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
        self._task = BackgroundButton(self._btn, "model list")
        # RichLabel：API 錯誤訊息裡的網址要能點；它自己依寬度換行，不必 bind_wrap
        self._status = RichLabel(self, fg=HINT_COLOR, bg=ttk_background(self),
                                 font="TkDefaultFont")
        self._status.set(t("hint.model_idle"))
        self._status.grid(row=1, column=1, columnspan=2, sticky="ew")

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
        return self._status.text()

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
        self._watching_clicks = False

    def _watch_outside_click(self) -> None:
        """展開期間監看整個視窗的點擊，點到別的控件就收起清單。原生靠 global grab
        才做到「點哪都關」，而 grab 已為了邊看清單邊打字拆掉（見 _make_popdown_modeless）。
        清單本身是另一個 toplevel，其點擊不會傳到這裡，不會誤收。
        綁定只掛一次、收合時不解除而是關旗標：`unbind(seq, funcid)` 在 Python 3.13 之前
        會把該序列上所有綁定一起清掉，日後誰在這個 toplevel 綁 <Button-1> 都會被拆。"""
        if self._outside_click_id is None:
            self._outside_click_id = self.winfo_toplevel().bind(
                "<Button-1>", self._on_click_elsewhere, add="+")
        self._watching_clicks = True

    def _on_click_elsewhere(self, event) -> None:
        if self._watching_clicks and event.widget is not self._combo:
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
        重抓 global grab，所以 <Map> 也得改寫 —— 只留「輸入框顯示按下狀態」。"""
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
        """把抓到的模型清單填進下拉選單，並在說明列報告數量（0 個時提示自行輸入）。"""
        self._all_models = list(models)
        self._combo.configure(values=self._all_models)
        self._set_status(t("hint.model_found", count=len(models)) if models
                         else t("hint.model_empty"))

    def show_error(self, exc: Exception) -> None:
        """抓模型清單失敗：清空選單、在說明列顯示易懂的原因（端點沒清單不算錯誤）。"""
        self._all_models = []
        self._combo.configure(values=[])
        if isinstance(exc, TranslatorNoModelList):
            self._set_status(t("hint.model_no_list"))
        else:
            key, kwargs = friendly_error(exc)
            self._set_status(t(key, **kwargs), error=True)

    def _set_status(self, text: str, error: bool = False) -> None:
        self._status.set(text, ERROR_COLOR if error else HINT_COLOR)

    # --- 取得清單 ---
    def _start_refresh(self) -> None:
        api = self._api_getter()
        errors = validate_endpoint_fields(api)
        if errors:
            self._set_status(t("sep.errors").join(t(e) for e in errors), error=True)
            return
        self._set_status(t("hint.model_idle"))
        self._task.start(lambda: list_models(api), lambda result: self._on_refreshed(result, api),
                         t("button.loading"))

    def _on_refreshed(self, result, api: dict) -> None:
        """抓清單的結果：模型 ID 清單，或拋出的例外。
        api.get：只有自訂端點的設定檔有 base_url，官方服務商用 api[...] 會 KeyError。"""
        if isinstance(result, Exception):
            reason = "unsupported" if isinstance(result, TranslatorNoModelList) else "failed"
            log(f"[settings] model list {reason} (provider={api['provider']}, "
                f"base_url={api.get('base_url', '')}): {result}")
            self.show_error(result)
            return
        log(f"[settings] model list fetched (provider={api['provider']}, "
            f"count={len(result)})")
        self.show_models(result)
