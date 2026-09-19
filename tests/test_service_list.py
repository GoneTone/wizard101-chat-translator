"""ServicePane：服務清單分頁的行為。"""
import pytest

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


def test_cards_are_listed_in_order_with_the_default_marked(two):
    pane, a, b = two
    # 全形空白＋半形空格才與「● 」等寬對齊，非預設那列不能只有全形空白
    assert pane.card_labels() == [f"● {a['name']}", f"　 {b['name']}"]


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
    assert pane.delete_button_enabled(a["id"]) is False
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
