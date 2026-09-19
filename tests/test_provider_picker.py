"""ProviderCards／ProviderPicker：選服務商的卡片清單與它的 modal 包裝。"""
import tkinter as tk
from tkinter import ttk

from src.i18n import t
from src.services import PROVIDERS
from src.ui.provider_picker import ProviderCards, ProviderPicker


def _cancel_button(parent):
    """視窗裡的〔取消〕按鈕（卡片沒有按鈕，整棵樹裡只有這一顆）。"""
    for widget in parent.winfo_children():
        if isinstance(widget, ttk.Button) and str(widget.cget("text")) == t("button.cancel"):
            return widget
        found = _cancel_button(widget)
        if found is not None:
            return found
    return None


def test_one_card_per_provider(root):
    cards = ProviderCards(root, lambda key: None)
    assert len(cards.pack_slaves()) == len(PROVIDERS)


def test_each_card_titles_its_provider(root):
    cards = ProviderCards(root, lambda key: None)
    titles = [card.pack_slaves()[0].cget("text") for card in cards.pack_slaves()]
    assert titles == [t(PROVIDERS[key].label_key) for key in PROVIDERS]


def test_each_card_describes_its_provider(root):
    cards = ProviderCards(root, lambda key: None)
    descriptions = [card.pack_slaves()[1].cget("text") for card in cards.pack_slaves()]
    assert descriptions == [t(f"provider.{key}_desc") for key in PROVIDERS]


def test_clicking_a_card_reports_its_provider(root):
    picker = ProviderPicker(root)
    picker.cards.pack_slaves()[1].event_generate("<Button-1>")
    assert picker.result == list(PROVIDERS)[1]


def test_clicking_the_text_inside_a_card_counts_too(root):
    """整張卡都可點：卡片被子標籤蓋滿，點在文字上收到事件的是標籤而不是卡片。"""
    picker = ProviderPicker(root)
    picker.cards.pack_slaves()[0].pack_slaves()[1].event_generate("<Button-1>")
    assert picker.result == list(PROVIDERS)[0]


def test_cancel_leaves_the_result_none(root):
    picker = ProviderPicker(root)
    picker._cancel()
    assert picker.result is None


def test_picker_geometry_carries_a_position(root, monkeypatch):
    """選擇框要自己算好位置：conftest 的停放 fixture 只改寫帶 `+x+y` 的 geometry 字串，
    少了座標就由視窗管理員自行擺放，測試會把視窗開在執行中的遊戲上面。"""
    asked = []
    original = tk.Wm.geometry

    def spy(self, newGeometry=None):
        if newGeometry:
            asked.append(newGeometry)
        return original(self, newGeometry)

    monkeypatch.setattr(tk.Wm, "geometry", spy)
    picker = ProviderPicker(root)
    picker._cancel()
    assert asked and all("+" in geometry for geometry in asked)


def test_picker_grabs_input_and_releases_it_on_close(root):
    picker = ProviderPicker(root)
    assert picker.win.grab_current() is picker.win
    picker._pick("claude")
    assert root.grab_current() is None


def test_picker_hands_the_grab_back_to_a_modal_parent(root):
    """巢狀 modal：picker 常是從有 grab 的服務對話框裡開出來的。Tk 沒有 grab 堆疊，
    關掉 picker 只 release 會讓那個對話框默默失去 modal —— 使用者就能按到背後的
    〔儲存〕，把還在編輯的對話框連同 wait_window 一起拆掉。"""
    owner = tk.Toplevel(root)
    owner.geometry("200x100+100+100")   # 帶座標才會被 conftest 的停放 fixture 收走
    owner.grab_set()
    try:
        picker = ProviderPicker(owner)
        picker._cancel()
        assert root.grab_current() is owner
    finally:
        owner.grab_release()
        owner.destroy()


def test_escape_is_wired_to_cancel(root):
    """鍵盤事件只送得到焦點視窗，而測試不搶焦點（開發者常開著遊戲跑測試），
    所以只確認 <Escape> 綁上去了，關閉行為由〔取消〕那則測試涵蓋。"""
    picker = ProviderPicker(root)
    assert picker.win.bind("<Escape>")
    picker._cancel()
    assert root.grab_current() is None


def test_the_cancel_button_closes_the_picker(root):
    picker = ProviderPicker(root)
    _cancel_button(picker.win).invoke()
    assert picker.result is None
    assert root.grab_current() is None
