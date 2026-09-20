"""ServicePane：服務清單分頁的行為。"""
import tkinter as tk

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
