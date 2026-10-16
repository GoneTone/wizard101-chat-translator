//! 在模組記憶體中搜尋位元組 pattern（語意比照 wizwalker `pattern_scan`）。

use std::fmt;

use regex::bytes::Regex;
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_READONLY, PAGE_READWRITE,
};

use super::memory::{MemError, ProcessMemory};
use crate::translation::pyrepr::repr_str;

/// 保護屬性必須「恰為」其中之一，帶 PAGE_GUARD 等修飾旗標的區段不掃（同 wizwalker）。
const SCANNABLE: [u32; 4] = [
    PAGE_EXECUTE_READ,
    PAGE_EXECUTE_READWRITE,
    PAGE_READWRITE,
    PAGE_READONLY,
];

/// pattern 掃描錯誤。
#[derive(Debug)]
pub enum ScanError {
    /// 沒有命中（對應 wizwalker `PatternFailed`），帶 pattern 原文。
    NotFound(String),
    /// 命中不只一處（對應 wizwalker `PatternMultipleResults`），帶命中數與 pattern 原文。
    Multiple(usize, String),
    /// pattern 無法編譯。
    Invalid(String),
    /// 讀取模組或區段失敗。
    Mem(MemError),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(p) => write!(
                f,
                "Pattern {} failed. You most likely need to restart the client.",
                py_bytes(p)
            ),
            Self::Multiple(n, p) => write!(f, "Got {n} results for {}", py_bytes(p)),
            Self::Invalid(err) => write!(f, "invalid pattern: {err}"),
            Self::Mem(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for ScanError {}

impl ScanError {
    /// Python 版對應的例外類別名稱（log 的 `{type}: {exc}` 格式用）。
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "PatternFailed",
            Self::Multiple(..) => "PatternMultipleResults",
            Self::Invalid(_) => "error",
            Self::Mem(err) => err.type_name(),
        }
    }
}

/// Python 對 pattern bytes 的 repr；pattern 原文是 ASCII，與 `repr(str)` 只差 `b` 前綴。
fn py_bytes(pattern: &str) -> String {
    format!("b{}", repr_str(pattern))
}

impl From<MemError> for ScanError {
    fn from(err: MemError) -> Self {
        Self::Mem(err)
    }
}

/// 把與 Python 相同的跳脫字串編成位元組 regex（DOTALL、不走 Unicode）。
pub fn compile(pattern: &str) -> Result<Regex, ScanError> {
    Regex::new(&format!("(?s-u){pattern}")).map_err(|e| ScanError::Invalid(e.to_string()))
}

/// 掃描模組的可讀區段，回傳唯一命中的位址；比對不跨區段。
pub fn scan(mem: &dyn ProcessMemory, module: &str, pattern: &str) -> Result<u64, ScanError> {
    let regex = compile(pattern)?;
    let mut found = Vec::new();
    for region in mem.regions(module)? {
        if region.state != MEM_COMMIT || !SCANNABLE.contains(&region.protect) {
            continue;
        }
        let bytes = mem.read(region.base, region.size)?;
        found.extend(
            regex
                .find_iter(&bytes)
                .map(|m| region.base + m.start() as u64),
        );
    }
    match found[..] {
        [] => Err(ScanError::NotFound(pattern.to_owned())),
        [addr] => Ok(addr),
        _ => Err(ScanError::Multiple(found.len(), pattern.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::System::Memory::{
        MEM_RESERVE, PAGE_EXECUTE, PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS,
    };

    use super::*;
    use crate::reader::memory::{FakeMemory, Region};

    const MODULE: &str = "WizardGraphicalClient.exe";
    const HIT: &[u8] = b"\x48\x8B\x01\x0A\xFF";
    const PATTERN: &str = r"\x48\x8B\x01.\xFF";

    fn memory_with(regions: &[(u32, u32, &[u8])]) -> (FakeMemory, Vec<u64>) {
        let mem = FakeMemory::new();
        mem.add_module(MODULE, 0x1_4000_0000);
        let mut bases = Vec::new();
        let mut base = 0x1_4000_1000;
        for &(state, protect, bytes) in regions {
            mem.add_region(
                MODULE,
                Region {
                    base,
                    size: bytes.len(),
                    protect,
                    state,
                },
            );
            if state == MEM_COMMIT {
                mem.set_bytes(base, bytes);
            }
            bases.push(base);
            base += bytes.len() as u64;
        }
        (mem, bases)
    }

    #[test]
    fn scan_skips_non_listed_protections() {
        let hit = [&b"\x00\x00"[..], HIT].concat();
        let skipped = [
            (MEM_COMMIT, PAGE_EXECUTE),
            (MEM_COMMIT, PAGE_NOACCESS),
            (MEM_COMMIT, PAGE_EXECUTE_WRITECOPY),
            (MEM_COMMIT, PAGE_READWRITE | PAGE_GUARD),
            (MEM_RESERVE, PAGE_READONLY),
        ];
        let mut regions: Vec<(u32, u32, &[u8])> =
            skipped.iter().map(|&(s, p)| (s, p, &hit[..])).collect();
        regions.push((MEM_COMMIT, PAGE_READONLY, &hit));
        let (mem, bases) = memory_with(&regions);
        assert_eq!(scan(&mem, MODULE, PATTERN).unwrap(), bases[5] + 2);

        let allowed: Vec<(u32, u32, &[u8])> = SCANNABLE
            .iter()
            .map(|&p| (MEM_COMMIT, p, &hit[..]))
            .collect();
        let (mem, _) = memory_with(&allowed);
        assert!(matches!(
            scan(&mem, MODULE, PATTERN),
            Err(ScanError::Multiple(4, _))
        ));
    }

    #[test]
    fn scan_does_not_span_regions() {
        let (head, tail) = HIT.split_at(2);
        let (mem, _) = memory_with(&[
            (MEM_COMMIT, PAGE_EXECUTE_READ, head),
            (MEM_COMMIT, PAGE_EXECUTE_READ, tail),
        ]);
        assert!(matches!(
            scan(&mem, MODULE, PATTERN),
            Err(ScanError::NotFound(_))
        ));
    }

    #[test]
    fn scan_reports_multiple() {
        let twice = [HIT, b"\x90", HIT].concat();
        let (mem, _) = memory_with(&[(MEM_COMMIT, PAGE_EXECUTE_READ, &twice)]);
        assert!(matches!(
            scan(&mem, MODULE, PATTERN),
            Err(ScanError::Multiple(2, _))
        ));

        let (mem, _) = memory_with(&[
            (MEM_COMMIT, PAGE_EXECUTE_READ, HIT),
            (MEM_COMMIT, PAGE_READWRITE, HIT),
        ]);
        assert!(matches!(
            scan(&mem, MODULE, PATTERN),
            Err(ScanError::Multiple(2, _))
        ));
    }

    #[test]
    fn scan_propagates_memory_errors() {
        let (mem, bases) = memory_with(&[(MEM_COMMIT, PAGE_EXECUTE_READ, HIT)]);
        assert!(matches!(
            scan(&mem, "other.dll", PATTERN),
            Err(ScanError::Mem(MemError::ModuleNotFound(_)))
        ));
        mem.fail_reads_at(bases[0]);
        assert!(matches!(
            scan(&mem, MODULE, PATTERN),
            Err(ScanError::Mem(MemError::Read(_)))
        ));
        assert!(matches!(
            scan(&mem, MODULE, r"\x4"),
            Err(ScanError::Invalid(_))
        ));
    }

    #[test]
    fn errors_display_wizwalker_messages() {
        assert_eq!(
            ScanError::NotFound(PATTERN.into()).to_string(),
            r"Pattern b'\\x48\\x8B\\x01.\\xFF' failed. You most likely need to restart the client."
        );
        assert_eq!(
            ScanError::Multiple(2, PATTERN.into()).to_string(),
            r"Got 2 results for b'\\x48\\x8B\\x01.\\xFF'"
        );
    }
}
