import asyncio
import faulthandler
import struct
import sys
from unittest.mock import AsyncMock

import pytest
from unicorn import UC_HOOK_MEM_WRITE, Uc
from unicorn.unicorn_const import UC_ARCH_X86, UC_MODE_64
from unicorn.x86_const import (
    UC_X86_REG_EFLAGS,
    UC_X86_REG_R8,
    UC_X86_REG_R9,
    UC_X86_REG_R10,
    UC_X86_REG_RAX,
    UC_X86_REG_RBX,
    UC_X86_REG_RCX,
    UC_X86_REG_RDI,
    UC_X86_REG_RDX,
    UC_X86_REG_RSI,
    UC_X86_REG_RSP,
)

from src.reader.chat_hook import (
    DATA_OFF,
    HEADER,
    MAX_CHARS,
    SLOT_SIZE,
    SLOTS,
    AddLineHook,
    HookRecord,
    group_broadcasts,
    read_ring,
    stub_bytes,
)

RING, CODE, STACK, HEAP, STR = 0x10000000, 0x20000000, 0x30000000, 0x40000000, 0x50000000
RING_SIZE = HEADER + SLOTS * SLOT_SIZE
CALLER = 0x1407443B1
THIS = 0x7777
DF = 0x400


@pytest.fixture(autouse=True)
def _quiet_faulthandler():
    # unicorn 在 Windows 建立映射時會觸發並自行處理 access violation，faulthandler 會誤印成 fatal
    faulthandler.disable()
    yield
    faulthandler.enable(sys.__stderr__)


def _emulate(text, tab, start_count):
    """在 unicorn 跑一次 stub，回傳 (emulator, 初始暫存器, rsp, 寫入位址清單)。"""
    code = stub_bytes(RING)
    mu = Uc(UC_ARCH_X86, UC_MODE_64)
    # 環形緩衝多映射一頁，越界寫入不會讓 unicorn 當掉，留給寫入位址斷言抓
    for addr, size in ((RING, RING_SIZE + 0x2000), (CODE, 0x1000), (STACK, 0x10000),
                       (HEAP, 0x10000), (STR, 0x1000)):
        mu.mem_map(addr, (size + 0xFFF) & ~0xFFF)
    mu.mem_write(RING, struct.pack("<Q", start_count))
    mu.mem_write(CODE, code)
    w = text.encode("utf-16-le")
    if len(text) < 8:
        mu.mem_write(STR, w.ljust(16, b"\0") + struct.pack("<QQ", len(text), 7))
    else:
        mu.mem_write(HEAP, w)
        mu.mem_write(STR, struct.pack("<QQQQ", HEAP, 0, len(text), len(text) | 7))
    rsp = STACK + 0x8000
    mu.mem_write(rsp, struct.pack("<Q", CALLER))
    regs = {UC_X86_REG_RSP: rsp, UC_X86_REG_RCX: THIS, UC_X86_REG_RDX: STR, UC_X86_REG_R8: tab,
            UC_X86_REG_RBX: 0xB0B0, UC_X86_REG_RAX: 0xAAAA, UC_X86_REG_RSI: 0x5151,
            UC_X86_REG_RDI: 0xD1D1, UC_X86_REG_R9: 0x9999, UC_X86_REG_R10: 0x1010,
            UC_X86_REG_EFLAGS: 0x202 | DF}
    for reg, value in regs.items():
        mu.reg_write(reg, value)
    writes = []
    mu.hook_add(UC_HOOK_MEM_WRITE, lambda _uc, _access, addr, size, _v, _d: writes.append((addr, size)))
    mu.emu_start(CODE, CODE + len(code))
    return mu, regs, rsp, writes


def _reader(mu):
    async def read_bytes(addr, size):
        return bytes(mu.mem_read(addr, size))
    return read_bytes


def test_stub_copies_sso_and_heap_strings_and_restores_registers():
    assert len(stub_bytes(RING)) + 5 <= 256  # 加上 SimpleHook 的回跳仍放得進 get_hook_address 的配置
    for text, tab, seq in (("hi", 3, 1), ("[Bob] hello there everyone", 0, 2)):
        mu, regs, rsp, _ = _emulate(text, tab, seq - 1)
        for reg, value in regs.items():
            if reg != UC_X86_REG_EFLAGS:
                assert mu.reg_read(reg) == value, reg
        assert mu.reg_read(UC_X86_REG_EFLAGS) & DF
        assert struct.unpack("<Q", mu.mem_read(rsp + 0x20, 8))[0] == 0xB0B0
        assert struct.unpack("<Q", mu.mem_read(RING, 8))[0] == seq
        got = asyncio.run(read_ring(_reader(mu), RING, seq - 1))
        assert got.records == [HookRecord(seq, CALLER, THIS, tab, len(text), text)]
        assert (got.last_seq, got.lost) == (seq, 0)


def test_stub_truncates_long_lines_and_keeps_original_length():
    mu, *_ = _emulate("x" * 1500, 5, 0)
    (record,) = asyncio.run(read_ring(_reader(mu), RING, 0)).records
    assert record.length == 1500
    assert len(record.text) == MAX_CHARS


def test_stub_wraps_past_the_slot_count():
    mu, _, _, writes = _emulate("wrap me please", 2, SLOTS)
    ring_writes = [(addr, size) for addr, size in writes if not STACK <= addr < STACK + 0x10000]
    assert ring_writes
    assert all(RING <= addr <= RING + RING_SIZE - size for addr, size in ring_writes)
    assert struct.unpack("<Q", mu.mem_read(RING + HEADER, 8))[0] == SLOTS + 1
    (record,) = asyncio.run(read_ring(_reader(mu), RING, SLOTS)).records
    assert (record.seq, record.text) == (SLOTS + 1, "wrap me please")


class _FakeRing:
    """以 bytearray 模擬遊戲內的環形緩衝。"""

    def __init__(self):
        self.buf = bytearray(RING_SIZE)
        self.after_read = None

    def slot(self, seq):
        return HEADER + ((seq - 1) % SLOTS) * SLOT_SIZE

    def put(self, seq, text, tab=0, slot_seq=None):
        off = self.slot(seq)
        struct.pack_into("<QQQIII", self.buf, off, seq if slot_seq is None else slot_seq,
                         CALLER, THIS, tab, len(text), len(text))
        self.buf[off + DATA_OFF:off + DATA_OFF + len(text) * 2] = text.encode("utf-16-le")
        struct.pack_into("<Q", self.buf, 0, max(seq, self.count))

    @property
    def count(self):
        return struct.unpack_from("<Q", self.buf)[0]

    async def read_bytes(self, addr, size):
        data = bytes(self.buf[addr - RING:addr - RING + size])
        if self.after_read:
            self.after_read(addr - RING, size)
        return data


def _read(ring, last_seq):
    return asyncio.run(read_ring(ring.read_bytes, RING, last_seq))


def test_read_ring_returns_new_records_in_order():
    ring = _FakeRing()
    for seq, text in ((1, "a"), (2, "b"), (3, "c")):
        ring.put(seq, text, tab=seq)
    got = _read(ring, 0)
    assert [(r.seq, r.tab, r.text) for r in got.records] == [(1, 1, "a"), (2, 2, "b"), (3, 3, "c")]
    assert (got.last_seq, got.lost) == (3, 0)


def test_read_ring_stops_at_a_slot_being_written():
    ring = _FakeRing()
    ring.put(1, "a")
    ring.put(2, "b", slot_seq=0)
    ring.put(3, "c")
    got = _read(ring, 0)
    assert [r.text for r in got.records] == ["a"]
    assert (got.last_seq, got.lost) == (1, 0)


def test_read_ring_counts_an_overwritten_slot_as_lost():
    ring = _FakeRing()
    ring.put(1, "a")
    ring.put(2, "b", slot_seq=2 + SLOTS)
    ring.put(3, "c")
    got = _read(ring, 0)
    assert [r.text for r in got.records] == ["a", "c"]
    assert (got.last_seq, got.lost) == (3, 1)


def test_overflow_reports_lost_and_resumes_at_oldest_valid_slot():
    ring = _FakeRing()
    last_seq = 10
    count = last_seq + SLOTS + 5
    for seq in range(last_seq + 6, count + 1):
        ring.put(seq, str(seq))
    got = _read(ring, last_seq)
    assert got.lost == 5
    assert [r.seq for r in got.records] == list(range(last_seq + 6, count + 1))
    assert len(got.records) == SLOTS
    assert got.last_seq == count


def test_read_ring_rereads_seq_to_detect_a_torn_read():
    ring = _FakeRing()
    ring.put(1, "a")
    ring.put(2, "torn")

    def overwrite_after_content(off, _size):
        if off == ring.slot(2) + DATA_OFF:
            struct.pack_into("<Q", ring.buf, ring.slot(2), 2 + SLOTS)

    ring.after_read = overwrite_after_content
    got = _read(ring, 0)
    assert [r.text for r in got.records] == ["a"]
    assert (got.last_seq, got.lost) == (2, 1)


def _rec(seq, tab, text="same"):
    return HookRecord(seq, CALLER, THIS, tab, len(text), text)


def test_group_broadcasts_collapses_tabs_0_to_3_plus_channels():
    records = [_rec(s, t) for s, t in zip(range(1, 6), (0, 1, 2, 3, 5), strict=True)]
    assert group_broadcasts(records) == [((0, 1, 2, 3, 5), records[0])]


def test_group_broadcasts_keeps_ordinary_lines_apart():
    pair = [_rec(1, 0), _rec(2, 1)]
    assert group_broadcasts(pair) == [((0,), pair[0]), ((1,), pair[1])]
    gapped = [_rec(s, t) for s, t in zip((1, 2, 4, 5), (0, 1, 2, 3), strict=True)]
    assert group_broadcasts(gapped) == [((r.tab,), r) for r in gapped]


def test_unhook_restores_the_jump_but_keeps_the_ring():
    hook = AddLineHook.__new__(AddLineHook)
    hook.jump_address, hook.jump_original_bytecode, hook.ring = 0x1000, b"\x48\x89\x5C\x24\x20", RING
    hook.write_bytes, hook.free = AsyncMock(), AsyncMock()
    asyncio.run(hook.unhook())
    hook.write_bytes.assert_awaited_once_with(0x1000, b"\x48\x89\x5C\x24\x20")
    hook.free.assert_not_awaited()
