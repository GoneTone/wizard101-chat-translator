import src.composer.paste as paste


def test_type_into_window_types_text(monkeypatch):
    written = {}
    monkeypatch.setattr(paste.keyboard, "write",
                        lambda text, delay=0: written.update(text=text, delay=delay))
    paste.type_into_window(None, "wanna team up?", delay=0.03)
    assert written["text"] == "wanna team up?"
    assert written["delay"] == 0.03


def test_type_into_window_flattens_newlines(monkeypatch):
    # keyboard.write 會把換行打成 Enter：模型多行輸出不可觸發遊戲送出
    written = {}
    monkeypatch.setattr(paste.keyboard, "write",
                        lambda text, delay=0: written.update(text=text))
    paste.type_into_window(None, "line one\nline two\r\nline three\n")
    assert written["text"] == "line one line two line three"
    assert "\n" not in written["text"] and "\r" not in written["text"]


def test_type_into_window_never_sends_enter(monkeypatch):
    # 自動鍵入絕不模擬 Enter：除了 keyboard.write，不應呼叫 keyboard.send / press_and_release
    monkeypatch.setattr(paste.keyboard, "write", lambda text, delay=0: None)
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
