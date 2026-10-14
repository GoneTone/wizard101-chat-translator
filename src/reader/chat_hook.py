"""收訊端：hook 遊戲的 ChatBox::AddLine，把每次呼叫寫進環形緩衝，再由讀取端依序號取出。

stub 在遊戲執行緒上執行；讀取端（`read_ring`、`group_broadcasts`）是純函式，記憶體讀取
以參數注入，不需遊戲即可測試。緩衝配置與 seqlock 規則見 docs/superpowers/specs 的設計文件。
"""
import struct
from typing import NamedTuple

from wizwalker.memory.hooks import AutoBotBaseHook, SimpleHook

SLOTS = 256
SLOT_SIZE = 0x800
HEADER = 0x40  # u64 累計呼叫數，槽位從其後開始
DATA_OFF = 0x30
MAX_CHARS = (SLOT_SIZE - DATA_OFF) // 2

# 48 89 5C 24 20 = mov [rsp+20],rbx：5 bytes、無 RIP 相對定址，可直接搬進 stub
PATTERN = (rb"\x48\x89\x5C\x24\x20\x55\x56\x57\x41\x54\x41\x55\x41\x56\x41\x57"
           rb"\x48\x8D\xAC\x24....\x48\x81\xEC....\x48\x8B\x05....\x48\x33\xC4"
           rb"\x48\x89\x85....\x44\x89\x44\x24.\x48\x89\x55.\x48\x8B\xF9"
           rb"\x48\x89\x55.\x41\x83\xF8\x05\x7C")


class HookRecord(NamedTuple):
    """一次 AddLine 呼叫；`len(text) < length` 代表內容被截斷。"""
    seq: int
    caller: int
    this: int
    tab: int
    length: int
    text: str


class RingRead(NamedTuple):
    """一輪讀取結果；`lost` 是本輪確定讀不到（已被覆寫）的記錄數。"""
    records: list[HookRecord]
    last_seq: int
    lost: int


def stub_bytes(ring: int) -> bytes:
    """AddLine hook 的 stub 機器碼（不含 SimpleHook 附加的回跳）。"""
    q = lambda v: struct.pack("<Q", v)  # noqa: E731
    d = lambda v: struct.pack("<I", v)  # noqa: E731
    return b"".join([
        b"\x48\x89\x5C\x24\x20",              # mov [rsp+20],rbx   (original)
        b"\x9C\x50\x51\x56\x57\x41\x51\x41\x52",  # pushfq; push rax,rcx,rsi,rdi,r9,r10
        b"\x4C\x8B\x4C\x24\x38",              # mov r9,[rsp+38]    return address
        b"\x48\xB8" + q(ring),                # mov rax,ring       counter
        b"\x4C\x8B\x10",                      # mov r10,[rax]
        b"\x49\xFF\xC2",                      # inc r10            seq (1-based)
        b"\x4C\x89\x10",                      # mov [rax],r10
        b"\x4C\x89\xD0",                      # mov rax,r10
        b"\x48\xFF\xC8",                      # dec rax
        b"\x25" + d(SLOTS - 1),               # and eax,imm32 (imm8 form sign-extends 0xFF)
        b"\x48\xC1\xE0\x0B",                  # shl rax,11         * SLOT_SIZE
        b"\x48\x05" + d(HEADER),              # add rax,HEADER
        b"\x48\xBF" + q(ring),                # mov rdi,ring
        b"\x48\x01\xC7",                      # add rdi,rax        rdi = slot
        b"\x48\xC7\x07\x00\x00\x00\x00",      # mov qword [rdi],0  slot busy
        b"\x4C\x89\x4F\x08",                  # mov [rdi+8],r9
        b"\x48\x89\x4F\x10",                  # mov [rdi+10],rcx   this
        b"\x44\x89\x47\x18",                  # mov [rdi+18],r8d   tab
        b"\x48\x8B\x4A\x10",                  # mov rcx,[rdx+10]   wstring size
        b"\x89\x4F\x20",                      # mov [rdi+20],ecx   original length
        b"\x48\x81\xF9" + d(MAX_CHARS),       # cmp rcx,MAX_CHARS
        b"\x76\x05",                          # jbe +5
        b"\xB9" + d(MAX_CHARS),               # mov ecx,MAX_CHARS
        b"\x89\x4F\x1C",                      # mov [rdi+1C],ecx   copied length
        b"\x48\x89\xD6",                      # mov rsi,rdx        SSO buffer
        b"\x48\x83\x7A\x18\x08",              # cmp qword [rdx+18],8  capacity
        b"\x72\x03",                          # jb +3
        b"\x48\x8B\x32",                      # mov rsi,[rdx]      heap buffer
        b"\x48\x83\xC7" + bytes([DATA_OFF]),  # add rdi,DATA_OFF
        b"\xFC",                              # cld
        b"\x66\xF3\xA5",                      # rep movsw
        b"\x48\xBF" + q(ring),                # mov rdi,ring
        b"\x48\x01\xC7",                      # add rdi,rax
        b"\x4C\x89\x17",                      # mov [rdi],r10      publish seq last
        b"\x41\x5A\x41\x59\x5F\x5E\x59\x58\x9D",  # pop r10,r9,rdi,rsi,rcx,rax; popfq
    ])


class AddLineHook(SimpleHook):
    """把每次 ChatBox::AddLine 呼叫的（序號, 回傳位址, this, 分頁, 內容）寫進 `ring`。"""
    pattern = PATTERN
    instruction_length = 5
    exports = [("ring", HEADER + SLOTS * SLOT_SIZE)]

    async def get_hook_address(self, size: int) -> int:
        return await self.alloc(256)  # 預設 50 bytes 放不下這個 stub

    async def bytecode_generator(self, packed_exports):
        return stub_bytes(self.ring)  # SimpleHook.get_hook_bytecode 已先 setattr ring

    async def unhook(self):
        # 不釋放 ring：還原跳躍點的瞬間，遊戲執行緒可能還在 stub 內寫入
        await AutoBotBaseHook.unhook(self)


async def read_ring(read_bytes, ring: int, last_seq: int) -> RingRead:
    """讀出序號 `last_seq` 之後的新記錄；`read_bytes` 是 `async (addr, size) -> bytes`。"""
    count = struct.unpack("<Q", await read_bytes(ring, 8))[0]
    lost = 0
    if count - last_seq > SLOTS:
        lost += count - last_seq - SLOTS
        last_seq = count - SLOTS
    records = []
    while last_seq < count:
        seq = last_seq + 1
        slot = ring + HEADER + ((seq - 1) % SLOTS) * SLOT_SIZE
        got, caller, this, tab, copied, length = struct.unpack_from(
            "<QQQIII", await read_bytes(slot, DATA_OFF))
        if got < seq:
            break  # 0＝寫入中；更舊的序號＝stub 已更新累計數、還沒清槽，都是下一輪再讀
        last_seq = seq
        if got > seq:
            lost += 1
            continue
        raw = await read_bytes(slot + DATA_OFF, copied * 2) if copied else b""
        text = raw.decode("utf-16-le", "replace")
        if struct.unpack("<Q", await read_bytes(slot, 8))[0] != seq:
            lost += 1  # 讀內容期間被覆寫
            continue
        records.append(HookRecord(seq, caller, this, tab, length, text))
    return RingRead(records, last_seq, lost)


def _continues_broadcast(run: list[HookRecord], rec: HookRecord) -> bool:
    prev = run[-1]
    if rec.seq != prev.seq + 1 or rec.text != prev.text:
        return False
    return rec.tab == len(run) if len(run) < 4 else rec.tab > max(prev.tab, 4)


def group_broadcasts(records: list[HookRecord]) -> list[tuple[tuple[int, ...], HookRecord]]:
    """把伺服器廣播（連號、同字、分頁 0,1,2,3 再接遞增的頻道分頁）收斂成一組，回傳（分頁組, 代表記錄）。"""
    # shortcut: 一組廣播被兩輪讀取切開時會變成各分頁各一則；廣播極少且寫入在微秒內完成，真的發生再改成保留尾段
    groups = []
    i = 0
    while i < len(records):
        run = [records[i]]
        if run[0].tab == 0:
            while i + len(run) < len(records) and _continues_broadcast(run, records[i + len(run)]):
                run.append(records[i + len(run)])
        if len(run) < 4:
            run = run[:1]
        groups.append((tuple(r.tab for r in run), run[0]))
        i += len(run)
    return groups
