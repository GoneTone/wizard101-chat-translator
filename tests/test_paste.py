from src.composer.paste import get_clipboard, set_clipboard


def test_clipboard_roundtrip_unicode():
    set_clipboard("hey wanna team up? 一起打副本")
    assert get_clipboard() == "hey wanna team up? 一起打副本"


def test_clipboard_overwrite():
    set_clipboard("first")
    set_clipboard("second")
    assert get_clipboard() == "second"
