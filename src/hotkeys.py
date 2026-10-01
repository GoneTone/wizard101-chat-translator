"""全域熱鍵：在按鍵送達前景程式之前決定吞不吞，熱鍵生效時遊戲就不會收到那顆鍵
（例如空白鍵讓 NPC 對話跳到下一句）。"""
import keyboard

from src.config import DEFAULT_CONFIG
from src.log import log

MODIFIER_GROUPS = ("ctrl", "shift", "alt", "windows")


class CannotSwallow(ValueError):
    """熱鍵組合沒辦法逐鍵攔截：只有修飾鍵，或是多段組合（如 `ctrl+a, s`）。"""


class HotkeyInterceptor:
    """一組熱鍵的逐次按鍵決策：`handle(event)` 回 True 放行、False 吞掉。
    掛在 keyboard 的 blocking hook 上，跑在 Windows 低階 hook 裡，`on_trigger` 必須立刻返回。
    組合裡的非修飾鍵按下、湊齊組合（修飾鍵須完全吻合）時呼叫 `on_trigger()`，回 True 表示
    有接手，就吞掉這顆鍵直到放開；先按下的鍵早已送出，吞不回來。"""

    def __init__(self, hotkey: str, on_trigger) -> None:
        steps = keyboard.parse_hotkey(hotkey)
        if len(steps) != 1:
            raise CannotSwallow(f"{hotkey!r} is a multi-step hotkey")
        self._keys = steps[0]
        self._trigger_codes = {code for key in self._keys if not _is_modifier(key)
                               for code in key}
        if not self._trigger_codes:
            raise CannotSwallow(f"{hotkey!r} has only modifier keys")
        self._unwanted_modifiers = [codes for codes in map(_scan_codes, MODIFIER_GROUPS)
                                    if not any(set(key) & codes for key in self._keys)]
        self._on_trigger = on_trigger
        self._held: set[int] = set()  # 已接手、吞到放開為止的鍵；按住時 Windows 會連續送 KEY_DOWN

    def handle(self, event) -> bool:
        code = event.scan_code
        if code in self._held:
            if event.event_type == keyboard.KEY_UP:
                self._held.discard(code)
            return False
        if code not in self._trigger_codes:
            return True
        return not self._should_swallow(event)

    def _should_swallow(self, event) -> bool:
        if event.event_type != keyboard.KEY_DOWN or not self._completes_combo(event.scan_code):
            return False
        if not self._on_trigger():
            return False
        self._held.add(event.scan_code)
        return True

    def _completes_combo(self, code: int) -> bool:
        others_held = all(_any_pressed(key) for key in self._keys if code not in key)
        return others_held and not any(_any_pressed(codes) for codes in self._unwanted_modifiers)


def _scan_codes(name: str) -> set[int]:
    return set(keyboard.key_to_scan_codes(name, error_if_missing=False))


def _is_modifier(key: tuple[int, ...]) -> bool:
    return any(keyboard.is_modifier(code) for code in key)


def _any_pressed(codes) -> bool:
    return any(keyboard.is_pressed(code) for code in codes)


def install(interceptor: HotkeyInterceptor):
    """把攔截器掛上 keyboard 的 blocking hook（suppress 才能逐次決定吞不吞），回傳解除用的函式。"""
    handle = keyboard.hook(interceptor.handle, suppress=True)
    return lambda: keyboard.unhook(handle)


def register_hotkey(hotkey: str, callback,
                    fallback: str = DEFAULT_CONFIG["hotkey"]) -> tuple[object, str]:
    """註冊全域熱鍵，回傳（解除用的函式，實際生效的熱鍵）。`callback()` 回 True 表示有接手。
    手改 config.json 填了不認得的鍵名時退回 fallback：windowed exe 在這裡炸掉等於無聲退出，
    而設定視窗改熱鍵時舊的已先解除，失敗會讓輸入框再也呼不出來。"""
    try:
        return _register(hotkey, callback), hotkey
    except ValueError as exc:
        log(f"[hotkey] {hotkey!r} is not a valid key combination ({exc}); using {fallback!r}")
        return _register(fallback, callback), fallback


def _register(hotkey: str, callback):
    try:
        interceptor = HotkeyInterceptor(hotkey, callback)
    except CannotSwallow as exc:
        log(f"[hotkey] {exc}; registered without swallowing, its keys still reach the game")
        handle = keyboard.add_hotkey(hotkey, callback)
        return lambda: keyboard.remove_hotkey(handle)
    log(f"[hotkey] {hotkey!r} registered; its trigger key is swallowed when handled")
    return install(interceptor)
