//! 在 `FakeMemory` 裡擺出遊戲的樣子：pattern 命中與 UI 樹控件。
#![allow(dead_code)]

use windows_sys::Win32::System::Memory::{MEM_COMMIT, PAGE_EXECUTE_READ};
use wizard101_chat_translator::reader::chat_hook::{DATA_OFF, HEADER, SLOT_SIZE, SLOTS};
use wizard101_chat_translator::reader::chat_tabs::LABEL_OFFSET;
use wizard101_chat_translator::reader::hooks::MODULE;
use wizard101_chat_translator::reader::memory::{FakeMemory, ProcessMemory, Region};

/// 依 pattern 原文造一段命中位元組（同 golden exporter 的 `_pattern_instance`）。
pub fn pattern_instance(pattern: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = pattern;
    while !rest.is_empty() {
        if let Some(hex) = rest.strip_prefix(r"\x") {
            out.push(u8::from_str_radix(&hex[..2], 16).unwrap());
            rest = &hex[2..];
        } else {
            assert!(rest.starts_with('.'), "unsupported token in {pattern}");
            out.push(0x0A);
            rest = &rest[1..];
        }
    }
    out
}

/// 在模組裡加一個可掃描區段並放入 pattern 命中，回傳整段內容。
pub fn place(mem: &FakeMemory, addr: u64, pattern: &str, size: usize) -> Vec<u8> {
    let mut bytes = pattern_instance(pattern);
    bytes.resize(size, 0xCC);
    mem.add_region(
        MODULE,
        Region {
            base: addr,
            size,
            protect: PAGE_EXECUTE_READ,
            state: MEM_COMMIT,
        },
    );
    mem.set_bytes(addr, &bytes);
    bytes
}

/// 控件名稱（16 bytes 以上放在 `win + 0x300_000`）。
pub fn put_name(mem: &FakeMemory, win: u64, name: &str) {
    let mut head = [0u8; 0x20];
    if name.len() >= 16 {
        let heap = win + 0x300_000;
        mem.set_bytes(heap, name.as_bytes());
        head[..8].copy_from_slice(&heap.to_le_bytes());
    } else {
        head[..name.len()].copy_from_slice(name.as_bytes());
    }
    head[16..20].copy_from_slice(&(name.len() as i32).to_le_bytes());
    mem.set_bytes(win + 0x50, &head);
}

/// 子節點向量（放在 `win + 0x100_000`）。
pub fn put_children(mem: &FakeMemory, win: u64, kids: &[u64]) {
    let vec_at = win + 0x100_000;
    let raw: Vec<u8> = kids
        .iter()
        .flat_map(|k| [k.to_le_bytes(), [0; 8]].concat())
        .collect();
    if !raw.is_empty() {
        mem.set_bytes(vec_at, &raw);
    }
    let header = [
        vec_at.to_le_bytes(),
        (vec_at + raw.len() as u64).to_le_bytes(),
    ]
    .concat();
    mem.set_bytes(win + 0x70, &header);
}

/// 分頁按鈕：名稱、旗標、外層容器（`win + 0x200_000`）旗標與 SSO 按鈕文字（最多 7 字）。
pub fn put_button(
    mem: &FakeMemory,
    win: u64,
    name: &str,
    label: &str,
    flags: u32,
    parent_flags: u32,
) {
    put_name(mem, win, name);
    put_children(mem, win, &[]);
    let parent = win + 0x200_000;
    mem.set_bytes(win + 0x88, &parent.to_le_bytes());
    mem.set_bytes(win + 0x9C, &flags.to_le_bytes());
    mem.set_bytes(parent + 0x9C, &parent_flags.to_le_bytes());
    let data: Vec<u8> = label.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut head = vec![0u8; 0x20];
    head[..data.len()].copy_from_slice(&data);
    head[0x10..0x18].copy_from_slice(&(label.encode_utf16().count() as u64).to_le_bytes());
    head[0x18..].copy_from_slice(&7u64.to_le_bytes());
    mem.set_bytes(win + LABEL_OFFSET, &head);
}

/// 環形緩衝第 `seq` 筆所在的槽位位址。
pub fn slot_addr(ring: u64, seq: u64) -> u64 {
    ring + HEADER + ((seq - 1) % SLOTS) * SLOT_SIZE
}

/// 照 stub 的格式寫一個槽位並把累計數推到至少 `seq`；`slot_seq` 可偽造槽內序號。
pub fn put_slot(
    mem: &FakeMemory,
    ring: u64,
    seq: u64,
    units: &[u16],
    tab: u32,
    this: u64,
    slot_seq: Option<u64>,
) {
    let at = slot_addr(ring, seq);
    let mut head = Vec::new();
    head.extend(slot_seq.unwrap_or(seq).to_le_bytes());
    head.extend(0x1407443B1u64.to_le_bytes());
    head.extend(this.to_le_bytes());
    head.extend(tab.to_le_bytes());
    head.extend((units.len() as u32).to_le_bytes());
    head.extend((units.len() as u32).to_le_bytes());
    head.resize(DATA_OFF as usize, 0);
    mem.set_bytes(at, &head);
    let data: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    if !data.is_empty() {
        mem.set_bytes(at + DATA_OFF, &data);
    }
    let count = mem.read_u64(ring).unwrap_or(0).max(seq);
    mem.set_bytes(ring, &count.to_le_bytes());
}
