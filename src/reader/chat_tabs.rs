//! 收訊端：讀遊戲聊天框的分頁狀態（目前分頁編號、各分頁按鈕上的文字）。

use std::fmt;

use super::ui_tree::{FLAG_VISIBLE, Window, find_by_name};
use crate::game::memory::{MemError, ProcessMemory, decode_utf16le};

/// 聊天框控件的名稱。
pub const CHATBOX_WINDOW: &str = "WizardChatBox";
/// 聊天框內「目前分頁編號」的偏移。
pub const CUR_TAB_OFFSET: u64 = 0x7A0;
/// 分頁按鈕內文字（`std::wstring`）的偏移。
pub const LABEL_OFFSET: u64 = 0x2E0;
/// wchar_t 內嵌最多 7 字；capacity 達 8 時 +0 是堆積指標。
pub const SSO_CAPACITY: u64 = 8;
/// 按鈕文字的長度上限；超過視為讀到垃圾。
pub const MAX_LABEL_CHARS: u64 = 4096;

/// 一則廣播每個分頁各寫一筆，筆數上限即分頁數。
pub const MAX_BROADCAST_RECORDS: usize = TAB_BUTTONS.len();

/// 依遊戲由左到右；編號 0–3 已實機確認，4、5–8 是靜態推論。
const TAB_BUTTONS: [(&str, i32); 9] = [
    ("MainFilterButton", 0),
    ("GroupFilterButton", 1),
    ("HouseFilterButton", 2),
    ("FriendFilterButton", 3),
    ("Channel1FilterButton", 5),
    ("Channel2FilterButton", 6),
    ("Channel3FilterButton", 7),
    ("Channel4FilterButton", 8),
    ("TeamUpFilterButton", 4),
];

/// 聊天框的分頁狀態；`tabs` 是目前可見分頁的（編號, 按鈕文字），依 `TAB_BUTTONS` 順序。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabState {
    pub tabs: Vec<(i32, String)>,
    pub current: i32,
}

/// 隱藏分頁的按鈕文字是未解析的 `<string;...>` token，回 None；其餘原樣回傳。
pub fn visible_label(raw: &str) -> Option<String> {
    (!raw.is_empty() && !raw.starts_with("<string;")).then(|| raw.to_owned())
}

/// 讀 MSVC `std::wstring`；size 為 0、超過上限或大於 capacity（讀到垃圾）回空字串。
fn read_label(mem: &dyn ProcessMemory, addr: u64) -> Result<String, MemError> {
    let head = mem.read(addr, 0x20)?;
    let word = |at: usize| u64::from_le_bytes(head[at..at + 8].try_into().expect("8 bytes"));
    let (size, capacity) = (word(0x10), word(0x18));
    if size == 0 || size > MAX_LABEL_CHARS.min(capacity) {
        return Ok(String::new());
    }
    let len = size as usize * 2;
    let raw = if capacity >= SSO_CAPACITY {
        mem.read(word(0), len)?
    } else {
        head[..len].to_vec()
    };
    Ok(decode_utf16le(&raw))
}

fn is_tab_id(tab: i32) -> bool {
    TAB_BUTTONS.iter().any(|&(_, id)| id == tab)
}

/// 只讀目前分頁編號（一次記憶體讀取）；讀不到或超出範圍回 None。
pub fn read_current_tab(mem: &dyn ProcessMemory, chatbox: u64) -> Option<i32> {
    let current = mem.read_i32(chatbox.wrapping_add(CUR_TAB_OFFSET)).ok()?;
    is_tab_id(current).then_some(current)
}

/// 按鈕與外層容器都帶 VISIBLE：顯示過又被隱藏的分頁（如解散隊伍）文字不會變回 token。
fn button_shown(mem: &dyn ProcessMemory, button: Window) -> Result<bool, MemError> {
    let mut windows = vec![button];
    windows.extend(button.parent(mem)?);
    for window in windows {
        if window.flags(mem)? & FLAG_VISIBLE == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

/// 讀分頁狀態失敗的原因；`Display` 直接寫進 log。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabError {
    /// 目前分頁編號不在已知範圍（ChatBox 已失效）。
    OutOfRange { current: i32, chatbox: u64 },
    /// 記憶體讀取失敗。
    Mem(MemError),
}

impl fmt::Display for TabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { current, chatbox } => write!(
                f,
                "current tab {current} out of range (stale chat box {chatbox:#x}?)"
            ),
            Self::Mem(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for TabError {}

impl From<MemError> for TabError {
    fn from(e: MemError) -> Self {
        Self::Mem(e)
    }
}

/// 從 root window 找各分頁按鈕（依 `TAB_BUTTONS` 順序，找不到的略過）；結果由呼叫端快取。
pub fn find_tab_buttons(
    mem: &dyn ProcessMemory,
    root: Window,
) -> Result<Vec<(i32, Window)>, MemError> {
    let mut found = Vec::new();
    for (name, tab) in TAB_BUTTONS {
        if let Some(&node) = find_by_name(mem, root, name)?.first() {
            found.push((tab, node));
        }
    }
    Ok(found)
}

/// 聊天框的目前分頁與可見分頁；目前分頁超出範圍（ChatBox 已失效）或讀取失敗回 `TabError`。
pub fn read_tab_state(
    mem: &dyn ProcessMemory,
    chatbox: u64,
    buttons: &[(i32, Window)],
) -> Result<TabState, TabError> {
    let current = mem.read_i32(chatbox.wrapping_add(CUR_TAB_OFFSET))?;
    if !is_tab_id(current) {
        return Err(TabError::OutOfRange { current, chatbox });
    }
    let mut tabs = Vec::new();
    for &(tab, button) in buttons {
        if !button_shown(mem, button)? {
            continue;
        }
        if let Some(label) =
            visible_label(&read_label(mem, button.addr.wrapping_add(LABEL_OFFSET))?)
        {
            tabs.push((tab, label));
        }
    }
    Ok(TabState { tabs, current })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::memory::FakeMemory;

    fn put_label(mem: &FakeMemory, at: u64, text: &str, capacity: u64, size: Option<u64>) {
        let data: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut head = vec![0u8; 0x20];
        if capacity >= SSO_CAPACITY {
            head[..8].copy_from_slice(&0x9000u64.to_le_bytes());
            mem.set_bytes(0x9000, &data);
        } else {
            head[..data.len()].copy_from_slice(&data);
        }
        let size = size.unwrap_or(text.encode_utf16().count() as u64);
        head[0x10..0x18].copy_from_slice(&size.to_le_bytes());
        head[0x18..].copy_from_slice(&capacity.to_le_bytes());
        mem.set_bytes(at, &head);
    }

    fn label(text: &str, capacity: u64, size: Option<u64>) -> String {
        let mem = FakeMemory::new();
        put_label(&mem, 0x1000, text, capacity, size);
        read_label(&mem, 0x1000).unwrap()
    }

    #[test]
    fn read_label_inline_heap_and_rejected_sizes() {
        assert_eq!(label("主要的", 7, None), "主要的");
        assert_eq!(label("队伍(2)", 7, None), "队伍(2)");
        assert_eq!(label("Party 1 (3)", 15, None), "Party 1 (3)");
        assert_eq!(label("", 7, None), "");
        assert_eq!(label("abc", 7, Some(10_000_000)), "");
        assert_eq!(label("abc", 7, Some(8)), "");
        assert_eq!(label("abc", 2, Some(3)), "");
        assert_eq!(label("abcdefg", 7, None), "abcdefg");
        assert_eq!(label("abc", 8, None), "abc");
        let long = "a".repeat(4096);
        assert_eq!(label(&long, 4096, None), long);
        assert_eq!(label(&"a".repeat(4097), 4097, None), "");
    }

    #[test]
    fn tab_buttons_cover_the_verified_mapping() {
        let id = |name| TAB_BUTTONS.iter().find(|b| b.0 == name).map(|b| b.1);
        assert_eq!(id("MainFilterButton"), Some(0));
        assert_eq!(id("GroupFilterButton"), Some(1));
        assert_eq!(id("HouseFilterButton"), Some(2));
        let names: std::collections::HashSet<_> = TAB_BUTTONS.iter().map(|b| b.0).collect();
        let ids: std::collections::HashSet<_> = TAB_BUTTONS.iter().map(|b| b.1).collect();
        assert_eq!((names.len(), ids.len()), (9, 9));
    }

    #[test]
    fn current_tab_must_be_a_known_id() {
        let mem = FakeMemory::new();
        mem.set_bytes(0x1000 + CUR_TAB_OFFSET, &4i32.to_le_bytes());
        assert_eq!(read_current_tab(&mem, 0x1000), Some(4));
        mem.set_bytes(0x1000 + CUR_TAB_OFFSET, &9i32.to_le_bytes());
        assert_eq!(read_current_tab(&mem, 0x1000), None);
        assert_eq!(read_current_tab(&mem, 0x2000), None);
    }
}
