//! 收訊端：hook 遊戲的 ChatBox::AddLine，把每次呼叫寫進環形緩衝。
//!
//! stub 在遊戲執行緒上執行，位元組與 Python 版 `chat_hook.stub_bytes` 逐一相同。

use super::hooks::{AutobotCave, HookError, JumpPatch, SimpleHook};
use super::memory::{MemError, ProcessMemory};

/// 環形緩衝的槽位數。
pub const SLOTS: u64 = 256;
/// 每個槽位的大小。
pub const SLOT_SIZE: u64 = 0x800;
/// 緩衝開頭的 u64 累計呼叫數，槽位從其後開始。
pub const HEADER: u64 = 0x40;
/// 槽位內文字的起點。
pub const DATA_OFF: u64 = 0x30;
/// 每則最多複製的 UTF-16 字元數。
pub const MAX_CHARS: u64 = (SLOT_SIZE - DATA_OFF) / 2;

/// `ChatBox::AddLine` 開頭的 pattern；48 89 5C 24 20 = mov [rsp+20],rbx：5 bytes、無 RIP 相對定址，可直接搬進 stub。
pub const PATTERN: &str = concat!(
    r"\x48\x89\x5C\x24\x20\x55\x56\x57\x41\x54\x41\x55\x41\x56\x41\x57",
    r"\x48\x8D\xAC\x24....\x48\x81\xEC....\x48\x8B\x05....\x48\x33\xC4",
    r"\x48\x89\x85....\x44\x89\x44\x24.\x48\x89\x55.\x48\x8B\xF9",
    r"\x48\x89\x55.\x41\x83\xF8\x05\x7C",
);

/// AddLine hook 的 stub 機器碼（不含回跳）。
pub fn stub_bytes(ring: u64) -> Vec<u8> {
    let q = ring.to_le_bytes();
    let d = |v: u64| (v as u32).to_le_bytes();
    [
        &[0x48, 0x89, 0x5C, 0x24, 0x20][..], // mov [rsp+20],rbx   (original)
        &[0x9C, 0x50, 0x51, 0x56, 0x57, 0x41, 0x51, 0x41, 0x52], // pushfq; push rax,rcx,rsi,rdi,r9,r10
        &[0x4C, 0x8B, 0x4C, 0x24, 0x38], // mov r9,[rsp+38]    return address
        &[0x48, 0xB8],                   // mov rax,ring       counter
        &q,
        &[0x4C, 0x8B, 0x10], // mov r10,[rax]
        &[0x49, 0xFF, 0xC2], // inc r10            seq (1-based)
        &[0x4C, 0x89, 0x10], // mov [rax],r10
        &[0x4C, 0x89, 0xD0], // mov rax,r10
        &[0x48, 0xFF, 0xC8], // dec rax
        &[0x25],             // and eax,imm32 (imm8 form sign-extends 0xFF)
        &d(SLOTS - 1),
        &[0x48, 0xC1, 0xE0, 0x0B], // shl rax,11         * SLOT_SIZE
        &[0x48, 0x05],             // add rax,HEADER
        &d(HEADER),
        &[0x48, 0xBF], // mov rdi,ring
        &q,
        &[0x48, 0x01, 0xC7], // add rdi,rax        rdi = slot
        &[0x48, 0xC7, 0x07, 0x00, 0x00, 0x00, 0x00], // mov qword [rdi],0  slot busy
        &[0x4C, 0x89, 0x4F, 0x08], // mov [rdi+8],r9
        &[0x48, 0x89, 0x4F, 0x10], // mov [rdi+10],rcx   this
        &[0x44, 0x89, 0x47, 0x18], // mov [rdi+18],r8d   tab
        &[0x48, 0x8B, 0x4A, 0x10], // mov rcx,[rdx+10]   wstring size
        &[0x89, 0x4F, 0x20], // mov [rdi+20],ecx   original length
        &[0x48, 0x81, 0xF9], // cmp rcx,MAX_CHARS
        &d(MAX_CHARS),
        &[0x76, 0x05], // jbe +5
        &[0xB9],       // mov ecx,MAX_CHARS
        &d(MAX_CHARS),
        &[0x89, 0x4F, 0x1C],                 // mov [rdi+1C],ecx   copied length
        &[0x48, 0x89, 0xD6],                 // mov rsi,rdx        SSO buffer
        &[0x48, 0x83, 0x7A, 0x18, 0x08],     // cmp qword [rdx+18],8  capacity
        &[0x72, 0x03],                       // jb +3
        &[0x48, 0x8B, 0x32],                 // mov rsi,[rdx]      heap buffer
        &[0x48, 0x83, 0xC7, DATA_OFF as u8], // add rdi,DATA_OFF
        &[0xFC],                             // cld
        &[0x66, 0xF3, 0xA5],                 // rep movsw
        &[0x48, 0xBF],                       // mov rdi,ring
        &q,
        &[0x48, 0x01, 0xC7],                                     // add rdi,rax
        &[0x4C, 0x89, 0x17], // mov [rdi],r10      publish seq last
        &[0x41, 0x5A, 0x41, 0x59, 0x5F, 0x5E, 0x59, 0x58, 0x9D], // pop r10,r9,rdi,rsi,rcx,rax; popfq
    ]
    .concat()
}

const ADD_LINE: SimpleHook = SimpleHook {
    pattern: PATTERN,
    instruction_length: 5,
    noops: 0,
    // wizwalker 預設 50 bytes 放不下這個 stub
    stub_size: 256,
    export_size: (HEADER + SLOTS * SLOT_SIZE) as usize,
    // 不釋放 ring：還原跳轉點的瞬間，遊戲執行緒可能還在 stub 內寫入
    free_export: false,
};

/// 把每次 ChatBox::AddLine 呼叫的（序號, 回傳位址, this, 分頁, 內容）寫進環形緩衝。
pub struct AddLineHook {
    patch: JumpPatch,
    ring: u64,
}

impl AddLineHook {
    /// 掛上並登記到 cave（由 [`AutobotCave::uninstall`] 一併解除）。
    pub fn install(mem: &dyn ProcessMemory, cave: &mut AutobotCave) -> Result<Self, HookError> {
        let (patch, ring) = ADD_LINE.install(mem, cave, stub_bytes)?;
        Ok(Self { patch, ring })
    }

    /// 環形緩衝的位址。
    pub fn ring_addr(&self) -> u64 {
        self.ring
    }

    /// 單獨解除：只寫回原碼，環形緩衝刻意不釋放。
    pub fn uninstall(&self, mem: &dyn ProcessMemory) -> Result<(), MemError> {
        self.patch.undo(mem)
    }
}
