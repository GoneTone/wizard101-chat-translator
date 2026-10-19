//! 收訊端：hook 遊戲的 ChatBox::AddLine，把每次呼叫寫進環形緩衝。
//!
//! stub 在遊戲執行緒上執行，位元組與 Python 版 `chat_hook.stub_bytes` 逐一相同。

use super::chat_tabs::MAX_BROADCAST_RECORDS;
use super::hooks::{AutobotCave, HookError, SimpleHook};
use super::memory::{MemError, ProcessMemory, decode_utf16le};

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
    jump: u64,
    ring: u64,
}

impl AddLineHook {
    /// 掛上並登記到 cave（由 [`AutobotCave::uninstall_with`] 解除，環形緩衝刻意不釋放）。
    pub fn install(mem: &dyn ProcessMemory, cave: &mut AutobotCave) -> Result<Self, HookError> {
        let (jump, ring) = ADD_LINE.install(mem, cave, stub_bytes)?;
        Ok(Self { jump, ring })
    }

    /// 被改成跳轉的 `ChatBox::AddLine` 開頭位址。
    pub fn jump_addr(&self) -> u64 {
        self.jump
    }

    /// 環形緩衝的位址。
    pub fn ring_addr(&self) -> u64 {
        self.ring
    }
}

/// 一次 AddLine 呼叫；`copied < length`（UTF-16 字元數）代表內容被截斷。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRecord {
    pub seq: u64,
    /// 呼叫端的回傳位址。
    pub ret: u64,
    pub this: u64,
    pub tab: u32,
    pub copied: u32,
    pub length: u32,
    pub text: String,
}

/// 一輪讀取結果；`lost` 是本輪確定讀不到（已被覆寫）的記錄數。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RingRead {
    pub records: Vec<HookRecord>,
    pub last_seq: u64,
    pub lost: u64,
}

/// 解碼一個槽位：`bytes` 是 0x30 bytes 標頭加其後的 UTF-16LE 內容（缺的部分視為 0 或空）。
pub fn decode_record(bytes: &[u8]) -> HookRecord {
    let mut head = [0u8; DATA_OFF as usize];
    let n = bytes.len().min(head.len());
    head[..n].copy_from_slice(&bytes[..n]);
    let u64_at = |at: usize| u64::from_le_bytes(head[at..at + 8].try_into().expect("8 bytes"));
    let u32_at = |at: usize| u32::from_le_bytes(head[at..at + 4].try_into().expect("4 bytes"));
    HookRecord {
        seq: u64_at(0),
        ret: u64_at(8),
        this: u64_at(0x10),
        tab: u32_at(0x18),
        copied: u32_at(0x1C),
        length: u32_at(0x20),
        text: decode_utf16le(bytes.get(DATA_OFF as usize..).unwrap_or_default()),
    }
}

/// 讀出序號 `last_seq` 之後的新記錄。
pub fn read_ring(
    mem: &dyn ProcessMemory,
    ring: u64,
    mut last_seq: u64,
) -> Result<RingRead, MemError> {
    let count = mem.read_u64(ring)?;
    let mut lost = 0;
    if count.saturating_sub(last_seq) > SLOTS {
        lost += count - last_seq - SLOTS;
        last_seq = count - SLOTS;
    }
    let mut records = Vec::new();
    while last_seq < count {
        let seq = last_seq + 1;
        let slot = ring + HEADER + ((seq - 1) % SLOTS) * SLOT_SIZE;
        let mut bytes = mem.read(slot, DATA_OFF as usize)?;
        let head = decode_record(&bytes);
        // 0＝寫入中；更舊的序號＝stub 已更新累計數、還沒清槽，都是下一輪再讀
        if head.seq < seq {
            break;
        }
        last_seq = seq;
        if head.seq > seq {
            lost += 1;
            continue;
        }
        // 讀到一半被覆寫的槽位 copied 可能是垃圾，先夾在 stub 的複製上限內
        let chars = u64::from(head.copied).min(MAX_CHARS);
        if chars > 0 {
            bytes.extend(mem.read(slot + DATA_OFF, chars as usize * 2)?);
        }
        if mem.read_u64(slot)? != seq {
            lost += 1;
            continue;
        }
        records.push(decode_record(&bytes));
    }
    Ok(RingRead {
        records,
        last_seq,
        lost,
    })
}

fn continues_broadcast(run: &[HookRecord], rec: &HookRecord) -> bool {
    let Some(prev) = run.last() else {
        return false;
    };
    if rec.seq != prev.seq + 1 || rec.text != prev.text {
        return false;
    }
    if run.len() < 4 {
        rec.tab as usize == run.len()
    } else {
        rec.tab > prev.tab.max(4)
    }
}

/// 把伺服器廣播（連號、同字、分頁 0,1,2,3 再接遞增的頻道分頁）收斂成一組，回傳（分頁組, 代表記錄）。
pub fn group_broadcasts(records: &[HookRecord]) -> Vec<(Vec<u32>, HookRecord)> {
    let mut groups = Vec::new();
    let mut i = 0;
    while i < records.len() {
        let mut run = 1;
        if records[i].tab == 0 {
            while i + run < records.len()
                && continues_broadcast(&records[i..i + run], &records[i + run])
            {
                run += 1;
            }
        }
        if run < 4 {
            run = 1;
        }
        let tabs = records[i..i + run].iter().map(|r| r.tab).collect();
        groups.push((tabs, records[i].clone()));
        i += run;
    }
    groups
}

/// 結尾若是廣播的開頭（連號、同字、分頁 0 起；含已湊滿 4 筆但還可能接頻道分頁者）回傳其筆數，否則 0。
pub fn pending_broadcast(records: &[HookRecord]) -> usize {
    for n in (2..=records.len().min(MAX_BROADCAST_RECORDS)).rev() {
        let tail = &records[records.len() - n..];
        if tail[0].tab == 0 && (1..n).all(|i| continues_broadcast(&tail[..i], &tail[i])) {
            return n;
        }
    }
    0
}
