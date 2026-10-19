//! 移植 Python `keyboard` 套件（Windows）的按鍵名稱表與快捷鍵解析，讓設定裡的快捷鍵字串對到相同的 scan code。
//!
//! 名稱表依本機鍵盤配置在第一次查詢時建立，與 Python 版同一套 Win32 呼叫與優先順序。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use regex::Regex;
use serde::Deserialize;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyNameTextW, MAPVK_VK_TO_CHAR, MAPVK_VK_TO_VSC_EX, MAPVK_VSC_TO_VK_EX, MapVirtualKeyExW,
    MapVirtualKeyW, ToUnicode,
};

use crate::py::pyrepr::repr_str;
use crate::util::lock;

/// 與 `keyboard` 事件的 `scan_code` 相同：原始 scan code，scan code 為 0 時是 `-vk`。
pub type Code = i32;
/// 一顆鍵的所有替代 code，順序與 `key_to_scan_codes` 相同。
pub type Key = Vec<Code>;

#[derive(Deserialize)]
struct Tables {
    canonical_names: HashMap<String, String>,
    official_virtual_keys: HashMap<u32, (String, bool)>,
    keypad_keys: Vec<(u32, u32, u32)>,
    distinct_modifiers: Vec<Vec<String>>,
    sided_modifiers: Vec<String>,
    all_modifiers: Vec<String>,
}

#[derive(Deserialize)]
struct Fixture {
    output: Tables,
}

static TABLES: LazyLock<Tables> = LazyLock::new(|| {
    let mut fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("../../tests/golden/keyboard_tables.json"))
            .expect("valid keyboard_tables.json");
    fixtures.remove(0).output
});

struct Names {
    /// 正規化名稱 → 依 Python `order_key` 排序、去重後的 code。
    from_name: HashMap<String, Key>,
    scan_code_to_vk: HashMap<u32, u32>,
}

static NAMES: LazyLock<Names> = LazyLock::new(setup_name_tables);

// `re.split(r',\s?')` 與 `re.split(r'\s?\+\s?')`；Python 的 `\s` 比 Rust 多 `\x1c`–`\x1f`。
static STEP_SEP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r",[\s\x1c-\x1f]?").expect("valid regex"));
static KEY_SEP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\s\x1c-\x1f]?\+[\s\x1c-\x1f]?").expect("valid regex"));

static MODIFIER_CODES: LazyLock<HashSet<Code>> = LazyLock::new(|| {
    TABLES
        .all_modifiers
        .iter()
        .flat_map(|name| key_codes_or_empty(name))
        .collect()
});

/// `_canonical_names.normalize_name`：清理名稱並換成標準寫法（如 `"LEFT CONTROL"` → `"left ctrl"`）。
pub fn normalize_name(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err(format!(
            "Can only normalize non-empty string names. Unexpected {}",
            repr_str(name)
        ));
    }
    let mut name = if name.chars().count() > 1 {
        name.to_lowercase()
    } else {
        name.to_string()
    };
    if name != "_" && name.contains('_') {
        name = name.replace('_', " ");
    }
    Ok(TABLES.canonical_names.get(&name).cloned().unwrap_or(name))
}

/// `_winkeyboard.get_event_names` 的第一個名稱（經 `normalize_name`）；modifiers 是 `shift`、`alt gr`、`caps lock` 等。
pub fn event_name(scan: u32, vk: u32, extended: bool, modifiers: &[&str]) -> Option<String> {
    let mods: Vec<String> = modifiers.iter().map(|m| (*m).to_string()).collect();
    let names = lock(&BUFFERS).event_names(scan, vk, u32::from(extended), &mods);
    let first = names.first()?;
    normalize_name(first).ok()
}

/// `keyboard.get_hotkey_name(names)`：去掉左右前綴、`+` 換成 `plus`，修飾鍵依 ctrl、alt、shift、windows 排前面。
pub fn hotkey_name(names: &[String]) -> String {
    const ORDER: [&str; 4] = ["ctrl", "alt", "shift", "windows"];
    let clean: BTreeSet<String> = names
        .iter()
        .filter_map(|name| normalize_name(name).ok())
        .map(|name| {
            name.replace("left ", "")
                .replace("right ", "")
                .replace('+', "plus")
        })
        .collect();
    let mut sorted: Vec<String> = clean.into_iter().collect();
    sorted.sort_by_key(|k| ORDER.iter().position(|m| m == k).unwrap_or(5));
    sorted.join("+")
}

/// `keyboard.key_to_scan_codes`：名稱對應的所有 code；對不到任何鍵時回錯誤。
pub fn key_to_scan_codes(name: &str) -> Result<Key, String> {
    scan_codes(name, true)
}

/// `key_to_scan_codes(name, error_if_missing=False)`：對不到時回空清單。
pub fn key_codes_or_empty(name: &str) -> Key {
    scan_codes(name, false).unwrap_or_default()
}

fn scan_codes(key: &str, error_if_missing: bool) -> Result<Key, String> {
    let normalized = normalize_name(key)?;
    if TABLES.sided_modifiers.contains(&normalized) {
        let mut codes = scan_codes(&format!("left {normalized}"), false)?;
        for code in scan_codes(&format!("right {normalized}"), false)? {
            if !codes.contains(&code) {
                codes.push(code);
            }
        }
        return Ok(codes);
    }
    let codes = NAMES
        .from_name
        .get(&normalized)
        .cloned()
        .unwrap_or_default();
    if codes.is_empty() && error_if_missing {
        return Err(format!(
            "Key {} is not mapped to any known key.",
            repr_str(key)
        ));
    }
    Ok(codes)
}

/// `keyboard.parse_hotkey`：回傳 steps → keys → codes（如 `"ctrl+a, s"` 是兩個 step）。
pub fn parse_hotkey(hotkey: &str) -> Result<Vec<Vec<Key>>, String> {
    if hotkey.chars().count() == 1 {
        return Ok(vec![vec![key_to_scan_codes(hotkey)?]]);
    }
    STEP_SEP
        .split(hotkey)
        .map(|step| KEY_SEP.split(step).map(key_to_scan_codes).collect())
        .collect()
}

/// `keyboard.is_modifier`：code 是否屬於任一修飾鍵。
pub fn is_modifier(code: Code) -> bool {
    MODIFIER_CODES.contains(&code)
}

/// `_winkeyboard` 注入按鍵時用的 scan code → vk（`code` 不是正數時直接是 `-vk`）。
pub fn scan_code_to_vk(code: Code) -> u16 {
    if code > 0 {
        NAMES
            .scan_code_to_vk
            .get(&(code as u32))
            .copied()
            .unwrap_or(0) as u16
    } else {
        (-code) as u16
    }
}

fn setup_name_tables() -> Names {
    let modifiers = &TABLES.distinct_modifiers;
    let mut buffers = lock(&BUFFERS);
    let mut from_name: HashMap<String, Vec<(usize, Entry)>> = HashMap::new();
    let mut scan_code_to_vk = HashMap::new();
    // SAFETY：純查表呼叫，HKL 傳 NULL 與 Python 版相同。
    let map_ex = |code, kind| unsafe { MapVirtualKeyExW(code, kind, std::ptr::null_mut()) };
    let all_scan_codes = (0..0x100).map(|sc| (sc, map_ex(sc, MAPVK_VSC_TO_VK_EX)));
    let all_vks = (0..0x100).map(|vk| (map_ex(vk, MAPVK_VK_TO_VSC_EX), vk));
    for (scan_code, vk) in all_scan_codes.chain(all_vks) {
        scan_code_to_vk.entry(scan_code).or_insert(vk);
        for extended in [0, 1] {
            for (m, mods) in modifiers.iter().enumerate() {
                let names = buffers.event_names(scan_code, vk, extended, mods);
                let lowercase: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
                for (i, name) in names.iter().chain(&lowercase).enumerate() {
                    let name = normalize_name(name).expect("event names are non-empty");
                    from_name
                        .entry(name)
                        .or_default()
                        .push((i, (scan_code, vk, extended, m)));
                }
            }
        }
    }
    // alt gr 在 Windows 上常被命名成 ctrl，Python 版手動補上這組。
    for extended in [0, 1] {
        for m in 0..modifiers.len() {
            from_name
                .entry("alt gr".to_string())
                .or_default()
                .push((1, (541, 162, extended, m)));
        }
    }
    let preference = |m: usize| match modifiers[m].as_slice() {
        [] => 0,
        [one] if one == "shift" => 1,
        [one] if one == "alt gr" => 2,
        _ => 10,
    };
    let from_name = from_name
        .into_iter()
        .map(|(name, mut entries)| {
            entries.sort_by_key(|&(i, (sc, vk, ext, m))| (preference(m), i, ext, vk, sc, m));
            let mut codes = Key::new();
            for (_, (sc, vk, _, _)) in entries {
                let code = if sc != 0 { sc as Code } else { -(vk as Code) };
                if !codes.contains(&code) {
                    codes.push(code);
                }
            }
            (name, codes)
        })
        .collect();
    Names {
        from_name,
        scan_code_to_vk,
    }
}

/// （scan code、vk、extended、`distinct_modifiers` 的索引）
type Entry = (u32, u32, u32, usize);

/// Python 版的全域緩衝區：跨呼叫保留內容，ToUnicode 不一定補 NUL，殘留字元也要一致。
struct Buffers {
    state: [u8; 256],
    unicode: [u16; 32],
    name: [u16; 32],
}

static BUFFERS: Mutex<Buffers> = Mutex::new(Buffers {
    state: [0; 256],
    unicode: [0; 32],
    name: [0; 32],
});

impl Buffers {
    /// `_winkeyboard.get_event_names`：依優先順序列出這組按鍵狀態的名稱。
    fn event_names(
        &mut self,
        scan_code: u32,
        vk: u32,
        is_extended: u32,
        mods: &[String],
    ) -> Vec<String> {
        let mut names = Vec::new();
        let is_keypad = TABLES.keypad_keys.contains(&(scan_code, vk, is_extended));
        let official = TABLES.official_virtual_keys.get(&vk).map(|(name, _)| name);
        if is_keypad && let Some(name) = official {
            names.push(name.clone());
        }

        let has = |m: &str| mods.iter().any(|x| x == m);
        self.state[0x10] = if has("shift") { 0x80 } else { 0 };
        self.state[0x11] = if has("alt gr") { 0x80 } else { 0 };
        self.state[0x12] = if has("alt gr") { 0x80 } else { 0 };
        self.state[0x14] = u8::from(has("caps lock"));
        self.state[0x90] = u8::from(has("num lock"));
        self.state[0x91] = u8::from(has("scroll lock"));
        let len = self.unicode.len() as i32;
        // SAFETY：緩衝區長度與傳入的 cchbuff 一致。
        let to_unicode = |b: &mut Self| unsafe {
            ToUnicode(
                vk,
                scan_code,
                b.state.as_ptr(),
                b.unicode.as_mut_ptr(),
                len,
                0,
            )
        };
        let ret = to_unicode(self);
        let value = until_nul(&self.unicode);
        if ret != 0 && !value.is_empty() {
            names.push(value);
            // ToUnicode 遇到 dead key 會改全域狀態，再呼叫一次清掉。
            to_unicode(self);
        }

        let lparam = ((i64::from(scan_code) << 16) | (i64::from(is_extended) << 24)) as i32;
        // SAFETY：緩衝區長度與傳入的 cchsize 一致。
        let ret =
            unsafe { GetKeyNameTextW(lparam, self.name.as_mut_ptr(), self.name.len() as i32) };
        let value = until_nul(&self.name);
        if ret != 0 && !value.is_empty() {
            names.push(value);
        }

        // SAFETY：純查表呼叫。
        let ch = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_CHAR) } & 0xFF;
        if ch != 0 {
            names.push(char::from(ch as u8).to_string());
        }

        if !is_keypad && let Some(name) = official {
            names.push(name.clone());
        }
        names
    }
}

fn until_nul(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// 測試用：目前鍵盤配置是否與 fixture 相同；不同時印出略過 `test` 的原因並回 `false`。
///
/// 名稱表取自目前配置（GetKeyNameText、ToUnicode 等），換了配置名稱與 scan code 都會變。
#[cfg(any(test, feature = "test-support"))]
pub fn on_fixture_layout(test: &str) -> bool {
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayoutNameW;
    use windows_sys::Win32::UI::WindowsAndMessaging::KL_NAMELENGTH;
    // fixture 錄製時的配置檔：美式鍵盤，繁中、簡中等輸入法的預設配置也是它
    const FIXTURE_LAYOUT_FILE: &str = "KBDUS.DLL";

    let mut klid = [0u16; KL_NAMELENGTH as usize];
    // SAFETY：緩衝區長度為 KL_NAMELENGTH。
    unsafe { GetKeyboardLayoutNameW(klid.as_mut_ptr()) };
    let klid = until_nul(&klid);
    let key: Vec<u16> = format!(r"SYSTEM\CurrentControlSet\Control\Keyboard Layouts\{klid}")
        .encode_utf16()
        .chain([0])
        .collect();
    let value: Vec<u16> = "Layout File".encode_utf16().chain([0]).collect();
    let mut file = [0u16; 64];
    let mut size = std::mem::size_of_val(&file) as u32;
    // SAFETY：字串皆以 NUL 結尾，size 是 file 的位元組數。
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            file.as_mut_ptr().cast(),
            &mut size,
        )
    };
    let file = until_nul(&file);
    if file.eq_ignore_ascii_case(FIXTURE_LAYOUT_FILE) {
        return true;
    }
    eprintln!(
        "[test] {test} skipped: key-name fixtures were recorded on {FIXTURE_LAYOUT_FILE}, \
         active layout is {klid} ({file:?})"
    );
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_table_has_no_name_python_does_not_know() {
        if !on_fixture_layout("the_name_table_has_no_name_python_does_not_know") {
            return;
        }
        let python: std::collections::HashSet<String> =
            serde_json::from_str::<Vec<serde_json::Value>>(include_str!(
                "../../tests/golden/keyboard_names.json"
            ))
            .unwrap()
            .iter()
            .map(|case| case["input"].as_str().unwrap().to_owned())
            .collect();
        let extra: Vec<&String> = NAMES
            .from_name
            .keys()
            .filter(|name| !python.contains(*name))
            .collect();
        assert!(extra.is_empty(), "names only Rust accepts: {extra:?}");
    }
}
