import asyncio
import struct

from src.reader.chat_tabs import TAB_BUTTONS, read_wstring, visible_label
from tests.reader.chat_hook_helpers import _wstring


def _read(text: str, cap: int, size: int | None = None) -> str:
    raw, heap = _wstring(text, cap, 0x9000)
    if size is not None:
        raw = raw[:16] + struct.pack("<Q", size) + raw[24:]
    mem = {0x1000: raw, **heap}

    async def read_bytes(addr, n):
        return mem[addr][:n]

    return asyncio.run(read_wstring(read_bytes, 0x1000))


def test_read_wstring_inline_short_string():
    assert _read("主要的", 7) == "主要的"


def test_read_wstring_five_chars_inline():
    assert _read("队伍(2)", 7) == "队伍(2)"


def test_read_wstring_heap_string():
    assert _read("Party 1 (3)", 15) == "Party 1 (3)"


def test_read_wstring_empty_and_absurd_sizes():
    assert _read("", 7) == ""
    assert _read("abc", 7, size=10**7) == ""


def test_read_wstring_size_limit_boundary():
    assert _read("a" * 4096, 4096) == "a" * 4096
    assert _read("a" * 4097, 4097) == ""


def test_read_wstring_sso_capacity_threshold():
    assert _read("abcdefg", 7) == "abcdefg"
    assert _read("abc", 8) == "abc"


def test_read_wstring_size_beyond_capacity_is_rejected():
    assert _read("abc", 7, size=8) == ""
    assert _read("abc", 2, size=3) == ""


def test_visible_label_hides_unresolved_tokens():
    assert visible_label("<string;GUI2_00000367>") is None
    assert visible_label("") is None


def test_visible_label_keeps_unread_suffix():
    assert visible_label("队伍(1)") == "队伍(1)"


def test_tab_buttons_cover_the_verified_mapping():
    mapping = dict(TAB_BUTTONS)
    assert [mapping[n] for n in ("MainFilterButton", "GroupFilterButton", "HouseFilterButton")] == [0, 1, 2]
    assert len(TAB_BUTTONS) == 9
    assert len({n for n, _ in TAB_BUTTONS}) == 9
    assert len({i for _, i in TAB_BUTTONS}) == 9
