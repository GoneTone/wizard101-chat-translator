"""AddLine hook 與分頁狀態測試共用的假記憶體（不需遊戲）。"""
import struct

from src.reader.chat_hook import DATA_OFF, HEADER, SLOT_SIZE, SLOTS

RING = 0x10000000
RING_SIZE = HEADER + SLOTS * SLOT_SIZE
CALLER = 0x1407443B1
THIS = 0x7777


def _units(text: str) -> int:
    """UTF-16 字元數（stub 記錄的長度單位）。"""
    return len(text.encode("utf-16-le")) // 2


class _FakeRing:
    """以 bytearray 模擬遊戲內的環形緩衝。"""

    def __init__(self):
        self.buf = bytearray(RING_SIZE)
        self.after_read = None

    def slot(self, seq):
        return HEADER + ((seq - 1) % SLOTS) * SLOT_SIZE

    def put(self, seq, text, tab=0, slot_seq=None):
        off = self.slot(seq)
        data = text.encode("utf-16-le")
        struct.pack_into("<QQQIII", self.buf, off, seq if slot_seq is None else slot_seq,
                         CALLER, THIS, tab, len(data) // 2, len(data) // 2)
        self.buf[off + DATA_OFF:off + DATA_OFF + len(data)] = data
        struct.pack_into("<Q", self.buf, 0, max(seq, self.count))

    @property
    def count(self):
        return struct.unpack_from("<Q", self.buf)[0]

    async def read_bytes(self, addr, size):
        data = bytes(self.buf[addr - RING:addr - RING + size])
        if self.after_read:
            self.after_read(addr - RING, size)
        return data


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
