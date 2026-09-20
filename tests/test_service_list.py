"""ServicePane：服務清單分頁的行為。"""
import tkinter as tk
from tkinter import ttk

import pytest

from src.i18n import t
from src.services import SLOT_INCOMING, SLOT_OUTGOING, SLOT_REGION, SLOTS, new_service
from src.ui.service_list import ServiceDialog, ServicePane


@pytest.fixture(autouse=True)
def _always_confirm(monkeypatch):
    """刪除確認框一律答「是」、驗證失敗的警示框直接關掉，免得測試停在等人按鈕。"""
    monkeypatch.setattr("src.ui.service_list.messagebox.askyesno", lambda *a, **k: True)
    monkeypatch.setattr("src.ui.service_list.messagebox.showwarning", lambda *a, **k: None)


def _service(provider, services, **fields):
    service = new_service(provider, services)
    service.update({"model": "m", "api_key": "k", **fields})
    return service


@pytest.fixture
def two(root):
    a = _service("openai", [])
    b = _service("claude", [a])
    section = {"services": [a, b], "default_service": a["id"],
               "service_slots": {slot: None for slot in SLOTS}}
    return ServicePane(root, section), a, b


def test_values_round_trip_unchanged(two):
    pane, a, b = two
    assert pane.values() == {"services": [a, b], "default_service": a["id"],
                             "service_slots": {slot: None for slot in SLOTS}}


def test_cards_are_listed_in_order_with_the_default_badged(two):
    pane, a, b = two
    assert pane.card_labels() == [f"{a['name']}　{t('service.badge_default')}",
                                  b["name"]]


def test_a_card_with_no_role_shows_its_name_alone(two):
    """沒有角色的服務不留前導空位：標記在名稱後面，沒什麼要對齊的。"""
    pane, _a, b = two
    assert pane.card_labels()[1] == b["name"]


def test_the_default_badge_comes_before_the_slot_badges(root):
    a = _service("openai", [])
    b = _service("claude", [a])
    pane = ServicePane(root, {"services": [a, b], "default_service": a["id"],
                              "service_slots": {SLOT_INCOMING: None,
                                                SLOT_OUTGOING: None,
                                                SLOT_REGION: a["id"]}})
    assert pane.card_labels()[0] == (
        f"{a['name']}　{t('service.badge_default')} · {t('slot.region')}")


def test_several_slots_on_one_service_are_badged_in_slot_order(root):
    a = _service("openai", [])
    b = _service("claude", [a])
    pane = ServicePane(root, {"services": [a, b], "default_service": a["id"],
                              "service_slots": {SLOT_INCOMING: b["id"],
                                                SLOT_OUTGOING: None,
                                                SLOT_REGION: b["id"]}})
    assert pane.card_labels()[1] == (
        f"{b['name']}　{t('slot.incoming')} · {t('slot.region')}")


def test_deleting_the_default_hands_it_to_the_first_remaining(two):
    pane, a, b = two
    pane.delete_service(a["id"])
    assert pane.values()["default_service"] == b["id"]
    assert [s["id"] for s in pane.values()["services"]] == [b["id"]]


def test_cancelling_delete_changes_nothing(two, monkeypatch):
    pane, a, _b = two
    before = pane.values()
    monkeypatch.setattr("src.ui.service_list.messagebox.askyesno", lambda *a, **k: False)
    pane.delete_service(a["id"])
    assert pane.values() == before


def test_deleting_an_assigned_service_frees_that_slot(root):
    a = _service("openai", [])
    b = _service("claude", [a])
    pane = ServicePane(root, {"services": [a, b], "default_service": a["id"],
                              "service_slots": {SLOT_INCOMING: None,
                                                SLOT_OUTGOING: None,
                                                SLOT_REGION: b["id"]}})
    pane.delete_service(b["id"])
    assert pane.values()["service_slots"][SLOT_REGION] is None


def test_the_last_service_cannot_be_deleted(root):
    a = _service("openai", [])
    pane = ServicePane(root, {"services": [a], "default_service": a["id"],
                              "service_slots": {slot: None for slot in SLOTS}})
    assert pane.delete_button_enabled() is False
    pane.delete_service(a["id"])
    assert len(pane.values()["services"]) == 1


def test_adding_to_an_empty_list_makes_it_the_default(root):
    pane = ServicePane(root, {"services": [], "default_service": None,
                              "service_slots": {slot: None for slot in SLOTS}})
    added = _service("openai", [])
    pane.apply_dialog_result(added)
    assert pane.values()["default_service"] == added["id"]


def test_a_saved_name_never_collides(two):
    pane, a, b = two
    renamed = dict(b, name=a["name"])
    pane.apply_dialog_result(renamed)
    assert pane.values()["services"][1]["name"] == f"{a['name']} (2)"


def test_slots_collapsed_when_everything_follows_the_default(two):
    pane, _a, _b = two
    assert pane.slots_expanded() is False


def test_slots_expanded_when_one_is_assigned(root):
    a = _service("openai", [])
    b = _service("claude", [a])
    pane = ServicePane(root, {"services": [a, b], "default_service": a["id"],
                              "service_slots": {SLOT_INCOMING: None,
                                                SLOT_OUTGOING: None,
                                                SLOT_REGION: b["id"]}})
    assert pane.slots_expanded() is True


def test_slot_options_start_with_follow_the_default(two):
    pane, a, b = two
    assert pane.slot_options() == [None, a["id"], b["id"]]


def test_dialog_ok_keeps_result_none_when_invalid(root):
    draft = new_service("openai", [])   # 沒填 model／api_key，驗證必定失敗
    dialog = ServiceDialog(root, draft, [])
    dialog._ok()
    assert dialog.result is None
    dialog.win.destroy()   # 驗證失敗時 _ok() 不會自己關窗，測試自己收尾


def test_dialog_ok_saves_full_service_when_valid(root):
    service = _service("openai", [])
    dialog = ServiceDialog(root, service, [])
    dialog._ok()
    assert dialog.result == service


def test_dialog_cancel_leaves_result_none(root):
    draft = new_service("openai", [])
    dialog = ServiceDialog(root, draft, [])
    dialog._cancel()
    assert dialog.result is None


def test_dialog_geometry_carries_a_position(root, monkeypatch):
    """對話框要自己算好位置：conftest 的停放 fixture 只改寫帶 `+x+y` 的 geometry 字串，
    少了座標就由視窗管理員自行擺放，測試會把視窗開在執行中的遊戲上面。"""
    asked = []
    original = tk.Wm.geometry

    def spy(self, newGeometry=None):
        if newGeometry:
            asked.append(newGeometry)
        return original(self, newGeometry)

    monkeypatch.setattr(tk.Wm, "geometry", spy)
    dialog = ServiceDialog(root, new_service("openai", []), [])
    dialog._cancel()
    assert asked and all("+" in geometry for geometry in asked)


def test_dialog_grabs_input_and_releases_it_on_close(root):
    """對話框開著時要擋住背後的設定視窗：在那裡按〔儲存〕會連對話框一起拆掉，
    而 wait_window 還在等它。關閉時要放掉 grab，免得漏進同一個 Tk session 的後續視窗。"""
    dialog = ServiceDialog(root, new_service("openai", []), [])
    assert dialog.win.grab_current() is dialog.win
    dialog._cancel()
    assert root.grab_current() is None


def test_add_service_opens_nothing_when_the_pick_is_cancelled(two, monkeypatch):
    # 沒選服務商就不該開一張空表單：那時還不知道要畫哪家的欄位
    pane, _a, _b = two
    before = pane.values()
    opened = []
    monkeypatch.setattr("src.ui.service_list.open_provider_picker", lambda parent: None)
    monkeypatch.setattr("src.ui.service_list.open_service_dialog",
                        lambda *args, **kwargs: opened.append(args) or None)
    pane.add_service()
    assert opened == []
    assert pane.values() == before


def test_add_service_drafts_the_picked_provider(two, monkeypatch):
    pane, _a, _b = two
    drafts = []
    monkeypatch.setattr("src.ui.service_list.open_provider_picker", lambda parent: "custom")
    monkeypatch.setattr("src.ui.service_list.open_service_dialog",
                        lambda parent, service, *args, **kwargs: drafts.append(service) or None)
    pane.add_service()
    assert [d["provider"] for d in drafts] == ["custom"]


def _card_frames(pane):
    """卡片區裡的卡片本體（同一區還可能有說明用的提示標籤）。"""
    return [w for w in pane._cards.pack_slaves() if isinstance(w, ttk.Frame)]


def _card_title(card):
    return next(w for w in card.pack_slaves()[0].pack_slaves()
                if isinstance(w, ttk.Label))


def _card_buttons(card):
    return [w for w in card.pack_slaves()[0].pack_slaves() if isinstance(w, ttk.Button)]


def _drawn_labels(pane):
    return [str(_card_title(card).cget("text")) for card in _card_frames(pane)]


def _cards_area_hints(pane):
    return [str(w.cget("text")) for w in pane._cards.pack_slaves()
            if isinstance(w, ttk.Label)]


def test_assigning_a_slot_rebadges_the_cards(root, two):
    """回歸：卡片標記現在也畫用途分派，改下拉就得重畫卡片。"""
    pane, _a, b = two
    pane._slot_shown[SLOT_INCOMING].set(b["name"])
    pane._slot_combos[SLOT_INCOMING].event_generate("<<ComboboxSelected>>")
    root.update()
    assert pane.values()["service_slots"][SLOT_INCOMING] == b["id"]
    assert _drawn_labels(pane) == pane.card_labels()
    assert _drawn_labels(pane)[1] == f"{b['name']}　{t('slot.incoming')}"


def test_a_long_name_does_not_squeeze_the_card_buttons(root):
    """名稱長到撐滿卡片時，被壓縮的要是標題文字而不是〔編輯〕〔刪除〕。"""
    win = tk.Toplevel(root)
    win.geometry("640x480+120+120")
    try:
        a = _service("openai", [])
        a["name"] = "My self-hosted DeepSeek endpoint"
        b = _service("claude", [a])
        pane = ServicePane(win, {"services": [a, b], "default_service": a["id"],
                                 "service_slots": {slot: a["id"] for slot in SLOTS}})
        pane.pack(fill="both", expand=True)
        win.update()
        widths = [(btn.winfo_width(), btn.winfo_reqwidth())
                  for btn in _card_buttons(_card_frames(pane)[0])]
        assert all(drawn >= wanted for drawn, wanted in widths), widths
    finally:
        win.destroy()


def test_dialog_escape_is_wired_to_cancel(root):
    """鍵盤事件只送得到焦點視窗，而測試不搶焦點（開發者常開著遊戲跑測試），
    所以只確認 <Escape> 綁上去了，關閉行為由〔取消〕那則測試涵蓋。"""
    dialog = ServiceDialog(root, new_service("openai", []), [])
    assert dialog.win.bind("<Escape>")
    dialog._cancel()
    assert root.grab_current() is None


def test_the_last_service_says_why_delete_is_greyed_out(root):
    """灰掉的按鈕自己不會說話：只剩一組時在卡片區寫明刪不掉的原因。"""
    a = _service("openai", [])
    pane = ServicePane(root, {"services": [a], "default_service": a["id"],
                              "service_slots": {slot: None for slot in SLOTS}})
    assert _cards_area_hints(pane) == [t("service.keep_one")]


def test_the_keep_one_hint_goes_away_once_there_are_two(two):
    pane, _a, _b = two
    assert _cards_area_hints(pane) == []


def test_deleting_an_assigned_service_says_its_slots_fall_back(root, monkeypatch):
    """指定它的用途會被悄悄退回預設，確認框要先講。"""
    asked = []
    monkeypatch.setattr("src.ui.service_list.messagebox.askyesno",
                        lambda title, message, **kwargs: asked.append(message) or True)
    a = _service("openai", [])
    b = _service("claude", [a])
    pane = ServicePane(root, {"services": [a, b], "default_service": a["id"],
                              "service_slots": {SLOT_INCOMING: b["id"],
                                                SLOT_OUTGOING: None,
                                                SLOT_REGION: None}})
    pane.delete_service(b["id"])
    assert asked == [t("service.confirm_delete_in_use", name=b["name"])]


def test_deleting_an_unassigned_service_keeps_the_plain_question(two, monkeypatch):
    asked = []
    monkeypatch.setattr("src.ui.service_list.messagebox.askyesno",
                        lambda title, message, **kwargs: asked.append(message) or True)
    pane, _a, b = two
    pane.delete_service(b["id"])
    assert asked == [t("service.confirm_delete", name=b["name"])]
