//! 改寫遊戲機器碼的共用部分：模組附近配置的 stub 區、跳轉編碼與 [`SimpleHook`] 的掛入順序。
//!
//! stub 在遊戲執行緒上執行，錯一個位元組就會讓遊戲當掉。

use std::fmt;
use std::time::Duration;

use crate::game::memory::{MemError, ProcessMemory};
use crate::game::pattern::ScanError;

/// 所有 hook 掃描的模組。
pub const MODULE: &str = crate::game::process::PROCESS_NAME;

/// stub 區大小（一頁）。
pub const ARENA_SIZE: usize = 0x1000;

/// 跳轉點寫回原碼後，等遊戲執行點離開 stub 的時間。
pub const STUB_EXIT_WAIT: Duration = Duration::from_millis(500);

/// hook 掛入錯誤。
#[derive(Debug)]
pub enum HookError {
    /// pattern 掃描失敗（含找不到、多重命中）。
    Scan(ScanError),
    /// 讀寫或配置失敗。
    Mem(MemError),
    /// stub 區放不下更多 stub。
    CaveFull,
    /// 跳轉距離超出 rel32。
    JumpOutOfRange,
    /// 比對到的位置與已知的 hook 形狀不符，不寫入遊戲。
    Mismatch(String),
}

impl fmt::Display for HookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scan(err) => err.fmt(f),
            Self::Mem(err) => err.fmt(f),
            Self::CaveFull => f.write_str("stub arena is full"),
            Self::JumpOutOfRange => f.write_str("jump distance does not fit in rel32"),
            Self::Mismatch(detail) => {
                write!(f, "hook site does not fit this game build: {detail}")
            }
        }
    }
}

impl std::error::Error for HookError {}

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

/// 已寫入的跳轉：解除時寫回原碼，有 export 要釋放就在等待後釋放；只由 stub 區持有與解除。
#[derive(Debug)]
struct JumpPatch {
    addr: u64,
    original: Vec<u8>,
    free: Option<u64>,
}

/// 一個跳轉式 hook 的參數。
pub(crate) struct SimpleHook {
    pub instruction_length: u64,
    pub noops: usize,
    pub stub_size: usize,
    pub export_size: usize,
    /// 解除時是否釋放 export。
    pub free_export: bool,
}

impl SimpleHook {
    /// 在 `jump` 掛上並登記到 stub 區（之後只能由 stub 區解除），回傳 export 位址。
    ///
    /// `body` 由 export 位址產生 stub（不含回跳）。
    pub(crate) fn install(
        &self,
        mem: &dyn ProcessMemory,
        arena: &mut StubArena,
        jump: u64,
        body: impl FnOnce(u64) -> Vec<u8>,
    ) -> Result<u64, HookError> {
        let hook = arena.alloc_stub(self.stub_size)?;
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
        arena.hooks.push(JumpPatch {
            addr: jump,
            original,
            free: self.free_export.then_some(export),
        });
        Ok(export)
    }
}

/// 在遊戲模組附近配置、用來放 hook stub 的一頁可執行記憶體。
pub struct StubArena {
    addr: u64,
    pos: usize,
    hooks: Vec<JumpPatch>,
}

impl StubArena {
    /// 在模組基址的 rel32 範圍內配置 `ARENA_SIZE` bytes。
    pub fn install(mem: &dyn ProcessMemory) -> Result<Self, HookError> {
        let addr = mem.alloc_near(mem.module_base(MODULE)?, ARENA_SIZE)?;
        Ok(Self {
            addr,
            pos: 0,
            hooks: Vec::new(),
        })
    }

    /// stub 區的起點。
    pub fn addr(&self) -> u64 {
        self.addr
    }

    /// 依序切出 `size` bytes 放 stub（不回收）。
    pub fn alloc_stub(&mut self, size: usize) -> Result<u64, HookError> {
        if self.pos + size > ARENA_SIZE {
            return Err(HookError::CaveFull);
        }
        let addr = self.addr + self.pos as u64;
        self.pos += size;
        Ok(addr)
    }

    /// hookstate 用的還原操作：各跳轉點的原碼，依掛入順序。
    pub fn restore_ops(&self) -> Vec<(u64, Vec<u8>)> {
        self.hooks
            .iter()
            .map(|h| (h.addr, h.original.clone()))
            .collect()
    }

    /// 依掛入順序寫回各跳轉點，等 `STUB_EXIT_WAIT`（由 `wait` 執行）後釋放 export 與 stub 區；唯一的解除途徑。
    pub fn uninstall_with(
        self,
        mem: &dyn ProcessMemory,
        wait: impl FnOnce(Duration),
    ) -> Result<(), MemError> {
        if !mem.is_alive() {
            return Ok(());
        }
        // 有一處寫不回就什麼都不釋放：仍指向 stub 的跳轉會跑進已釋放的頁面
        for hook in &self.hooks {
            mem.write(hook.addr, &hook.original)?;
        }
        // 仍在 stub 裡的遊戲執行緒會寫 export 與執行 stub 區，等它離開才釋放
        wait(STUB_EXIT_WAIT);
        for addr in self.hooks.iter().filter_map(|h| h.free) {
            mem.free(addr);
        }
        mem.free(self.addr);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn alloc_stub_refuses_to_overflow_the_arena() {
        let mut arena = StubArena {
            addr: 0x1000,
            pos: 0,
            hooks: Vec::new(),
        };
        assert_eq!(arena.alloc_stub(ARENA_SIZE).unwrap(), 0x1000);
        let err = arena.alloc_stub(1).unwrap_err();
        assert_eq!(err.to_string(), "stub arena is full");
    }
}
