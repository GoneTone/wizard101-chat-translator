"""「翻譯服務」分頁：預設服務、用途分派與服務清單，外加新增／編輯用的對話框。"""
import copy
import tkinter as tk
from tkinter import messagebox, ttk

from src.i18n import t
from src.log import log
from src.services import SLOTS, describe, new_service, unique_name, validate_service
from src.ui.fonts import ui_font
from src.ui.form import HINT_COLOR, collapsible, hint_label
from src.ui.geometry import centered_position
from src.ui.provider_picker import open_provider_picker
from src.ui.service_form import ServiceForm

_DIALOG_SIZE = (560, 480)   # 容得下最長的一組欄位（自訂端點）與測試結果訊息


class ServiceDialog:
    """新增／編輯單筆服務的 modal 對話框；`result` 是確定後的服務，取消時為 None。
    測試直接建構本類別並呼叫 `_ok()`／`_cancel()`，不經 `open_service_dialog` 的等待迴圈。"""

    def __init__(self, parent, service: dict, services: list[dict],
                 target_language_fn=None):
        self._services = services
        self.result: dict | None = None
        self.win = tk.Toplevel(parent)
        self.win.title(t("service.dialog_title"))
        win_w, win_h = _DIALOG_SIZE
        x, y = centered_position(self.win.winfo_screenwidth(),
                                 self.win.winfo_screenheight(), win_w, win_h)
        self.win.geometry(f"{win_w}x{win_h}+{x}+{y}")
        self.win.transient(parent.winfo_toplevel())
        # 抓住輸入：對話框開著時在背後按〔儲存〕會連這個視窗一起拆掉，wait_window 還在等
        self.win.grab_set()
        self.win.protocol("WM_DELETE_WINDOW", self._cancel)

        buttons = ttk.Frame(self.win, padding=(8, 0, 8, 8))
        buttons.pack(side="bottom", fill="x")
        ttk.Button(buttons, text=t("button.cancel"), command=self._cancel).pack(
            side="right")
        ttk.Button(buttons, text=t("button.ok"), command=self._ok).pack(
            side="right", padx=(0, 8))

        self.form = ServiceForm(self.win, service, services=services)
        if target_language_fn is not None:
            self.form.set_target_language_fn(target_language_fn)
        self.form.pack(fill="both", expand=True, padx=12, pady=12)

    def _ok(self) -> None:
        errors = validate_service(self.form.api_values())
        if errors:
            messagebox.showwarning(t("dialog.incomplete_title"),
                                   "\n".join(t(e) for e in errors), parent=self.win)
            return
        service = self.form.values()
        # 清單裡永遠不會出現兩個一樣的名稱：三個分派下拉只顯示名稱，同名會讓人選錯
        service["name"] = unique_name(service["name"], self._services,
                                      ignore_id=service["id"])
        self.result = service
        log(f"[settings] service saved (provider={service['provider']}, "
            f"has_key={bool(service.get('api_key'))})")
        self._close()

    def _cancel(self) -> None:
        self._close()

    def _close(self) -> None:
        # 先放掉輸入：grab 漏著不放會讓同一個 Tk session 之後的視窗收不到事件
        self.win.grab_release()
        self.win.destroy()


def open_service_dialog(parent, service: dict, services: list[dict],
                        target_language_fn=None) -> dict | None:
    """顯示對話框並等待關閉；確定回傳服務，取消回傳 None。"""
    dialog = ServiceDialog(parent, service, services, target_language_fn)
    parent.wait_window(dialog.win)
    return dialog.result


class ServicePane(ttk.Frame):
    """翻譯服務分頁：預設服務下拉、摺疊的用途分派、服務卡片清單。
    `values()` 回傳的三個鍵可直接寫回 cfg。"""

    def __init__(self, parent, section: dict, target_language_fn=None):
        super().__init__(parent)
        self._target_language_fn = target_language_fn
        self._services = copy.deepcopy(section["services"])
        self._default = section["default_service"]
        self._slots = dict(section["service_slots"])

        self._default_shown = tk.StringVar()
        self._slot_shown = {slot: tk.StringVar() for slot in SLOTS}
        self._default_row()
        self._slots_body = collapsible(
            self, t("service.slots_title"),
            expanded=any(self._slots[slot] is not None for slot in SLOTS))
        self._slot_rows()
        ttk.Label(self, text=t("service.my_services"), font=ui_font(10, "bold")).pack(
            anchor="w", pady=(12, 4))
        self._cards = ttk.Frame(self)
        self._cards.pack(fill="x")
        ttk.Button(self, text=t("button.add_service"), command=self.add_service).pack(
            pady=(8, 0))
        self._refresh()

    # --- 值存取 ---
    def values(self) -> dict:
        """目前編輯中的三個鍵（設定視窗按儲存時寫回 cfg）。"""
        return {"services": copy.deepcopy(self._services),
                "default_service": self._default,
                "service_slots": dict(self._slots)}

    def card_labels(self) -> list[str]:
        """卡片的標題列文字（預設那張前綴 ●，其餘留等寬空位）。"""
        return [f"{'●' if s['id'] == self._default else '　'} {s['name']}"
                for s in self._services]

    def slot_options(self) -> list[str | None]:
        """分派下拉的選項順序：None（跟隨預設）在最前面。"""
        return [None, *(s["id"] for s in self._services)]

    def slots_expanded(self) -> bool:
        return self._slots_body.is_expanded()

    def delete_button_enabled(self) -> bool:
        """最後一筆不給刪：刪光就無從翻譯，擋在按鈕比擋在儲存清楚。"""
        return len(self._services) > 1

    # --- 清單操作 ---
    def add_service(self) -> None:
        # 先問服務商再開表單：沒選就什麼都不開，那時還不知道該畫哪家的欄位
        picked = open_provider_picker(self)
        if picked is None:
            return
        draft = new_service(picked, self._services)
        result = open_service_dialog(self, draft, self._services,
                                     self._target_language_fn)
        if result is not None:
            self.apply_dialog_result(result)

    def edit_service(self, service_id: str) -> None:
        current = next(s for s in self._services if s["id"] == service_id)
        result = open_service_dialog(self, copy.deepcopy(current), self._services,
                                     self._target_language_fn)
        if result is not None:
            self.apply_dialog_result(result)

    def apply_dialog_result(self, service: dict) -> None:
        """把對話框確定的服務併回清單（新增或就地取代），必要時接手預設。"""
        service = dict(service)
        service["name"] = unique_name(service["name"], self._services,
                                      ignore_id=service["id"])
        for index, existing in enumerate(self._services):
            if existing["id"] == service["id"]:
                self._services[index] = service
                break
        else:
            self._services.append(service)
        if self._default is None:
            self._default = service["id"]
            log(f"[settings] first service added; default={service['id']}")
        self._refresh()

    def delete_service(self, service_id: str) -> None:
        """刪除一筆服務（先問確認）；連帶把指到它的用途退回跟隨預設，
        被刪的若是預設服務則交給清單第一筆接手。"""
        if not self.delete_button_enabled():
            return
        target = next(s for s in self._services if s["id"] == service_id)
        if not messagebox.askyesno(t("dialog.confirm_title"),
                                   t("service.confirm_delete", name=target["name"]),
                                   parent=self.winfo_toplevel()):
            return
        self._services = [s for s in self._services if s["id"] != service_id]
        for slot in SLOTS:
            if self._slots[slot] == service_id:
                self._slots[slot] = None
                log(f"[settings] slot {slot} followed the default after its service "
                    f"was deleted")
        if self._default == service_id:
            self._default = self._services[0]["id"] if self._services else None
            log(f"[settings] default service deleted; default={self._default}")
        self._refresh()

    # --- 版面 ---
    def _default_row(self) -> None:
        row = ttk.Frame(self)
        row.pack(fill="x", pady=(0, 8))
        ttk.Label(row, text=t("service.default")).pack(side="left")
        self._default_combo = ttk.Combobox(row, textvariable=self._default_shown,
                                           state="readonly")
        self._default_combo.pack(side="left", fill="x", expand=True, padx=(8, 0))
        self._default_combo.bind("<<ComboboxSelected>>",
                                 lambda e: self._pick_default())

    def _slot_rows(self) -> None:
        self._slot_combos = {}
        for slot in SLOTS:
            row = ttk.Frame(self._slots_body)
            row.pack(fill="x", pady=2)
            ttk.Label(row, text=t(f"slot.{slot}")).pack(side="left")
            combo = ttk.Combobox(row, textvariable=self._slot_shown[slot],
                                 state="readonly")
            combo.pack(side="left", fill="x", expand=True, padx=(8, 0))
            combo.bind("<<ComboboxSelected>>",
                       lambda e, s=slot: self._pick_slot(s))
            self._slot_combos[slot] = combo
        hint_label(self._slots_body, t("service.slots_hint")).pack(fill="x",
                                                                   pady=(2, 0))

    def _names(self) -> list[str]:
        return [s["name"] for s in self._services]

    def _pick_default(self) -> None:
        # 靠名稱換回 id，前提是名稱不重複；這個保證來自寫入端的 unique_name()
        self._default = self._services[self._names().index(
            self._default_shown.get())]["id"]
        log(f"[settings] default service set to {self._default}")
        self._refresh()

    def _pick_slot(self, slot: str) -> None:
        # 靠名稱換回 id，前提是名稱不重複；這個保證來自寫入端的 unique_name()
        index = [t("service.follow_default"), *self._names()].index(
            self._slot_shown[slot].get())
        self._slots[slot] = self.slot_options()[index]
        log(f"[settings] slot {slot} set to {self._slots[slot] or 'default'}")
        # 不重畫：要更新的只有下拉自己的顯示，而它的 StringVar 已經拿著新選項了

    def _refresh(self) -> None:
        """下拉選項與卡片都由清單現況重畫（新增、刪除、改名共用同一條路）。"""
        names = self._names()
        self._default_combo.configure(values=names)
        current = next((s for s in self._services if s["id"] == self._default), None)
        self._default_shown.set(current["name"] if current else "")
        for slot in SLOTS:
            self._slot_combos[slot].configure(
                values=[t("service.follow_default"), *names])
            assigned = next((s for s in self._services
                             if s["id"] == self._slots[slot]), None)
            self._slot_shown[slot].set(
                assigned["name"] if assigned else t("service.follow_default"))
        for widget in self._cards.winfo_children():
            widget.destroy()
        for label, service in zip(self.card_labels(), self._services, strict=True):
            self._card(label, service)

    def _card(self, label: str, service: dict) -> None:
        card = ttk.Frame(self._cards, relief="solid", borderwidth=1, padding=8)
        card.pack(fill="x", pady=2)
        top = ttk.Frame(card)
        top.pack(fill="x")
        ttk.Label(top, text=label, font=ui_font(10, "bold")).pack(side="left")
        delete = ttk.Button(top, text=t("button.delete"), width=7,
                            command=lambda: self.delete_service(service["id"]))
        delete.pack(side="right")
        if not self.delete_button_enabled():
            delete.configure(state="disabled")
        ttk.Button(top, text=t("button.edit"), width=7,
                   command=lambda: self.edit_service(service["id"])).pack(
            side="right", padx=(0, 4))
        ttk.Label(card, text=describe(service), foreground=HINT_COLOR).pack(anchor="w")
