//! root window hook：pattern 不綁暫存器，base 暫存器由比對到的位元組推導，stub 把目前的 root window 位址寫到 export。

use super::cave::{HookError, MODULE, SimpleHook, StubArena};
use crate::game::memory::ProcessMemory;
use crate::game::pattern;
use crate::util::hex;

/// 跳轉點 pattern；兩條指令的 base 暫存器各自比對，一致性在 [`decode`] 檢查。
pub const ROOT_WINDOW_PATTERN: &str = concat!(
    r"[\x48\x49]\x8B[\x88-\x8F]\xD8\x00\x00\x00\x48\x8B\x01",
    r"[\x48\x49]\x8B[\x90-\x97]\xC8\x00\x00\x00\xFF\x50\x70\x84",
);

/// pattern 比對到的位元組數。
pub const MATCH_LEN: usize = 21;

/// 定位並解碼後的跳轉點。
pub struct RootWindowSite {
    /// 跳轉點位址（被覆寫的 `mov rcx,[base+D8]` 起點）。
    pub jump: u64,
    /// base 暫存器編號 0～15。
    pub base: u8,
    /// 被覆寫的 7 bytes 原指令。
    pub original: [u8; 7],
}

/// 由 REX 與 ModRM 組出 base 暫存器編號。
fn base_of(rex: u8, modrm: u8) -> u8 {
    (rex & 1) << 3 | (modrm & 7)
}

/// 從比對到的位元組（至少 `MATCH_LEN` bytes，多的忽略） 取出 base 暫存器與原指令；不安全或前後不一致時回錯誤說明。
pub fn decode(bytes: &[u8]) -> Result<(u8, [u8; 7]), String> {
    let bytes = bytes
        .get(..MATCH_LEN)
        .ok_or_else(|| format!("expected {MATCH_LEN} bytes, got {}", bytes.len()))?;
    let first = base_of(bytes[0], bytes[2]);
    let second = base_of(bytes[10], bytes[12]);
    if first != second {
        return Err(format!(
            "base register differs between instructions ({} vs {})",
            reg_name(first),
            reg_name(second)
        ));
    }
    if bytes[2] & 7 == 4 {
        return Err("base register needs a SIB byte (rsp/r12)".into());
    }
    if first == 1 {
        return Err("base register is rcx, clobbered by the replayed instruction".into());
    }
    let original = bytes[..7].try_into().expect("slice of 7");
    Ok((first, original))
}

/// 掃描並解碼跳轉點；比對到但解碼失敗回 [`HookError::Mismatch`]。
pub fn locate(mem: &dyn ProcessMemory) -> Result<RootWindowSite, HookError> {
    let addr = pattern::scan(mem, MODULE, ROOT_WINDOW_PATTERN)?;
    let bytes = mem.read(addr, MATCH_LEN)?;
    match decode(&bytes) {
        Ok((base, original)) => Ok(RootWindowSite {
            jump: addr,
            base,
            original,
        }),
        Err(why) => Err(HookError::Mismatch(format!(
            "root window at {addr:#x}: {why} (bytes={})",
            hex(&bytes)
        ))),
    }
}

/// 組出 stub（不含回跳）：把 `[base+D8]` 寫到 export，再重放原指令。
pub fn stub_for(export: u64, site: &RootWindowSite) -> Vec<u8> {
    [
        &[0x50][..],
        &[
            0x48 | (site.base >> 3),
            0x8B,
            0x80 | (site.base & 7),
            0xD8,
            0,
            0,
            0,
        ],
        &[0x48, 0xA3],
        &export.to_le_bytes(),
        &[0x58],
        &site.original,
    ]
    .concat()
}

const ROOT_WINDOW: SimpleHook = SimpleHook {
    instruction_length: 7,
    noops: 2,
    stub_size: 50,
    export_size: 8,
    free_export: true,
};

/// root window hook：把目前的 root window 位址寫到 export。
pub struct RootWindowHook {
    export: u64,
}

impl RootWindowHook {
    /// 掛上並登記到 stub 區（由 [`StubArena::uninstall_with`] 解除）。
    pub fn install(
        mem: &dyn ProcessMemory,
        arena: &mut StubArena,
        site: &RootWindowSite,
    ) -> Result<Self, HookError> {
        let export = ROOT_WINDOW.install(mem, arena, site.jump, |export| stub_for(export, site))?;
        Ok(Self { export })
    }

    /// 存放 root window 位址的 8 bytes。
    pub fn export_addr(&self) -> u64 {
        self.export
    }
}

/// 暫存器編號對應的名稱，供 log 使用。
pub fn reg_name(n: u8) -> &'static str {
    const NAMES: [&str; 16] = [
        "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12",
        "r13", "r14", "r15",
    ];
    NAMES[usize::from(n & 15)]
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, PAGE_EXECUTE_READ};

    use super::*;
    use crate::game::memory::{FakeMemory, Region};

    fn unhex(s: &str) -> Vec<u8> {
        crate::util::unhex(s).unwrap()
    }

    const NEW_BUILD: &str = "498b8dd8000000488b01498b95c8000000ff507084db"; // r13
    const NOV12_BUILD: &str = "498b8fd8000000488b01498b97c8000000ff507084c0"; // r15

    #[test]
    fn decode_reads_base_register() {
        assert_eq!(decode(&unhex(NEW_BUILD)).unwrap().0, 13);
        assert_eq!(decode(&unhex(NOV12_BUILD)).unwrap().0, 15);
        let rbx = unhex("488b8bd8000000488b01488b93c8000000ff507084c0");
        assert_eq!(decode(&rbx).unwrap(), (3, rbx[..7].try_into().unwrap()));
    }

    #[test]
    fn decode_rejects_unsafe_or_inconsistent_bases() {
        for bad in [
            "498b8dd8000000488b01498b97c8000000ff507084c0", // r13 / r15 不一致
            "488b8dd8000000488b01498b95c8000000ff507084c0", // rbp / r13：REX.B 不一致
            "488b8cd8000000488b01488b94c8000000ff507084c0", // rm=100（rsp）
            "498b8cd8000000488b01498b94c8000000ff507084c0", // rm=100（r12）
            "488b89d8000000488b01488b91c8000000ff507084c0", // rcx
        ] {
            assert!(decode(&unhex(bad)).is_err(), "{bad}");
        }
        let r9 = unhex("498b89d8000000488b01498b91c8000000ff507084c0");
        assert_eq!(decode(&r9).unwrap().0, 9);
    }

    #[test]
    fn pattern_matches_both_builds() {
        let re = pattern::compile(ROOT_WINDOW_PATTERN).unwrap();
        assert!(re.is_match(&unhex(NEW_BUILD)) && re.is_match(&unhex(NOV12_BUILD)));
    }

    #[test]
    fn stub_reads_derived_register_then_replays_original() {
        let site = RootWindowSite {
            jump: 0,
            base: 13,
            original: unhex(NEW_BUILD)[..7].try_into().unwrap(),
        };
        let export = 0x2000_0000u64;
        let expected = [
            &[0x50][..],
            &[0x49, 0x8B, 0x85, 0xD8, 0, 0, 0],
            &[0x48, 0xA3],
            &export.to_le_bytes(),
            &[0x58],
            &site.original,
        ]
        .concat();
        assert_eq!(stub_for(export, &site), expected);
    }

    #[test]
    fn locate_reports_mismatch_with_address_and_bytes() {
        let bad = "498b8dd8000000488b01498b97c8000000ff507084c0";
        let mem = FakeMemory::new();
        mem.add_module(MODULE, 0x1_4000_0000);
        let base = 0x1_4000_1000;
        mem.add_region(
            MODULE,
            Region {
                base,
                size: 22,
                protect: PAGE_EXECUTE_READ,
                state: MEM_COMMIT,
            },
        );
        mem.set_bytes(base, &unhex(bad));
        let Err(HookError::Mismatch(msg)) = locate(&mem) else {
            panic!("expected Mismatch");
        };
        assert!(
            msg.contains(&format!("{base:#x}")) && msg.contains(&bad[..2 * MATCH_LEN]),
            "{msg}"
        );
    }

    #[test]
    fn locate_decodes_a_matching_site() {
        let mem = FakeMemory::new();
        mem.add_module(MODULE, 0x1_4000_0000);
        mem.add_region(
            MODULE,
            Region {
                base: 0x1_4000_1000,
                size: 22,
                protect: PAGE_EXECUTE_READ,
                state: MEM_COMMIT,
            },
        );
        mem.set_bytes(0x1_4000_1000, &unhex(NEW_BUILD));
        let site = locate(&mem).unwrap();
        assert_eq!((site.jump, site.base), (0x1_4000_1000, 13));
    }

    #[test]
    fn reg_names_cover_all_sixteen() {
        assert_eq!(
            (reg_name(0), reg_name(13), reg_name(15)),
            ("rax", "r13", "r15")
        );
    }
}
