//! 設定介面測試用的假 `SettingsEnv`：記下每一種發起的工作，譯文快取的有無與筆數可設定。
#![allow(dead_code)]

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use wizard101_chat_translator::config::services::ServiceSpec;
use wizard101_chat_translator::ui::settings::env::SettingsEnv;

#[derive(Default)]
pub struct FakeEnv {
    pub tests: RefCell<Vec<(u64, ServiceSpec, String)>>,
    pub lists: RefCell<Vec<(u64, ServiceSpec)>>,
    pub checks: RefCell<Vec<u64>>,
    pub recordings: RefCell<Vec<u64>>,
    pub cancels: Cell<usize>,
    pub opened: RefCell<Vec<PathBuf>>,
    /// `None` 是沒有譯文快取；否則是清除時回報的筆數。
    pub cache: Option<usize>,
    pub cleared: Cell<usize>,
}

impl SettingsEnv for FakeEnv {
    fn test_connection(&self, round: u64, spec: ServiceSpec, target: String) {
        self.tests.borrow_mut().push((round, spec, target));
    }
    fn list_models(&self, round: u64, spec: ServiceSpec) {
        self.lists.borrow_mut().push((round, spec));
    }
    fn check_update(&self, round: u64) {
        self.checks.borrow_mut().push(round);
    }
    fn record_hotkey(&self, round: u64) {
        self.recordings.borrow_mut().push(round);
    }
    fn cancel_recording(&self) {
        self.cancels.set(self.cancels.get() + 1);
    }
    fn open_folder(&self, path: &Path) {
        self.opened.borrow_mut().push(path.to_path_buf());
    }
    fn has_cache(&self) -> bool {
        self.cache.is_some()
    }
    fn clear_cache(&self) -> usize {
        self.cleared.set(self.cleared.get() + 1);
        self.cache.unwrap_or(0)
    }
}
