"""選服務商的卡片清單，以及包著它的 modal 對話框。

獨立成一個模組是為了避開循環匯入：`service_form` 的〔更改〕與 `service_list` 的
新增都要用它，而 `service_list` 本來就匯入 `service_form`。
"""
import tkinter as tk
from tkinter import ttk

from src.i18n import t
from src.log import log
from src.services import PROVIDERS
from src.ui.fonts import ui_font
from src.ui.form import hint_label, keyboard_activatable
from src.ui.geometry import centered_position

_PICKER_SIZE = (440, 360)   # 容得下三張卡片（最長的說明會折成兩行）與按鈕列


class ProviderCards(ttk.Frame):
    """每家服務商一張卡片；點下去或用鍵盤選取都呼叫 `on_pick(provider_key)`。
    精靈直接內嵌這個元件，設定視窗則透過 `ProviderPicker` 用它。"""

    def __init__(self, parent, on_pick):
        super().__init__(parent)
        self._on_pick = on_pick
        for key in PROVIDERS:
            self._card(key)

    def _card(self, key: str) -> None:
        card = ttk.Frame(self, relief="solid", borderwidth=1, padding=8, cursor="hand2")
        card.pack(fill="x", pady=2)
        title = ttk.Label(card, text=t(PROVIDERS[key].label_key), font=ui_font(10, "bold"),
                          cursor="hand2")
        title.pack(anchor="w")
        description = hint_label(card, t(PROVIDERS[key].desc_key))
        description.configure(cursor="hand2")
        description.pack(fill="x")
        # 卡片被兩個標籤蓋滿，點在文字上收到事件的是標籤；Tk 不會把它往父層冒泡
        for widget in (card, title, description):
            widget.bind("<Button-1>", lambda event, k=key: self._on_pick(k))
        keyboard_activatable(card, lambda k=key: self._on_pick(k))


class ProviderPicker:
    """選服務商的 modal 對話框；`result` 是選到的 provider key，取消時為 None。
    測試直接建構本類別並呼叫 `_pick()`／`_cancel()`，不經 `open_provider_picker` 的等待迴圈。"""

    def __init__(self, parent):
        self.result: str | None = None
        self.win = tk.Toplevel(parent)
        self.win.title(t("service.pick_provider"))
        win_w, win_h = _PICKER_SIZE
        x, y = centered_position(self.win.winfo_screenwidth(),
                                 self.win.winfo_screenheight(), win_w, win_h)
        self.win.geometry(f"{win_w}x{win_h}+{x}+{y}")
        self.win.transient(parent.winfo_toplevel())
        # Tk 沒有 grab 堆疊：這個選擇框常是從已經有 grab 的服務對話框裡開出來的，
        # 關閉時得把 grab 還回去，否則那個對話框會默默失去 modal（見 _close）
        self._previous_grab = parent.grab_current()
        # 抓住輸入：選擇框開著時在背後操作會連這個視窗一起拆掉，wait_window 還在等
        self.win.grab_set()
        self.win.protocol("WM_DELETE_WINDOW", self._cancel)
        self.win.bind("<Escape>", lambda event: self._cancel())

        # 按鈕列先 pack：後宣告會被 expand=True 的卡片區擠掉
        buttons = ttk.Frame(self.win, padding=(8, 0, 8, 8))
        buttons.pack(side="bottom", fill="x")
        ttk.Button(buttons, text=t("button.cancel"), command=self._cancel).pack(side="right")

        self.cards = ProviderCards(self.win, self._pick)
        self.cards.pack(fill="both", expand=True, padx=12, pady=12)

    def _pick(self, provider: str) -> None:
        self.result = provider
        log(f"[settings] provider picked: {provider}")
        self._close()

    def _cancel(self) -> None:
        self._close()

    def _close(self) -> None:
        # 先放掉輸入：grab 漏著不放會讓同一個 Tk session 之後的視窗收不到事件
        self.win.grab_release()
        self.win.destroy()
        if self._previous_grab is not None and self._previous_grab.winfo_exists():
            self._previous_grab.grab_set()


def open_provider_picker(parent) -> str | None:
    """顯示服務商選擇對話框並等待關閉；選了回傳 provider key，取消回傳 None。"""
    picker = ProviderPicker(parent)
    parent.wait_window(picker.win)
    return picker.result
