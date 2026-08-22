import src.composer.paste as paste


def test_type_into_window_types_text(monkeypatch):
    written = {}
    monkeypatch.setattr(paste.keyboard, "write",
                        lambda text, delay=0: written.update(text=text, delay=delay))
    paste.type_into_window(None, "wanna team up?", delay=0.03)
    assert written["text"] == "wanna team up?"
    assert written["delay"] == 0.03


def test_type_into_window_never_sends_enter(monkeypatch):
    # 自動鍵入絕不模擬 Enter：除了 keyboard.write，不應呼叫 keyboard.send / press_and_release
    monkeypatch.setattr(paste.keyboard, "write", lambda text, delay=0: None)
    calls = []
    monkeypatch.setattr(paste.keyboard, "send", lambda *a, **k: calls.append(("send", a)))
    monkeypatch.setattr(paste.keyboard, "press_and_release", lambda *a, **k: calls.append(("par", a)))
    paste.type_into_window(None, "hello")
    assert calls == []
