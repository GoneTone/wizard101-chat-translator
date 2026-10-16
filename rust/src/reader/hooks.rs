//! 改寫遊戲機器碼的 hook：autobot code cave 與 root window hook（比照 wizwalker `HookHandler`／`SimpleHook`）。
//!
//! 位元組必須與 wizwalker 寫入的逐一相同，錯一個就會讓遊戲當掉。

use std::fmt;
use std::time::Duration;

use super::memory::{MemError, ProcessMemory};
use super::pattern::{self, ScanError};

/// 所有 hook 掃描的模組。
pub const MODULE: &str = "WizardGraphicalClient.exe";

/// autobot 函式的 pattern（wizwalker `HookHandler.AUTOBOT_PATTERN` 原文）；原文長度即要保存的位元組數。
pub const AUTOBOT_PATTERN: &str = concat!(
    r"\x48\x89\x5C\x24.\x48\x89\x74\x24.\x48\x89\x7C\x24.",
    r"\x55\x41\x54\x41\x55\x41\x56\x41\x57",
    r"\x48\x8D\xAC\x24....\x48\x81\xEC....",
    r"\x48\x8B\x05....\x48\x33\xC4\x48\x89\x85....",
    r"\x4C\x8B\xF1.......\x80......\x0F\x84....",
);

/// cave 清成 0 的大小（wizwalker `AUTOBOT_SIZE`）。
pub const AUTOBOT_SIZE: usize = 4100;

/// root window hook 的跳轉點 pattern（wizwalker `RootWindowHook.pattern` 原文）。
pub const ROOT_WINDOW_PATTERN: &str = r".......\x48\x8B\x01.......\xFF\x50\x70\x84";

/// hook 掛入錯誤。
#[derive(Debug)]
pub enum HookError {
    /// pattern 掃描失敗（含找不到、多重命中）。
    Scan(ScanError),
    /// 讀寫或配置失敗。
    Mem(MemError),
    /// cave 放不下更多 stub（對應 wizwalker 的 `RuntimeError`）。
    CaveFull,
    /// 跳轉距離超出 rel32（對應 Python `struct.pack("<i")` 的 `struct.error`）。
    JumpOutOfRange,
}

impl fmt::Display for HookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scan(err) => err.fmt(f),
            Self::Mem(err) => err.fmt(f),
            Self::CaveFull => f.write_str("Somehow went over autobot size"),
            Self::JumpOutOfRange => {
                f.write_str("'i' format requires -2147483648 <= number <= 2147483647")
            }
        }
    }
}

impl std::error::Error for HookError {}

impl HookError {
    /// Python 版對應的例外類別名稱（log 的 `{type}: {exc}` 格式用）。
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Scan(err) => err.type_name(),
            Self::Mem(err) => err.type_name(),
            Self::CaveFull => "RuntimeError",
            Self::JumpOutOfRange => "error",
        }
    }
}

impl From<ScanError> for HookError {
    fn from(err: ScanError) -> Self {
        Self::Scan(err)
    }
}

impl From<MemError> for HookError {
    fn from(err: MemError) -> Self {
        Self::Mem(err)
    }
}

/// `E9 rel32`：從 `from` 跳到 `to`。
fn jmp(from: u64, to: u64) -> Result<[u8; 5], HookError> {
    let rel =
        i32::try_from(to.wrapping_sub(from + 5) as i64).map_err(|_| HookError::JumpOutOfRange)?;
    let mut out = [0xE9, 0, 0, 0, 0];
    out[1..].copy_from_slice(&rel.to_le_bytes());
    Ok(out)
}

/// 已寫入的跳轉：解除時寫回原碼，有 export 要釋放就一併釋放；只由 cave 持有與解除。
#[derive(Debug)]
struct JumpPatch {
    addr: u64,
    original: Vec<u8>,
    free: Option<u64>,
}

impl JumpPatch {
    fn undo(&self, mem: &dyn ProcessMemory) -> Result<(), MemError> {
        mem.write(self.addr, &self.original)?;
        if let Some(addr) = self.free {
            mem.free(addr);
        }
        Ok(())
    }
}

/// wizwalker `SimpleHook` 的參數。
pub(crate) struct SimpleHook {
    pub pattern: &'static str,
    pub instruction_length: u64,
    pub noops: usize,
    pub stub_size: usize,
    pub export_size: usize,
    /// 解除時是否釋放 export。
    pub free_export: bool,
}

impl SimpleHook {
    /// 照 `SimpleHook.hook()` 的順序掛上並登記到 cave（之後只能由 cave 解除），回傳（跳轉點, export）位址。
    ///
    /// `body` 由 export 位址產生 stub（不含回跳）。
    pub(crate) fn install(
        &self,
        mem: &dyn ProcessMemory,
        cave: &mut AutobotCave,
        body: impl FnOnce(u64) -> Vec<u8>,
    ) -> Result<(u64, u64), HookError> {
        let jump = pattern::scan(mem, MODULE, self.pattern)?;
        let hook = cave.alloc_stub(self.stub_size)?;
        let export = mem.alloc(self.export_size)?;
        let mut stub = body(export);
        stub.extend(jmp(
            hook + stub.len() as u64,
            jump + self.instruction_length,
        )?);
        let mut jump_bytes = jmp(jump, hook)?.to_vec();
        jump_bytes.resize(5 + self.noops, 0x90);
        let original = mem.read(jump, jump_bytes.len())?;
        mem.write(hook, &stub)?;
        mem.write(jump, &jump_bytes)?;
        cave.hooks.push(JumpPatch {
            addr: jump,
            original,
            free: self.free_export.then_some(export),
        });
        Ok((jump, export))
    }
}

/// autobot code cave：遊戲裡一段被清成 0、用來放 hook stub 的函式。
pub struct AutobotCave {
    addr: u64,
    original: Vec<u8>,
    pos: usize,
    hooks: Vec<JumpPatch>,
}

impl AutobotCave {
    /// 掃 autobot 函式，保存開頭 208 bytes 後清成 4100 個 0。
    pub fn install(mem: &dyn ProcessMemory) -> Result<Self, HookError> {
        let addr = pattern::scan(mem, MODULE, AUTOBOT_PATTERN)?;
        let original = mem.read(addr, AUTOBOT_PATTERN.len())?;
        mem.write(addr, &[0; AUTOBOT_SIZE])?;
        Ok(Self {
            addr,
            original,
            pos: 0,
            hooks: Vec::new(),
        })
    }

    /// 依序切出 `size` bytes 放 stub（不回收）。
    pub fn alloc_stub(&mut self, size: usize) -> Result<u64, HookError> {
        if self.pos + size > AUTOBOT_SIZE {
            return Err(HookError::CaveFull);
        }
        let addr = self.addr + self.pos as u64;
        self.pos += size;
        Ok(addr)
    }

    /// hookstate 用的還原操作：各 hook 的跳轉原碼在前、cave 原本的 208 bytes 在最後。
    ///
    /// 依序寫回：先還原 cave 的話，仍指向它的跳轉點會跑進被改回的原始程式碼。
    pub fn restore_ops(&self) -> Vec<(u64, Vec<u8>)> {
        self.hooks
            .iter()
            .map(|h| (h.addr, h.original.clone()))
            .chain([(self.addr, self.original.clone())])
            .collect()
    }

    /// 依掛入順序解除每個 hook，等 0.5 秒後在內容不同時還原 cave 的 208 bytes；唯一的解除途徑。
    pub fn uninstall(self, mem: &dyn ProcessMemory) -> Result<(), MemError> {
        self.uninstall_with(mem, std::thread::sleep)
    }

    /// 同 [`AutobotCave::uninstall`]，等待由 `wait` 執行（測試用）。
    pub fn uninstall_with(
        self,
        mem: &dyn ProcessMemory,
        wait: impl FnOnce(Duration),
    ) -> Result<(), MemError> {
        // 遊戲已關就沒有東西要還原（同 wizwalker `Client.close`）
        if !mem.is_alive() {
            return Ok(());
        }
        for hook in &self.hooks {
            hook.undo(mem)?;
        }
        let current = mem.read(self.addr, self.original.len())?;
        // 讓執行點離開 stub
        wait(Duration::from_millis(500));
        if current != self.original {
            mem.write(self.addr, &self.original)?;
        }
        Ok(())
    }
}

const ROOT_WINDOW: SimpleHook = SimpleHook {
    pattern: ROOT_WINDOW_PATTERN,
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
    /// 掛上並登記到 cave（由 [`AutobotCave::uninstall`] 解除）。
    pub fn install(mem: &dyn ProcessMemory, cave: &mut AutobotCave) -> Result<Self, HookError> {
        let (_, export) = ROOT_WINDOW.install(mem, cave, |export| {
            [
                &[0x50][..],                                 // push rax
                &[0x49, 0x8B, 0x85, 0xD8, 0x00, 0x00, 0x00], // mov rax,[r13+D8]
                &[0x48, 0xA3],                               // mov [export],rax
                &export.to_le_bytes(),
                &[0x58],                                     // pop rax
                &[0x49, 0x8B, 0x8D, 0xD8, 0x00, 0x00, 0x00], // mov rcx,[r13+D8]（原指令）
            ]
            .concat()
        })?;
        Ok(Self { export })
    }

    /// 存放 root window 位址的 8 bytes。
    pub fn export_addr(&self) -> u64 {
        self.export
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autobot_pattern_is_208_chars() {
        assert_eq!(AUTOBOT_PATTERN.len(), 208);
    }

    #[test]
    fn jmp_encodes_rel32_both_directions() {
        assert_eq!(jmp(0x1000, 0x2000).unwrap(), [0xE9, 0xFB, 0x0F, 0x00, 0x00]);
        assert_eq!(jmp(0x2000, 0x1000).unwrap(), [0xE9, 0xFB, 0xEF, 0xFF, 0xFF]);
        assert_eq!(jmp(0, 0x8000_0000).unwrap(), [0xE9, 0xFB, 0xFF, 0xFF, 0x7F]);
        assert!(matches!(
            jmp(0, 0x8000_0005),
            Err(HookError::JumpOutOfRange)
        ));
        assert!(matches!(
            jmp(0x8000_0000, 0),
            Err(HookError::JumpOutOfRange)
        ));
    }

    #[test]
    fn alloc_stub_refuses_to_overflow_the_cave() {
        let mut cave = AutobotCave {
            addr: 0x1000,
            original: Vec::new(),
            pos: 0,
            hooks: Vec::new(),
        };
        assert_eq!(cave.alloc_stub(AUTOBOT_SIZE).unwrap(), 0x1000);
        let err = cave.alloc_stub(1).unwrap_err();
        assert_eq!(err.to_string(), "Somehow went over autobot size");
    }
}
