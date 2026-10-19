//! 移植 Python `keyboard` 套件（Windows）的按鍵名稱表與快捷鍵解析，讓設定裡的快捷鍵字串對到相同的 scan code。
//!
//! 名稱表依本機鍵盤配置在第一次查詢時建立，與 Python 版同一套 Win32 呼叫與優先順序；
//! 字母、數字鍵另以虛擬鍵命名（`a`～`z`、`0`～`9`），非拉丁字母配置下熱鍵與 Ctrl+V 照樣對得到。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use regex::Regex;
use serde::Deserialize;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyNameTextW, GetKeyboardLayoutList, MAPVK_VK_TO_CHAR, MAPVK_VK_TO_VSC_EX,
    MAPVK_VSC_TO_VK_EX, MapVirtualKeyExW, MapVirtualKeyW, ToUnicode, VkKeyScanExW,
};

use crate::logs::log;
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

static NAMES: LazyLock<Names> = LazyLock::new(|| {
    // SAFETY：純查表呼叫，HKL 傳 NULL 與 Python 版相同。
    let map_ex = |code, kind| unsafe { MapVirtualKeyExW(code, kind, std::ptr::null_mut()) };
    let all_scan_codes = (0..0x100).map(|sc| (sc, map_ex(sc, MAPVK_VSC_TO_VK_EX)));
    let all_vks = (0..0x100).map(|vk| (map_ex(vk, MAPVK_VK_TO_VSC_EX), vk));
    let mut buffers = lock(&BUFFERS);
    build_names(all_scan_codes.chain(all_vks), |sc, vk, ext, mods| {
        buffers.event_names(sc, vk, ext, mods)
    })
});

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
    let first = names.into_iter().next()?;
    normalize_name(&layout_independent_name(vk, first)).ok()
}

/// 字母、數字鍵的虛擬鍵名稱（`a`～`z`、`0`～`9`），其他鍵回 `None`。
fn latin_key_name(vk: u32) -> Option<String> {
    matches!(vk, 0x30..=0x39 | 0x41..=0x5A)
        .then(|| char::from(vk as u8).to_ascii_lowercase().to_string())
}

/// 字母鍵打出的不是 ASCII 字母、數字鍵打出非 ASCII 字元（如西里爾字母）時改用虛擬鍵名稱。
///
/// 錄到的熱鍵因此換了鍵盤配置也解析得到；美式配置下字母鍵只打出 ASCII 字母，名稱不變。
fn layout_independent_name(vk: u32, name: String) -> String {
    let Some(latin) = latin_key_name(vk) else {
        return name;
    };
    let keep = if latin.as_bytes()[0].is_ascii_alphabetic() {
        name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic()
    } else {
        name.is_ascii()
    };
    if keep { name } else { latin }
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
    let codes = NAMES.codes(&normalized, vk_on_installed_layouts);
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

impl Names {
    /// 名稱對應的 code；對不到的單一字元交給 `vk_of` 查是哪顆字母／數字鍵。
    ///
    /// 舊版在非拉丁字母配置錄到的在地名稱（如 `ctrl+м`）在別的配置下才對得回原本那顆鍵。
    fn codes(&self, name: &str, vk_of: impl Fn(char) -> Option<u32>) -> Key {
        if let Some(codes) = self.from_name.get(name) {
            return codes.clone();
        }
        let mut chars = name.chars();
        let (Some(ch), None) = (chars.next(), chars.next()) else {
            return Key::new();
        };
        let Some(latin) = vk_of(ch).and_then(latin_key_name) else {
            return Key::new();
        };
        log(format!(
            "[hotkey] key name {} is not on the current keyboard layout; matched it as {}",
            repr_str(name),
            repr_str(&latin)
        ));
        self.from_name.get(&latin).cloned().unwrap_or_default()
    }
}

/// 使用者已載入的鍵盤配置中，打得出 `ch` 的字母／數字鍵的虛擬鍵；只查表，不切換配置。
fn vk_on_installed_layouts(ch: char) -> Option<u32> {
    let unit = u16::try_from(u32::from(ch)).ok()?;
    let mut layouts = [std::ptr::null_mut(); 64];
    // SAFETY：緩衝區長度與傳入的 nbuff 一致。
    let count = unsafe { GetKeyboardLayoutList(layouts.len() as i32, layouts.as_mut_ptr()) };
    layouts[..count.max(0) as usize].iter().find_map(|&hkl| {
        // SAFETY：純查表呼叫，hkl 來自 GetKeyboardLayoutList。
        let found = unsafe { VkKeyScanExW(unit, hkl) };
        let vk = u32::from(found as u16 & 0xFF);
        (found != -1 && latin_key_name(vk).is_some()).then_some(vk)
    })
}

/// 依 `keys`（scan code、vk）與 `event_names` 建名稱表；`event_names` 參數同 [`Buffers::event_names`]。
fn build_names(
    keys: impl Iterator<Item = (u32, u32)>,
    mut event_names: impl FnMut(u32, u32, u32, &[String]) -> Vec<String>,
) -> Names {
    let modifiers = &TABLES.distinct_modifiers;
    let mut from_name: HashMap<String, Vec<(usize, Entry)>> = HashMap::new();
    let mut scan_code_to_vk = HashMap::new();
    for (scan_code, vk) in keys {
        scan_code_to_vk.entry(scan_code).or_insert(vk);
        for extended in [0, 1] {
            for (m, mods) in modifiers.iter().enumerate() {
                let names = event_names(scan_code, vk, extended, mods);
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
        if let Some(name) = vk_char_name(unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_CHAR) }) {
            names.push(name);
        }

        if !is_keypad && let Some(name) = official {
            names.push(name.clone());
        }
        names
    }
}

/// `MapVirtualKey(MAPVK_VK_TO_CHAR)` 的字元：取低位 word（最高位元是 dead key 旗標）。
///
/// Python 版取 `& 0xFF`，西里爾等字母被截成別的字元（`Р` U+0420 變空白），名稱撞到別顆鍵。
fn vk_char_name(raw: u32) -> Option<String> {
    char::from_u32(raw & 0xFFFF)
        .filter(|&ch| ch != '\0')
        .map(String::from)
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

    const V: (u32, u32) = (0x2F, 0x56);
    const H: (u32, u32) = (0x23, 0x48);
    const SPACE: (u32, u32) = (0x39, 0x20);

    /// 假的俄文配置：依序是 ToUnicode、GetKeyNameText、MAPVK_VK_TO_CHAR、官方虛擬鍵名稱。
    fn russian_names() -> Names {
        let names = |sc, vk| -> Vec<String> {
            let (typed, key_name, vk_char, official) = match (sc, vk) {
                V => ("м", "М", 0x041C, "v"),
                H => ("р", "Р", 0x0420, "h"),
                SPACE => (" ", "Space", 0x20, "spacebar"),
                _ => return Vec::new(),
            };
            [
                Some(typed.into()),
                Some(key_name.into()),
                vk_char_name(vk_char),
                Some(official.into()),
            ]
            .into_iter()
            .flatten()
            .collect()
        };
        build_names([V, H, SPACE].into_iter(), |sc, vk, _, mods| {
            if mods.is_empty() {
                names(sc, vk)
            } else {
                Vec::new()
            }
        })
    }

    fn no_other_layout(_: char) -> Option<u32> {
        None
    }

    #[test]
    fn letter_names_resolve_to_their_virtual_key_on_a_non_latin_layout() {
        let names = russian_names();
        let codes = |name| names.codes(name, no_other_layout);
        assert_eq!(codes("v"), [V.0 as Code]);
        assert_eq!(codes("h"), [H.0 as Code]);
        assert_eq!(
            codes("space"),
            [SPACE.0 as Code],
            "no other key is named space"
        );
    }

    #[test]
    fn a_localized_name_saved_by_an_older_version_still_resolves() {
        let names = russian_names();
        assert_eq!(names.codes("м", no_other_layout), [V.0 as Code]);
        let latin_only = build_names([V].into_iter(), |_, _, _, mods| {
            if mods.is_empty() {
                vec!["v".into()]
            } else {
                Vec::new()
            }
        });
        let russian_vk = |ch| (ch == 'м').then_some(V.1);
        assert_eq!(latin_only.codes("м", russian_vk), [V.0 as Code]);
        assert!(latin_only.codes("м", no_other_layout).is_empty());
        assert!(latin_only.codes("ж", russian_vk).is_empty());
    }

    #[test]
    fn vk_char_name_keeps_the_whole_character() {
        assert_eq!(vk_char_name(0x0420).as_deref(), Some("Р"));
        assert_eq!(vk_char_name(0x41).as_deref(), Some("A"));
        assert_eq!(
            vk_char_name(0x8000_0027).as_deref(),
            Some("'"),
            "dead key flag"
        );
        assert_eq!(vk_char_name(0), None);
    }

    #[test]
    fn recorded_letter_and_digit_names_do_not_depend_on_the_layout() {
        for (vk, name, recorded) in [
            (0x56, "м", "v"),
            (0x56, "М", "v"),
            (0x51, ";", "q"),
            (0x32, "ě", "2"),
            (0x41, "a", "a"),
            (0x41, "A", "A"),
            (0x31, "!", "!"),
            (0x31, "&", "&"),
            (0xBA, "ж", "ж"),
            (0x20, "Space", "Space"),
        ] {
            assert_eq!(
                layout_independent_name(vk, name.into()),
                recorded,
                "{vk:#x} {name:?}"
            );
        }
    }
}
