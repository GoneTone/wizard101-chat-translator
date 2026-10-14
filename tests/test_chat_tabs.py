import asyncio
import struct

from src.reader.chat_tabs import TAB_BUTTONS, read_wstring, visible_label


def _wstring(text: str, cap: int, heap_addr: int | None = None) -> tuple[bytes, dict[int, bytes]]:
    """組出 std::wstring 結構（位址 0x1000）與其堆積內容。"""
    data = text.encode("utf-16-le")
    if cap >= 8:
        head = struct.pack("<Q", heap_addr) + bytes(8)
        heap = {heap_addr: data}
    else:
        head = data.ljust(16, b"\0")
        heap = {}
    return head + struct.pack("<QQ", len(text), cap), heap


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


def test_visible_label_hides_unresolved_tokens():
    assert visible_label("<string;GUI2_00000367>") is None
    assert visible_label("") is None


def test_visible_label_keeps_unread_suffix():
    assert visible_label("队伍(1)") == "队伍(1)"


def test_tab_buttons_cover_the_verified_mapping():
    mapping = dict(TAB_BUTTONS)
    assert [mapping[n] for n in ("MainFilterButton", "GroupFilterButton", "HouseFilterButton")] == [0, 1, 2]
