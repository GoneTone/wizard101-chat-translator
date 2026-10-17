//! 設定介面測試用的假 `SettingsEnv`：只記下發起的測試連線與模型清單請求。
#![allow(dead_code)]

use std::cell::RefCell;
use std::path::Path;

use wizard101_chat_translator::config::services::ServiceSpec;
use wizard101_chat_translator::ui::settings::env::SettingsEnv;

#[derive(Default)]
pub struct FakeEnv {
    pub tests: RefCell<Vec<(u64, ServiceSpec, String)>>,
    pub lists: RefCell<Vec<(u64, ServiceSpec)>>,
}

impl SettingsEnv for FakeEnv {
    fn test_connection(&self, round: u64, spec: ServiceSpec, target: String) {
        self.tests.borrow_mut().push((round, spec, target));
    }
    fn list_models(&self, round: u64, spec: ServiceSpec) {
        self.lists.borrow_mut().push((round, spec));
    }
    fn check_update(&self, _round: u64) {}
    fn record_hotkey(&self, _round: u64) {}
    fn cancel_recording(&self) {}
    fn open_folder(&self, _path: &Path) {}
    fn has_cache(&self) -> bool {
        false
    }
    fn clear_cache(&self) -> usize {
        0
    }
}
