from types import SimpleNamespace

import pytest

import src.hotkeys as hotkeys
from src.config import DEFAULT_CONFIG

KEY_DOWN, KEY_UP = hotkeys.keyboard.KEY_DOWN, hotkeys.keyboard.KEY_UP


def _code(name: str) -> int:
    return hotkeys.keyboard.key_to_scan_codes(name)[0]


CTRL, SHIFT, A, W, SPACE = (_code(n) for n in ("ctrl", "shift", "a", "w", "space"))


def _event(event_type: str, scan_code: int):
    return SimpleNamespace(event_type=event_type, scan_code=scan_code)


@pytest.fixture
def pressed(monkeypatch) -> set[int]:
    """目前按著的鍵（scan code）；換掉 is_pressed，測試不會真的掛上鍵盤 hook。"""
    held: set[int] = set()
    monkeypatch.setattr(hotkeys.keyboard, "is_pressed", lambda code: code in held)
    return held


def _interceptor(hotkey: str, handled: bool = True):
    triggers = []

    def on_trigger():
        triggers.append(1)
        return handled
    return hotkeys.HotkeyInterceptor(hotkey, on_trigger), triggers


def test_swallows_the_trigger_key_when_the_combo_is_handled(pressed):
    interceptor, triggers = _interceptor("ctrl+space")
    pressed.add(CTRL)
    assert interceptor.handle(_event(KEY_DOWN, SPACE)) is False
    assert interceptor.handle(_event(KEY_UP, SPACE)) is False
    assert triggers == [1]


def test_passes_the_key_through_when_the_callback_does_not_handle_it(pressed):
    # 前景不是遊戲時回呼回 False：Ctrl+Space 要留給其他程式（例如切換輸入法）
    interceptor, triggers = _interceptor("ctrl+space", handled=False)
    pressed.add(CTRL)
    assert interceptor.handle(_event(KEY_DOWN, SPACE)) is True
    assert interceptor.handle(_event(KEY_UP, SPACE)) is True
    assert triggers == [1]


def test_passes_the_bare_trigger_key_through(pressed):
    interceptor, triggers = _interceptor("ctrl+space")
    assert interceptor.handle(_event(KEY_DOWN, SPACE)) is True
    assert triggers == []


def test_extra_modifier_does_not_match(pressed):
    # Ctrl+Shift+Space 不能誤觸 Ctrl+Space（兩把熱鍵掛在同一顆鍵上）
    interceptor, triggers = _interceptor("ctrl+space")
    pressed.update({CTRL, SHIFT})
    assert interceptor.handle(_event(KEY_DOWN, SPACE)) is True
    assert triggers == []


def test_extra_non_modifier_key_still_matches(pressed):
    # 按住 W 走路時按熱鍵照樣觸發
    interceptor, triggers = _interceptor("ctrl+space")
    pressed.update({CTRL, W})
    assert interceptor.handle(_event(KEY_DOWN, SPACE)) is False
    assert triggers == [1]


def test_unrelated_keys_pass_without_calling_back(pressed):
    interceptor, triggers = _interceptor("ctrl+space")
    pressed.add(CTRL)
    assert interceptor.handle(_event(KEY_DOWN, A)) is True
    assert interceptor.handle(_event(KEY_DOWN, CTRL)) is True
    assert triggers == []


def test_key_repeat_is_swallowed_until_release_even_after_the_modifier_is_up(pressed):
    # 先放 Ctrl 再放 Space 時，中間的自動重複不能漏進遊戲
    interceptor, triggers = _interceptor("ctrl+space")
    pressed.add(CTRL)
    assert interceptor.handle(_event(KEY_DOWN, SPACE)) is False
    pressed.discard(CTRL)
    assert interceptor.handle(_event(KEY_DOWN, SPACE)) is False
    assert interceptor.handle(_event(KEY_UP, SPACE)) is False
    assert triggers == [1]
    assert interceptor.handle(_event(KEY_DOWN, SPACE)) is True   # 放開後的單獨 Space 照常


def test_combo_without_modifiers_swallows_whichever_key_completes_it(pressed):
    # a+space：最後按下、湊齊組合的那顆被吞；先按的那顆早已送出，與順序無關
    interceptor, triggers = _interceptor("a+space")
    pressed.add(A)
    assert interceptor.handle(_event(KEY_DOWN, SPACE)) is False
    interceptor.handle(_event(KEY_UP, SPACE))
    pressed.clear()
    pressed.add(SPACE)
    assert interceptor.handle(_event(KEY_DOWN, A)) is False
    assert triggers == [1, 1]


@pytest.mark.parametrize("hotkey", ["ctrl+shift", "ctrl+a, s"])
def test_rejects_combos_it_cannot_swallow(hotkey):
    with pytest.raises(hotkeys.CannotSwallow):
        hotkeys.HotkeyInterceptor(hotkey, lambda: True)


# --- register_hotkey：能吞就掛攔截器，不能吞就退回 add_hotkey，鍵名壞掉退回預設 ---
@pytest.fixture
def fake_keyboard(monkeypatch):
    calls = []

    def hook(callback, suppress=False):
        calls.append(("hook", suppress))
        return "hook-handle"

    def add_hotkey(hotkey, callback):
        calls.append(("add_hotkey", hotkey))
        return "hotkey-handle"
    monkeypatch.setattr(hotkeys.keyboard, "hook", hook)
    monkeypatch.setattr(hotkeys.keyboard, "add_hotkey", add_hotkey)
    monkeypatch.setattr(hotkeys.keyboard, "unhook",
                        lambda handle: calls.append(("unhook", handle)))
    monkeypatch.setattr(hotkeys.keyboard, "remove_hotkey",
                        lambda handle: calls.append(("remove_hotkey", handle)))
    logged = []
    monkeypatch.setattr(hotkeys, "log", logged.append)
    return calls, logged


def test_register_hotkey_installs_a_suppressing_hook(fake_keyboard):
    calls, _ = fake_keyboard
    unregister, used = hotkeys.register_hotkey("ctrl+space", lambda: True)
    assert used == "ctrl+space"
    assert calls == [("hook", True)]
    unregister()
    assert calls[-1] == ("unhook", "hook-handle")


@pytest.mark.parametrize("hotkey", ["ctrl+shift", "ctrl+a, s"])
def test_register_hotkey_falls_back_to_add_hotkey_when_it_cannot_swallow(fake_keyboard, hotkey):
    calls, logged = fake_keyboard
    unregister, used = hotkeys.register_hotkey(hotkey, lambda: True)
    assert used == hotkey
    assert calls == [("add_hotkey", hotkey)]
    assert any(hotkey in line for line in logged)
    unregister()
    assert calls[-1] == ("remove_hotkey", "hotkey-handle")


def test_register_hotkey_falls_back_to_the_default_on_an_unknown_key(fake_keyboard):
    calls, logged = fake_keyboard
    _, used = hotkeys.register_hotkey("ctrl+nope", lambda: True)
    assert used == DEFAULT_CONFIG["hotkey"]
    assert calls == [("hook", True)]
    assert any("ctrl+nope" in line for line in logged)


def test_register_hotkey_falls_back_to_the_given_fallback_on_an_unknown_key(fake_keyboard):
    # 區域熱鍵壞掉時要退回自己的預設值，不能撞上輸入框熱鍵的預設值
    _, used = hotkeys.register_hotkey("ctrl+nope", lambda: True, fallback="ctrl+shift+space")
    assert used == "ctrl+shift+space"
