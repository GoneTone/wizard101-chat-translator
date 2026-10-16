//! 遊戲 UI 樹：以記憶體位址走訪 Window 控件（子節點、名稱、父節點、旗標、矩形）。
//!
//! 偏移與 wizwalker `Window` 相同；讀取錯誤不丟出，走訪時直接略過壞節點。

use std::collections::HashSet;

use super::memory::{MemError, ProcessMemory};

const CHILDREN_OFFSET: u64 = 0x70;
const NAME_OFFSET: u64 = 0x50;
const PARENT_OFFSET: u64 = 0x88;
const FLAGS_OFFSET: u64 = 0x9C;
const RECT_OFFSET: u64 = 0xA0;
/// 子節點數的上限；超過視為讀到垃圾。
const MAX_CHILDREN: i128 = 1000;
/// `shared_ptr` 的大小，子節點向量的元素。
const SHARED_PTR_SIZE: i128 = 16;

/// 控件的 `VISIBLE` 旗標。
pub const FLAG_VISIBLE: u32 = 0x1;

/// 控件的矩形（相對於父控件的邏輯座標，x2／y2 為含端點的邊）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

/// 遊戲記憶體中的一個 Window 控件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Window {
    pub addr: u64,
}

impl Window {
    /// 子節點；向量大小為 0、為負、超過上限或內容讀不到時回空（同 wizwalker）。
    pub fn children(&self, mem: &dyn ProcessMemory) -> Result<Vec<Window>, MemError> {
        let start = mem.read_u64(self.addr + CHILDREN_OFFSET)? as i64;
        let end = mem.read_u64(self.addr + CHILDREN_OFFSET + 8)? as i64;
        let size = i128::from(end) - i128::from(start);
        if size <= 0 || size / SHARED_PTR_SIZE > MAX_CHILDREN {
            return Ok(Vec::new());
        }
        let Ok(raw) = mem.read(start as u64, size as usize) else {
            return Ok(Vec::new());
        };
        Ok(raw
            .chunks_exact(SHARED_PTR_SIZE as usize)
            .map(|c| u64::from_le_bytes(c[..8].try_into().expect("chunk has 16 bytes")))
            .filter(|&addr| addr != 0)
            .map(|addr| Window { addr })
            .collect())
    }

    /// 控件名稱。
    pub fn name(&self, mem: &dyn ProcessMemory) -> Result<String, MemError> {
        mem.read_std_string(self.addr + NAME_OFFSET)
    }

    /// 父控件；根視窗沒有父控件。
    pub fn parent(&self, mem: &dyn ProcessMemory) -> Result<Option<Window>, MemError> {
        let addr = mem.read_u64(self.addr + PARENT_OFFSET)?;
        Ok((addr != 0).then_some(Window { addr }))
    }

    /// 旗標位元（`FLAG_VISIBLE` 等）。
    pub fn flags(&self, mem: &dyn ProcessMemory) -> Result<u32, MemError> {
        mem.read_u32(self.addr + FLAGS_OFFSET)
    }

    /// 相對於父控件的矩形。
    pub fn rect(&self, mem: &dyn ProcessMemory) -> Result<Rect, MemError> {
        let at = self.addr + RECT_OFFSET;
        Ok(Rect {
            x1: mem.read_i32(at)?,
            y1: mem.read_i32(at + 4)?,
            x2: mem.read_i32(at + 8)?,
            y2: mem.read_i32(at + 12)?,
        })
    }
}

/// 在 `root` 底下找所有名稱相符的後代：先列直屬子節點中的符合者，再逐一深入；讀不到的節點略過。
pub fn find_by_name(mem: &dyn ProcessMemory, root: Window, name: &str) -> Vec<Window> {
    let mut found = Vec::new();
    // 已走過的節點：遊戲記憶體讀到壞資料時，環狀參照不致讓遞迴爆堆疊
    let mut seen = HashSet::from([root]);
    let children = root.children(mem).unwrap_or_default();
    for &child in &children {
        if child.name(mem).is_ok_and(|n| n == name) {
            found.push(child);
        }
    }
    for child in children {
        descend(mem, child, name, &mut seen, &mut found);
    }
    found
}

fn descend(
    mem: &dyn ProcessMemory,
    node: Window,
    name: &str,
    seen: &mut HashSet<Window>,
    found: &mut Vec<Window>,
) {
    if !seen.insert(node) {
        return;
    }
    for child in node.children(mem).unwrap_or_default() {
        if child.name(mem).is_ok_and(|n| n == name) {
            found.push(child);
        }
        descend(mem, child, name, seen, found);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::memory::FakeMemory;

    fn put_name(mem: &FakeMemory, win: u64, name: &str) {
        let mut head = [0u8; 0x20];
        head[..name.len()].copy_from_slice(name.as_bytes());
        head[16..20].copy_from_slice(&(name.len() as i32).to_le_bytes());
        mem.set_bytes(win + NAME_OFFSET, &head);
    }

    fn put_children(mem: &FakeMemory, win: u64, vec_at: u64, kids: &[u64]) {
        let raw: Vec<u8> = kids
            .iter()
            .flat_map(|k| [k.to_le_bytes(), [0; 8]].concat())
            .collect();
        mem.set_bytes(vec_at, &raw);
        let mut header = vec_at.to_le_bytes().to_vec();
        header.extend((vec_at + raw.len() as u64).to_le_bytes());
        mem.set_bytes(win + CHILDREN_OFFSET, &header);
    }

    #[test]
    fn find_by_name_lists_direct_matches_first_then_descends() {
        let mem = FakeMemory::new();
        put_children(&mem, 0x1000, 0x9000, &[0x2000, 0x3000]);
        put_name(&mem, 0x2000, "Box");
        put_name(&mem, 0x3000, "Target");
        put_children(&mem, 0x2000, 0x9100, &[0x4000]);
        put_name(&mem, 0x4000, "Target");
        put_children(&mem, 0x3000, 0x9200, &[]);
        put_children(&mem, 0x4000, 0x9300, &[]);
        let found = find_by_name(&mem, Window { addr: 0x1000 }, "Target");
        let addrs: Vec<u64> = found.iter().map(|w| w.addr).collect();
        assert_eq!(addrs, [0x3000, 0x4000]);
    }

    #[test]
    fn children_returns_empty_for_negative_oversize_and_zero_pointers() {
        let mem = FakeMemory::new();
        mem.set_bytes(
            0x1000 + CHILDREN_OFFSET,
            &[&0x9000u64.to_le_bytes()[..], &0x8000u64.to_le_bytes()].concat(),
        );
        assert!(Window { addr: 0x1000 }.children(&mem).unwrap().is_empty());
        let huge = 0x9000u64 + 16 * 1001;
        mem.set_bytes(
            0x2000 + CHILDREN_OFFSET,
            &[&0x9000u64.to_le_bytes()[..], &huge.to_le_bytes()].concat(),
        );
        assert!(Window { addr: 0x2000 }.children(&mem).unwrap().is_empty());
        put_children(&mem, 0x3000, 0x9100, &[0, 0x5000]);
        assert_eq!(
            Window { addr: 0x3000 }.children(&mem).unwrap(),
            [Window { addr: 0x5000 }]
        );
    }
}
