import src.composer.paste as paste


def _capture_writes(monkeypatch) -> tuple[list[str], list[float]]:
    """把 keyboard.write 換成記錄器，回傳（逐次送出的字串，逐次的 delay）。"""
    chunks, delays = [], []

    def fake_write(text, delay=0, restore_state_after=True):
        chunks.append(text)
        delays.append(delay)
        fake_write.restores.append(restore_state_after)
    fake_write.restores = []
    monkeypatch.setattr(paste.keyboard, "write", fake_write)
    monkeypatch.setattr(paste.keyboard, "is_pressed", lambda key: False)
    return chunks, delays


def test_typing_never_re_presses_the_modifiers_the_user_held(monkeypatch):
    # keyboard.write 預設打完會把使用者按著的 Ctrl 重新按回去，但那一步套件自己記不到，
    # 之後每個字都會帶著 Ctrl 送出（Enter 變 Ctrl+Enter）；一律不還原
    _capture_writes(monkeypatch)
    paste.type_into_window(None, "ab", delay=0)
    assert paste.keyboard.write.restores == [False, False]


def test_type_into_window_types_text_one_char_at_a_time(monkeypatch):
    chunks, delays = _capture_writes(monkeypatch)
    paste.type_into_window(None, "wanna team up?", delay=0.03)
    assert "".join(chunks) == "wanna team up?"
    assert all(len(chunk) == 1 for chunk in chunks)
    assert set(delays) == {0.03}


def test_type_into_window_flattens_newlines(monkeypatch):
    # keyboard.write 會把換行打成 Enter：模型多行輸出不可觸發遊戲送出
    chunks, _ = _capture_writes(monkeypatch)
    paste.type_into_window(None, "line one\nline two\r\nline three\n")
    typed = "".join(chunks)
    assert typed == "line one line two line three"
    assert "\n" not in typed and "\r" not in typed


def test_type_into_window_stops_when_foreground_leaves_target(monkeypatch):
    # 打到一半 alt-tab：剩下的字不能打進切過去的視窗
    chunks, _ = _capture_writes(monkeypatch)
    target = 0x1234
    foreground = [target, target, target, 0x9999, target]
    monkeypatch.setattr(paste.win32gui, "IsWindow", lambda hwnd: True)
    monkeypatch.setattr(paste.win32gui, "GetForegroundWindow", lambda: foreground.pop(0))
    monkeypatch.setattr(paste, "force_foreground", lambda hwnd: None)
    monkeypatch.setattr(paste.time, "sleep", lambda s: None)
    paste.type_into_window(target, "hello", delay=0)
    assert "".join(chunks) == "hel"


def test_type_into_window_never_sends_enter(monkeypatch):
    # 自動鍵入絕不模擬 Enter：除了 keyboard.write，不應呼叫 keyboard.send / press_and_release
    _capture_writes(monkeypatch)
    calls = []
    monkeypatch.setattr(paste.keyboard, "send", lambda *a, **k: calls.append(("send", a)))
    monkeypatch.setattr(paste.keyboard, "press_and_release", lambda *a, **k: calls.append(("par", a)))
    paste.type_into_window(None, "hello")
    assert calls == []


def test_foreground_exe_returns_none_when_lookup_fails(monkeypatch):
    # 沒前景視窗／查詢失敗都要安靜回 None，讓呼叫端當成「不是遊戲」處理
    monkeypatch.setattr(paste.win32gui, "GetForegroundWindow", lambda: 0)
    assert paste.foreground_exe() is None

    def boom():
        raise OSError("no window")
    monkeypatch.setattr(paste.win32gui, "GetForegroundWindow", boom)
    assert paste.foreground_exe() is None


class _Event:
    def __init__(self, event_type: str):
        self.event_type = event_type


def _interceptor(monkeypatch, ctrl_down: bool, intercept: bool):
    monkeypatch.setattr(paste.keyboard, "is_pressed", lambda key: ctrl_down)
    pastes = []
    interceptor = paste.PasteInterceptor(should_intercept=lambda: intercept,
                                         on_paste=lambda: pastes.append(1))
    return interceptor, pastes


def test_interceptor_passes_plain_v_through(monkeypatch):
    interceptor, pastes = _interceptor(monkeypatch, ctrl_down=False, intercept=True)
    assert interceptor.handle(_Event(paste.keyboard.KEY_DOWN)) is True
    assert interceptor.handle(_Event(paste.keyboard.KEY_UP)) is True
    assert pastes == []


def test_interceptor_passes_ctrl_v_through_outside_the_game(monkeypatch):
    # 瀏覽器等其他視窗的 Ctrl+V 要完全不受影響
    interceptor, pastes = _interceptor(monkeypatch, ctrl_down=True, intercept=False)
    assert interceptor.handle(_Event(paste.keyboard.KEY_DOWN)) is True
    assert interceptor.handle(_Event(paste.keyboard.KEY_UP)) is True
    assert pastes == []


def test_interceptor_swallows_ctrl_v_in_the_game_and_pastes_once(monkeypatch):
    interceptor, pastes = _interceptor(monkeypatch, ctrl_down=True, intercept=True)
    assert interceptor.handle(_Event(paste.keyboard.KEY_DOWN)) is False
    assert interceptor.handle(_Event(paste.keyboard.KEY_UP)) is False
    assert pastes == [1]


def test_interceptor_ignores_key_repeat_until_released(monkeypatch):
    # 按住不放時 Windows 會連續送 KEY_DOWN：只算一次，放開後才重新武裝
    interceptor, pastes = _interceptor(monkeypatch, ctrl_down=True, intercept=True)
    for _ in range(3):
        assert interceptor.handle(_Event(paste.keyboard.KEY_DOWN)) is False
    assert pastes == [1]
    interceptor.handle(_Event(paste.keyboard.KEY_UP))
    interceptor.handle(_Event(paste.keyboard.KEY_DOWN))
    assert pastes == [1, 1]


def test_interceptor_keeps_swallowing_v_until_released_even_after_ctrl_is_up(monkeypatch):
    # 先放 Ctrl 再放 V 時，中間的自動重複 V 不能漏進遊戲變成一串 v
    ctrl = {"down": True}
    monkeypatch.setattr(paste.keyboard, "is_pressed", lambda key: ctrl["down"])
    pastes = []
    interceptor = paste.PasteInterceptor(lambda: True, lambda: pastes.append(1))
    assert interceptor.handle(_Event(paste.keyboard.KEY_DOWN)) is False
    ctrl["down"] = False
    assert interceptor.handle(_Event(paste.keyboard.KEY_DOWN)) is False
    assert interceptor.handle(_Event(paste.keyboard.KEY_UP)) is False
    assert pastes == [1]
    assert interceptor.handle(_Event(paste.keyboard.KEY_DOWN)) is True  # 放開後的單獨 V 照常


def test_install_paste_hook_blocks_the_v_key(monkeypatch):
    hooked = {}

    def fake_hook_key(key, callback, suppress=False):
        hooked.update(key=key, callback=callback, suppress=suppress)
        return "handle"
    monkeypatch.setattr(paste.keyboard, "hook_key", fake_hook_key)
    interceptor = paste.PasteInterceptor(lambda: True, lambda: None)
    assert paste.install_paste_hook(interceptor) == "handle"
    assert hooked == {"key": "v", "callback": interceptor.handle, "suppress": True}


def test_clipboard_text_reads_unicode_text(monkeypatch):
    calls = []
    monkeypatch.setattr(paste.win32clipboard, "OpenClipboard", lambda: calls.append("open"))
    monkeypatch.setattr(paste.win32clipboard, "CloseClipboard", lambda: calls.append("close"))
    monkeypatch.setattr(paste.win32clipboard, "GetClipboardData", lambda fmt: "來組隊嗎？")
    assert paste.clipboard_text() == "來組隊嗎？"
    assert calls == ["open", "close"]


def test_clipboard_text_returns_none_when_clipboard_has_no_text(monkeypatch):
    # 剪貼簿是圖片／空的：GetClipboardData 會丟例外，要安靜回 None 並仍關閉剪貼簿
    calls = []
    monkeypatch.setattr(paste.win32clipboard, "OpenClipboard", lambda: None)
    monkeypatch.setattr(paste.win32clipboard, "CloseClipboard", lambda: calls.append("close"))

    def boom(fmt):
        raise OSError("no text")
    monkeypatch.setattr(paste.win32clipboard, "GetClipboardData", boom)
    assert paste.clipboard_text() is None
    assert calls == ["close"]


def test_paste_clipboard_types_the_clipboard_with_the_configured_delay(monkeypatch):
    chunks, delays = _capture_writes(monkeypatch)
    monkeypatch.setattr(paste, "clipboard_text", lambda: "hello there")
    monkeypatch.setattr(paste.win32gui, "IsWindow", lambda hwnd: True)
    monkeypatch.setattr(paste.win32gui, "GetForegroundWindow", lambda: 0x42)
    assert paste.paste_clipboard(0x42, delay=0.05, single_line=False) is True
    assert "".join(chunks) == "hello there"
    assert set(delays) == {0.05}


def test_paste_clipboard_skips_empty_clipboard(monkeypatch):
    typed = []
    monkeypatch.setattr(paste, "_type_chars", lambda *a, **k: typed.append(a))
    for content in (None, ""):
        monkeypatch.setattr(paste, "clipboard_text", lambda content=content: content)
        assert paste.paste_clipboard(0x42, delay=0, single_line=False) is False
    assert typed == []


def test_paste_clipboard_types_the_text_verbatim_like_native_paste(monkeypatch):
    # Ctrl+V 不只用在聊天框：空白、換行都要照原樣送，不能像譯文那樣壓平或 strip；
    # Windows 的 CRLF 對原生貼上是一個換行，這裡也送一個 Enter
    chunks, _ = _capture_writes(monkeypatch)
    monkeypatch.setattr(paste, "clipboard_text", lambda: "  line one\r\nline two\n  ")
    monkeypatch.setattr(paste.win32gui, "IsWindow", lambda hwnd: True)
    monkeypatch.setattr(paste.win32gui, "GetForegroundWindow", lambda: 0x42)
    assert paste.paste_clipboard(0x42, delay=0, single_line=False) is True
    assert "".join(chunks) == "  line one\nline two\n  "


def test_paste_clipboard_turns_line_breaks_into_spaces_for_the_chat_box(monkeypatch):
    # 聊天輸入框是單行的：換行打成 Enter 會把打到一半的內容送出；其他空白照原樣
    chunks, _ = _capture_writes(monkeypatch)
    monkeypatch.setattr(paste, "clipboard_text", lambda: "\u6e2c\u8a66\r\nTest\n\n\n123 ")
    monkeypatch.setattr(paste.win32gui, "IsWindow", lambda hwnd: True)
    monkeypatch.setattr(paste.win32gui, "GetForegroundWindow", lambda: 0x42)
    assert paste.paste_clipboard(0x42, delay=0, single_line=True) is True
    assert "".join(chunks) == "測試 Test   123 "


def test_paste_clipboard_waits_for_ctrl_and_v_to_be_released_before_typing(monkeypatch):
    # 使用者手還按著 Ctrl 時就打字，Enter 會變成 Ctrl+Enter；先等放開再開始
    chunks, _ = _capture_writes(monkeypatch)
    monkeypatch.setattr(paste, "clipboard_text", lambda: "x")
    monkeypatch.setattr(paste.win32gui, "IsWindow", lambda hwnd: True)
    monkeypatch.setattr(paste.win32gui, "GetForegroundWindow", lambda: 0x42)
    held = {"ctrl": 3, "v": 1}   # 各自還會回報「按著」幾次
    timeline = []

    def is_pressed(key):
        timeline.append(("poll", key))
        held[key] -= 1
        return held[key] >= 0
    monkeypatch.setattr(paste.keyboard, "is_pressed", is_pressed)
    monkeypatch.setattr(paste.time, "sleep", lambda s: timeline.append(("sleep", s)))
    monkeypatch.setattr(paste.time, "monotonic", lambda: 0.0)
    assert paste.paste_clipboard(0x42, delay=0, single_line=False) is True
    assert "".join(chunks) == "x"
    assert timeline.count(("poll", "ctrl")) >= 3 and ("poll", "v") in timeline
    assert any(kind == "sleep" for kind, _ in timeline)


def test_paste_clipboard_gives_up_waiting_after_the_timeout(monkeypatch):
    # 使用者一直按著不放：等到上限就照打，不能讓貼上永遠卡住
    chunks, _ = _capture_writes(monkeypatch)
    monkeypatch.setattr(paste, "clipboard_text", lambda: "x")
    monkeypatch.setattr(paste.win32gui, "IsWindow", lambda hwnd: True)
    monkeypatch.setattr(paste.win32gui, "GetForegroundWindow", lambda: 0x42)
    monkeypatch.setattr(paste.keyboard, "is_pressed", lambda key: True)
    clock = {"now": 0.0}

    def sleep(seconds):
        clock["now"] += seconds
    monkeypatch.setattr(paste.time, "sleep", sleep)
    monkeypatch.setattr(paste.time, "monotonic", lambda: clock["now"])
    assert paste.paste_clipboard(0x42, delay=0, single_line=False) is True
    assert "".join(chunks) == "x"
    assert clock["now"] >= paste.MODIFIER_RELEASE_TIMEOUT


def test_paste_clipboard_stops_when_foreground_leaves_target(monkeypatch):
    chunks, _ = _capture_writes(monkeypatch)
    monkeypatch.setattr(paste, "clipboard_text", lambda: "hello")
    foreground = [0x42, 0x42, 0x9999, 0x42, 0x42]
    monkeypatch.setattr(paste.win32gui, "IsWindow", lambda hwnd: True)
    monkeypatch.setattr(paste.win32gui, "GetForegroundWindow", lambda: foreground.pop(0))
    paste.paste_clipboard(0x42, delay=0, single_line=False)
    assert "".join(chunks) == "he"


def test_paste_clipboard_ignores_a_second_paste_while_typing(monkeypatch):
    import threading

    started, release = threading.Event(), threading.Event()

    def slow_type(hwnd, text, delay):
        started.set()
        release.wait(timeout=5)
    monkeypatch.setattr(paste, "clipboard_text", lambda: "slow")
    monkeypatch.setattr(paste, "_type_chars", slow_type)
    worker = threading.Thread(target=paste.paste_clipboard, args=(0x42, 0, False), daemon=True)
    worker.start()
    assert started.wait(timeout=5)
    try:
        assert paste.paste_clipboard(0x42, delay=0, single_line=False) is False
    finally:
        release.set()
        worker.join(timeout=5)
    assert paste.paste_clipboard(0x42, delay=0, single_line=False) is True   # 打完就能再貼


def test_interceptor_swallows_v_while_typing_is_in_progress(monkeypatch):
    # 打字途中每個字都會先把使用者按著的 Ctrl 放開，第二次 Ctrl+V 的 Ctrl 狀態查不到：
    # 鍵入進行中 V 一律吞掉，否則會在訊息中途插進一個 v
    monkeypatch.setattr(paste.keyboard, "is_pressed", lambda key: False)
    monkeypatch.setattr(paste, "typing_in_progress", lambda: True)
    pastes = []
    interceptor = paste.PasteInterceptor(lambda: True, lambda: pastes.append(1))
    assert interceptor.handle(_Event(paste.keyboard.KEY_DOWN)) is False
    assert interceptor.handle(_Event(paste.keyboard.KEY_UP)) is False
    assert pastes == []


def test_typing_in_progress_is_true_only_while_chars_are_being_typed(monkeypatch):
    seen = []
    monkeypatch.setattr(paste.keyboard, "write",
                        lambda text, delay=0, restore_state_after=True:
                        seen.append(paste.typing_in_progress()))
    monkeypatch.setattr(paste.keyboard, "is_pressed", lambda key: False)
    assert paste.typing_in_progress() is False
    paste.type_into_window(None, "ab", delay=0)
    assert seen == [True, True]
    assert paste.typing_in_progress() is False
