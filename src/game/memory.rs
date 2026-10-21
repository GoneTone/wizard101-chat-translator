//! 遊戲程序記憶體的讀寫、配置與區段列舉（取代 wizwalker／pymem 的對應部分）。

use std::fmt;
use std::mem::{size_of, size_of_val, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, HMODULE, STILL_ACTIVE};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_DEBUG_NAME,
    SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READWRITE, VirtualAllocEx,
    VirtualFreeEx, VirtualQueryEx,
};
use windows_sys::Win32::System::ProcessStatus::{
    EnumProcessModulesEx, GetModuleBaseNameW, GetModuleInformation, LIST_MODULES_ALL, MODULEINFO,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcess, OpenProcessToken, PROCESS_ALL_ACCESS,
};

use crate::logs::log;

/// wizwalker `read_string` 接受的最大長度（超過視為讀到垃圾）。
const MAX_STRING: i32 = 5_000;

/// 一個記憶體區段（`VirtualQueryEx` 的結果）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    /// 區段起點。
    pub base: u64,
    /// 區段大小（位元組）。
    pub size: usize,
    /// 保護屬性（`PAGE_*`）。
    pub protect: u32,
    /// 狀態（`MEM_COMMIT` 等）。
    pub state: u32,
}

/// 記憶體存取錯誤。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemError {
    /// 開不了程序 handle（對應 pymem `CouldNotOpenProcess`），帶 PID。
    AccessDenied(u32),
    /// 程序已結束（對應 wizwalker `ClientClosedError`）。
    Closed,
    /// 讀取失敗（對應 wizwalker `MemoryReadError`）。
    Read(u64),
    /// 寫入失敗（對應 wizwalker `MemoryWriteError`）。
    Write(u64),
    /// 位址不在使用者空間（對應 wizwalker `AddressOutOfRange`）。
    OutOfRange(u64),
    /// 程序沒有載入這個模組。
    ModuleNotFound(String),
    /// 配置失敗，帶要求的大小。
    Alloc(usize),
}

impl fmt::Display for MemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AccessDenied(pid) => write!(f, "Could not open process: {pid}"),
            Self::Closed => write!(f, "Client must be running to preform this action."),
            Self::Read(addr) => write!(f, "Unable to read memory at address {addr}."),
            Self::Write(addr) => write!(f, "Unable to write memory at address {addr}."),
            Self::OutOfRange(addr) => write!(f, "Address {addr} out of bounds"),
            Self::ModuleNotFound(name) => write!(f, "{name} module not found."),
            Self::Alloc(size) => write!(f, "could not allocate {size} bytes"),
        }
    }
}

impl std::error::Error for MemError {}

/// 與 wizwalker `read_bytes` 相同的位址檢查。
fn check_range(addr: u64) -> Result<(), MemError> {
    if addr == 0 || addr > 0x7FFF_FFFF_FFFF_FFFF {
        return Err(MemError::OutOfRange(addr));
    }
    Ok(())
}

/// UTF-16LE 解碼；位元組不成對時行為同 Python `decode("utf-16-le", "replace")`。
pub fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
        .collect();
    let mut text = String::from_utf16_lossy(&units);
    // 結尾落單的位元組多一個 U+FFFD；前一個單位若是高代理，Python 把兩者併成同一個錯誤
    let cut_pair = units.last().is_some_and(|u| (0xD800..0xDC00).contains(u));
    if bytes.len() % 2 == 1 && !cut_pair {
        text.push('\u{FFFD}');
    }
    text
}

fn read_array<const N: usize>(
    mem: &(impl ProcessMemory + ?Sized),
    addr: u64,
) -> Result<[u8; N], MemError> {
    mem.read(addr, N)?
        .try_into()
        .map_err(|_| MemError::Read(addr))
}

/// 一個可讀寫記憶體的程序；實機用 [`WinProcess`]，測試用 `FakeMemory`。
pub trait ProcessMemory: Send {
    /// 讀 `len` 個位元組。
    fn read(&self, addr: u64, len: usize) -> Result<Vec<u8>, MemError>;
    /// 寫入位元組。
    fn write(&self, addr: u64, bytes: &[u8]) -> Result<(), MemError>;
    /// 配置一塊 `MEM_COMMIT`、可執行可讀寫的記憶體，回傳位址。
    fn alloc(&self, size: usize) -> Result<u64, MemError>;
    /// 釋放 [`ProcessMemory::alloc`] 配置的記憶體；失敗不回報（同 pymem）。
    fn free(&self, addr: u64);
    /// 模組基址（名稱不分大小寫）。
    fn module_base(&self, name: &str) -> Result<u64, MemError>;
    /// 依位址順序列出模組映像範圍內的所有區段。
    fn regions(&self, module: &str) -> Result<Vec<Region>, MemError>;
    /// 程序是否仍在執行（`GetExitCodeProcess == 259`）。
    fn is_alive(&self) -> bool;

    /// 讀小端序 u64。
    fn read_u64(&self, addr: u64) -> Result<u64, MemError> {
        read_array(self, addr).map(u64::from_le_bytes)
    }

    /// 讀小端序 i32。
    fn read_i32(&self, addr: u64) -> Result<i32, MemError> {
        read_array(self, addr).map(i32::from_le_bytes)
    }

    /// 讀小端序 u32。
    fn read_u32(&self, addr: u64) -> Result<u32, MemError> {
        read_array(self, addr).map(u32::from_le_bytes)
    }

    /// 讀 MSVC `std::string`（比照 wizwalker `read_string`）。
    ///
    /// 長度不在 1..=5000 或內容不是合法 UTF-8 時回空字串。
    fn read_std_string(&self, addr: u64) -> Result<String, MemError> {
        let len = self.read_i32(addr.wrapping_add(16))?;
        if !(1..=MAX_STRING).contains(&len) {
            return Ok(String::new());
        }
        // 16 位元組以上不放在 SSO 緩衝，開頭改存指標
        let data = if len >= 16 {
            self.read_u64(addr)?
        } else {
            addr
        };
        let raw = self.read(data, len as usize)?;
        Ok(String::from_utf8(raw).unwrap_or_default())
    }
}

/// 以 Win32 API 存取另一個程序；Drop 時關閉 handle。
pub struct WinProcess {
    handle: HANDLE,
}

// 程序 handle 可在任一執行緒使用
unsafe impl Send for WinProcess {}

impl WinProcess {
    /// 先嘗試啟用 SeDebugPrivilege，再以 `PROCESS_ALL_ACCESS` 開啟程序。
    pub fn open(pid: u32) -> Result<Self, MemError> {
        // 視窗已消失時 GetWindowThreadProcessId 回 0：等同程序已結束
        if pid == 0 {
            return Err(MemError::Closed);
        }
        enable_debug_privilege();
        let handle = unsafe { OpenProcess(PROCESS_ALL_ACCESS, 0, pid) };
        if handle.is_null() {
            log(format!(
                "[reader] OpenProcess failed: pid={pid} error={}",
                unsafe { GetLastError() }
            ));
            return Err(MemError::AccessDenied(pid));
        }
        Ok(Self { handle })
    }

    /// 程序已結束時把錯誤換成 [`MemError::Closed`]（比照 wizwalker 失敗後才檢查）。
    fn failure(&self, err: MemError) -> MemError {
        if self.is_alive() {
            err
        } else {
            MemError::Closed
        }
    }

    fn module(&self, name: &str) -> Result<(u64, usize), MemError> {
        let mut modules: [HMODULE; 1024] = [null_mut(); 1024];
        let mut needed = 0u32;
        let ok = unsafe {
            EnumProcessModulesEx(
                self.handle,
                modules.as_mut_ptr(),
                size_of_val(&modules) as u32,
                &mut needed,
                LIST_MODULES_ALL,
            )
        };
        if ok == 0 {
            return Err(self.failure(MemError::ModuleNotFound(name.to_owned())));
        }
        let wanted = name.to_lowercase();
        let count = (needed as usize / size_of::<HMODULE>()).min(modules.len());
        for &module in &modules[..count] {
            let mut buf = [0u16; 260];
            let n = unsafe {
                GetModuleBaseNameW(self.handle, module, buf.as_mut_ptr(), buf.len() as u32)
            };
            if String::from_utf16_lossy(&buf[..n as usize]).to_lowercase() != wanted {
                continue;
            }
            let mut info: MODULEINFO = unsafe { zeroed() };
            let ok = unsafe {
                GetModuleInformation(
                    self.handle,
                    module,
                    &mut info,
                    size_of::<MODULEINFO>() as u32,
                )
            };
            if ok != 0 {
                return Ok((info.lpBaseOfDll as u64, info.SizeOfImage as usize));
            }
        }
        Err(self.failure(MemError::ModuleNotFound(name.to_owned())))
    }
}

impl Drop for WinProcess {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

/// 非系統管理員拿不到這個權限屬正常，失敗就照原權限開程序（同 pymem）。
fn enable_debug_privilege() {
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        ) == 0
        {
            return;
        }
        let mut privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: zeroed(),
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        if LookupPrivilegeValueW(null(), SE_DEBUG_NAME, &mut privileges.Privileges[0].Luid) != 0 {
            AdjustTokenPrivileges(token, 0, &privileges, 0, null_mut(), null_mut());
        }
        CloseHandle(token);
    }
}

impl ProcessMemory for WinProcess {
    fn read(&self, addr: u64, len: usize) -> Result<Vec<u8>, MemError> {
        check_range(addr)?;
        let mut buf = vec![0u8; len];
        let ok = unsafe {
            ReadProcessMemory(
                self.handle,
                addr as *const _,
                buf.as_mut_ptr().cast(),
                len,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(self.failure(MemError::Read(addr)));
        }
        Ok(buf)
    }

    fn write(&self, addr: u64, bytes: &[u8]) -> Result<(), MemError> {
        let ok = unsafe {
            WriteProcessMemory(
                self.handle,
                addr as *const _,
                bytes.as_ptr().cast(),
                bytes.len(),
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(self.failure(MemError::Write(addr)));
        }
        Ok(())
    }

    fn alloc(&self, size: usize) -> Result<u64, MemError> {
        let addr = unsafe {
            VirtualAllocEx(
                self.handle,
                null(),
                size,
                MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            )
        };
        if addr.is_null() {
            return Err(self.failure(MemError::Alloc(size)));
        }
        Ok(addr as u64)
    }

    fn free(&self, addr: u64) {
        unsafe { VirtualFreeEx(self.handle, addr as *mut _, 0, MEM_RELEASE) };
    }

    fn module_base(&self, name: &str) -> Result<u64, MemError> {
        self.module(name).map(|(base, _)| base)
    }

    fn regions(&self, module: &str) -> Result<Vec<Region>, MemError> {
        let (base, size) = self.module(module)?;
        let end = base + size as u64;
        let mut regions = Vec::new();
        let mut addr = base;
        while addr < end {
            let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
            let got = unsafe {
                VirtualQueryEx(self.handle, addr as *const _, &mut mbi, size_of_val(&mbi))
            };
            if got == 0 {
                return Err(self.failure(MemError::Read(addr)));
            }
            let region = Region {
                base: mbi.BaseAddress as u64,
                size: mbi.RegionSize,
                protect: mbi.Protect,
                state: mbi.State,
            };
            regions.push(region);
            addr = region.base + region.size as u64;
        }
        Ok(regions)
    }

    fn is_alive(&self) -> bool {
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(self.handle, &mut code) };
        code == STILL_ACTIVE as u32
    }
}

#[cfg(any(test, feature = "test-support"))]
pub use fake::{FakeMemory, MemOp};

#[cfg(any(test, feature = "test-support"))]
mod fake {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::{MemError, ProcessMemory, Region, check_range};

    const FIRST_ALLOC: u64 = 0x2000_0000;
    const ALLOC_GRANULARITY: u64 = 0x1_0000;

    /// [`FakeMemory`] 依序記下的一次記憶體變動。
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum MemOp {
        /// 成功的寫入。
        Write {
            /// 寫入位址。
            addr: u64,
            /// 寫入的內容。
            bytes: Vec<u8>,
        },
        /// 配置。
        Alloc {
            /// 配置到的位址。
            addr: u64,
            /// 要求的大小。
            size: usize,
        },
        /// 釋放。
        Free(u64),
    }

    #[derive(Default)]
    struct State {
        modules: HashMap<String, (u64, Vec<Region>)>,
        bytes: HashMap<u64, u8>,
        allocs: HashMap<u64, usize>,
        failing_reads: Vec<u64>,
        ops: Vec<MemOp>,
        next_alloc: u64,
        dead: bool,
    }

    impl State {
        /// 每個位元組都有對應時才讀得到。
        fn get(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
            (0..len as u64)
                .map(|i| self.bytes.get(&addr.checked_add(i)?).copied())
                .collect()
        }

        fn put(&mut self, addr: u64, bytes: &[u8]) {
            for (i, &b) in bytes.iter().enumerate() {
                self.bytes.insert(addr + i as u64, b);
            }
        }

        fn module(&self, name: &str) -> Result<&(u64, Vec<Region>), MemError> {
            if self.dead {
                return Err(MemError::Closed);
            }
            self.modules
                .get(&name.to_lowercase())
                .ok_or_else(|| MemError::ModuleNotFound(name.to_owned()))
        }
    }

    /// 測試用的假程序記憶體：可預置模組、區段與內容，並依序記下寫入、配置與釋放。
    pub struct FakeMemory {
        state: Mutex<State>,
    }

    impl Default for FakeMemory {
        fn default() -> Self {
            Self::new()
        }
    }

    impl FakeMemory {
        /// 空的假程序（活著、沒有模組）。
        pub fn new() -> Self {
            let state = State {
                next_alloc: FIRST_ALLOC,
                ..State::default()
            };
            Self {
                state: Mutex::new(state),
            }
        }

        fn state(&self) -> std::sync::MutexGuard<'_, State> {
            self.state.lock().unwrap()
        }

        /// 登記模組與其基址。
        pub fn add_module(&self, name: &str, base: u64) {
            self.state()
                .modules
                .insert(name.to_lowercase(), (base, Vec::new()));
        }

        /// 在已登記的模組下加一個區段（不放內容，內容用 [`FakeMemory::set_bytes`]）。
        pub fn add_region(&self, module: &str, region: Region) {
            let mut state = self.state();
            let entry = state
                .modules
                .get_mut(&module.to_lowercase())
                .expect("add_module first");
            entry.1.push(region);
            entry.1.sort_by_key(|r| r.base);
        }

        /// 直接放入位元組（不記入 [`FakeMemory::ops`]）。
        pub fn set_bytes(&self, addr: u64, bytes: &[u8]) {
            self.state().put(addr, bytes);
        }

        /// 之後涵蓋此位址的讀取都回 [`MemError::Read`]。
        pub fn fail_reads_at(&self, addr: u64) {
            self.state().failing_reads.push(addr);
        }

        /// 指定下一次 [`ProcessMemory::alloc`] 回傳的位址。
        pub fn set_next_alloc(&self, addr: u64) {
            self.state().next_alloc = addr;
        }

        /// 模擬程序結束：之後的存取都回 [`MemError::Closed`]。
        pub fn kill(&self) {
            self.state().dead = true;
        }

        /// 至今依序的寫入、配置與釋放。
        pub fn ops(&self) -> Vec<MemOp> {
            self.state().ops.clone()
        }
    }

    impl ProcessMemory for FakeMemory {
        fn read(&self, addr: u64, len: usize) -> Result<Vec<u8>, MemError> {
            check_range(addr)?;
            let state = self.state();
            if state.dead {
                return Err(MemError::Closed);
            }
            let end = addr.saturating_add(len as u64);
            if state
                .failing_reads
                .iter()
                .any(|&bad| (addr..end.max(addr + 1)).contains(&bad))
            {
                return Err(MemError::Read(addr));
            }
            state.get(addr, len).ok_or(MemError::Read(addr))
        }

        fn write(&self, addr: u64, bytes: &[u8]) -> Result<(), MemError> {
            let mut state = self.state();
            if state.dead {
                return Err(MemError::Closed);
            }
            if state.get(addr, bytes.len()).is_none() {
                return Err(MemError::Write(addr));
            }
            state.put(addr, bytes);
            state.ops.push(MemOp::Write {
                addr,
                bytes: bytes.to_vec(),
            });
            Ok(())
        }

        fn alloc(&self, size: usize) -> Result<u64, MemError> {
            let mut state = self.state();
            if state.dead {
                return Err(MemError::Closed);
            }
            let addr = state.next_alloc;
            state.next_alloc +=
                (size as u64).div_ceil(ALLOC_GRANULARITY).max(1) * ALLOC_GRANULARITY;
            state.put(addr, &vec![0; size]);
            state.allocs.insert(addr, size);
            state.ops.push(MemOp::Alloc { addr, size });
            Ok(addr)
        }

        fn free(&self, addr: u64) {
            let mut state = self.state();
            // 對已結束程序的 VirtualFreeEx 什麼都不會發生
            if state.dead {
                return;
            }
            if let Some(size) = state.allocs.remove(&addr) {
                for a in addr..addr + size as u64 {
                    state.bytes.remove(&a);
                }
            }
            state.ops.push(MemOp::Free(addr));
        }

        fn module_base(&self, name: &str) -> Result<u64, MemError> {
            self.state().module(name).map(|(base, _)| *base)
        }

        fn regions(&self, module: &str) -> Result<Vec<Region>, MemError> {
            self.state()
                .module(module)
                .map(|(_, regions)| regions.clone())
        }

        fn is_alive(&self) -> bool {
            !self.state().dead
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_memory_records_write_order() {
        let mem = FakeMemory::new();
        let a = mem.alloc(0x10).unwrap();
        let b = mem.alloc(0x2_0000).unwrap();
        assert_ne!(a, b);
        mem.write(b + 4, &[1, 2]).unwrap();
        mem.write(a, &[9]).unwrap();
        assert_eq!(mem.write(0x10, &[0]), Err(MemError::Write(0x10)));
        mem.free(a);
        assert_eq!(
            mem.ops(),
            vec![
                MemOp::Alloc {
                    addr: a,
                    size: 0x10
                },
                MemOp::Alloc {
                    addr: b,
                    size: 0x2_0000
                },
                MemOp::Write {
                    addr: b + 4,
                    bytes: vec![1, 2]
                },
                MemOp::Write {
                    addr: a,
                    bytes: vec![9]
                },
                MemOp::Free(a),
            ]
        );
        assert_eq!(mem.read(b + 3, 4).unwrap(), vec![0, 1, 2, 0]);
        assert_eq!(mem.read(a, 1), Err(MemError::Read(a)));
    }

    #[test]
    fn fake_memory_simulates_failures() {
        let mem = FakeMemory::new();
        mem.add_module("Game.exe", 0x40_0000);
        mem.set_bytes(0x1000, &[0; 16]);
        mem.fail_reads_at(0x1008);
        assert_eq!(mem.read(0x1000, 8).unwrap(), vec![0; 8]);
        assert_eq!(mem.read(0x1004, 8), Err(MemError::Read(0x1004)));
        assert_eq!(mem.read(0, 8), Err(MemError::OutOfRange(0)));
        assert_eq!(mem.module_base("GAME.EXE"), Ok(0x40_0000));
        assert!(matches!(
            mem.module_base("other.dll"),
            Err(MemError::ModuleNotFound(_))
        ));
        assert!(mem.is_alive());
        mem.kill();
        assert!(!mem.is_alive());
        assert_eq!(mem.read(0x1000, 8), Err(MemError::Closed));
        assert_eq!(mem.write(0x1000, &[1]), Err(MemError::Closed));
        assert_eq!(mem.module_base("Game.exe"), Err(MemError::Closed));
        let ops = mem.ops();
        mem.free(0x1000);
        assert_eq!(mem.ops(), ops);
    }

    #[test]
    fn fake_memory_overlapping_and_adjacent_bytes_merge() {
        let mem = FakeMemory::new();
        mem.set_bytes(0x1000, &[0; 0x20]);
        mem.set_bytes(0x1018, &[1; 16]);
        assert_eq!(mem.read(0x1010, 16).unwrap(), [[0; 8], [1; 8]].concat());
        assert_eq!(mem.read(0x1020, 8).unwrap(), vec![1; 8]);
        mem.set_bytes(0x1028, &[2; 4]);
        assert_eq!(mem.read(0x1026, 4).unwrap(), vec![1, 1, 2, 2]);
        mem.write(0x101E, &[3; 6]).unwrap();
        assert_eq!(
            mem.read(0x101C, 10).unwrap(),
            [&[1, 1][..], &[3; 6], &[1, 1]].concat()
        );
        assert_eq!(mem.read(0x102A, 4), Err(MemError::Read(0x102A)));
        assert_eq!(mem.write(0x102A, &[0; 4]), Err(MemError::Write(0x102A)));
    }

    #[test]
    fn errors_display_wizwalker_messages() {
        assert_eq!(
            MemError::Closed.to_string(),
            "Client must be running to preform this action."
        );
        assert_eq!(
            MemError::Read(4096).to_string(),
            "Unable to read memory at address 4096."
        );
        assert_eq!(
            MemError::Write(4096).to_string(),
            "Unable to write memory at address 4096."
        );
        assert_eq!(
            MemError::OutOfRange(0).to_string(),
            "Address 0 out of bounds"
        );
    }

    #[test]
    fn typed_reads_are_little_endian() {
        let mem = FakeMemory::new();
        mem.set_bytes(0x1000, &[0x78, 0x56, 0x34, 0x12, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(mem.read_u32(0x1000), Ok(0x1234_5678));
        assert_eq!(mem.read_i32(0x1004), Ok(-1));
        assert_eq!(mem.read_u64(0x1000), Ok(0xFFFF_FFFF_1234_5678));
        let text: Vec<u8> = "聊天"
            .encode_utf16()
            .chain([0xD800])
            .flat_map(u16::to_le_bytes)
            .collect();
        mem.set_bytes(0x2000, &text);
        assert_eq!(
            decode_utf16le(&mem.read(0x2000, 6).unwrap()),
            "聊天\u{FFFD}"
        );
    }

    fn std_string(mem: &FakeMemory, at: u64, len: i32, data: u64) {
        let mut head = [0u8; 0x20];
        head[..8].copy_from_slice(&data.to_le_bytes());
        head[16..20].copy_from_slice(&len.to_le_bytes());
        mem.set_bytes(at, &head);
    }

    #[test]
    fn read_std_string_follows_msvc_layout() {
        let mem = FakeMemory::new();
        mem.set_bytes(0x1000, &[0; 0x20]);
        mem.set_bytes(0x1000, b"ChatTab");
        mem.set_bytes(0x1010, &7i32.to_le_bytes());
        assert_eq!(mem.read_std_string(0x1000).unwrap(), "ChatTab");

        let long = "SixteenBytesLong";
        mem.set_bytes(0x9000, long.as_bytes());
        std_string(&mem, 0x2000, 16, 0x9000);
        assert_eq!(mem.read_std_string(0x2000).unwrap(), long);

        for len in [0, -1, MAX_STRING + 1] {
            std_string(&mem, 0x3000, len, 0x9000);
            assert_eq!(mem.read_std_string(0x3000).unwrap(), "");
        }
        mem.set_bytes(0xA000, &[0xFF; 20]);
        std_string(&mem, 0x4000, 20, 0xA000);
        assert_eq!(mem.read_std_string(0x4000).unwrap(), "");
    }

    #[test]
    fn win_process_works_on_own_process() {
        let proc = WinProcess::open(std::process::id()).unwrap();
        assert!(proc.is_alive());
        let addr = proc.alloc(64).unwrap();
        proc.write(addr + 8, &[1, 2, 3]).unwrap();
        assert_eq!(proc.read(addr + 7, 5).unwrap(), vec![0, 1, 2, 3, 0]);
        proc.free(addr);
        assert_eq!(proc.read(addr, 4), Err(MemError::Read(addr)));
        assert_eq!(proc.read(0, 4), Err(MemError::OutOfRange(0)));

        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_str().unwrap().to_uppercase();
        let base = proc.module_base(&name).unwrap();
        let regions = proc.regions(&name).unwrap();
        assert_eq!(regions[0].base, base);
        assert!(
            regions
                .windows(2)
                .all(|w| w[0].base + w[0].size as u64 == w[1].base)
        );
        let code = check_range as fn(u64) -> Result<(), MemError> as usize as u64;
        assert!(
            regions
                .iter()
                .any(|r| (r.base..r.base + r.size as u64).contains(&code))
        );
        assert!(matches!(
            proc.module_base("no-such.dll"),
            Err(MemError::ModuleNotFound(_))
        ));
        assert_eq!(WinProcess::open(0).err(), Some(MemError::Closed));
    }
}
