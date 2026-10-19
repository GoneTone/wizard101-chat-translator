//! 包住 `FakeMemory` 的偵測替身：記下讀取、可注入讀寫失敗，並能在讀取之後插手改記憶體。
#![allow(dead_code)]

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use wizard101_chat_translator::reader::memory::{FakeMemory, MemError, ProcessMemory, Region};
use wizard101_chat_translator::util;

/// 讀取之後呼叫：參數是剛讀的位址、長度與底下的記憶體（模擬遊戲執行緒的覆寫）。
pub type ReadHook = Box<dyn FnMut(u64, usize, &FakeMemory) + Send>;

pub struct Spy {
    pub mem: Arc<FakeMemory>,
    pub reads: Mutex<Vec<(u64, usize)>>,
    pub after_read: Mutex<Option<ReadHook>>,
    pub fail_writes: AtomicBool,
    fail_reads: Mutex<HashMap<u64, usize>>,
}

/// 可複製的把手：交給 reader 的那份與測試手上的那份共用同一個 `Spy`。
#[derive(Clone)]
pub struct SpyMemory(Arc<Spy>);

impl Deref for SpyMemory {
    type Target = Spy;
    fn deref(&self) -> &Spy {
        &self.0
    }
}

impl Default for SpyMemory {
    fn default() -> Self {
        Self::over(Arc::default())
    }
}

impl SpyMemory {
    pub fn over(mem: Arc<FakeMemory>) -> Self {
        Self(Arc::new(Spy {
            mem,
            reads: Mutex::default(),
            after_read: Mutex::default(),
            fail_writes: AtomicBool::default(),
            fail_reads: Mutex::default(),
        }))
    }

    /// 讀 `addr` 的接下來 `times` 次直接失敗。
    pub fn fail_reads_at(&self, addr: u64, times: usize) {
        util::lock(&self.fail_reads).insert(addr, times);
    }

    /// 到目前為止讀過 `addr` 幾次（含失敗的）。
    pub fn read_count(&self, addr: u64) -> usize {
        util::lock(&self.reads)
            .iter()
            .filter(|(a, _)| *a == addr)
            .count()
    }
}

impl ProcessMemory for SpyMemory {
    fn read(&self, addr: u64, len: usize) -> Result<Vec<u8>, MemError> {
        util::lock(&self.reads).push((addr, len));
        if let Some(left) = util::lock(&self.fail_reads).get_mut(&addr)
            && *left > 0
        {
            *left -= 1;
            return Err(MemError::Read(addr));
        }
        let out = self.mem.read(addr, len);
        if let Some(hook) = util::lock(&self.after_read).as_mut() {
            hook(addr, len, &self.mem);
        }
        out
    }

    fn write(&self, addr: u64, bytes: &[u8]) -> Result<(), MemError> {
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(MemError::Write(addr));
        }
        self.mem.write(addr, bytes)
    }

    fn alloc(&self, size: usize) -> Result<u64, MemError> {
        self.mem.alloc(size)
    }

    fn free(&self, addr: u64) {
        self.mem.free(addr)
    }

    fn module_base(&self, name: &str) -> Result<u64, MemError> {
        self.mem.module_base(name)
    }

    fn regions(&self, module: &str) -> Result<Vec<Region>, MemError> {
        self.mem.regions(module)
    }

    fn is_alive(&self) -> bool {
        self.mem.is_alive()
    }
}
