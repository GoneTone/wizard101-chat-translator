//! 內建本機模型：從 Hugging Face 取得並執行 GGUF 模型。

use std::sync::{Arc, RwLock};

use crate::logs::log;
use crate::util::{read, write};

pub mod catalog;
pub mod download;
mod downloads;
pub mod engine;
pub mod fit;
pub mod gguf;
pub mod hardware;
pub mod hf;
mod hub;
mod runtime;
pub mod server;
pub mod store;

pub use hub::{
    Command, DownloadState, Endpoints, EngineState, Local, RepoInfo, RepoState, Snapshot,
};
pub use runtime::Loading;

/// hub 的通知出口（UI 轉成重繪與載入橫幅）。
pub type Notify = Arc<dyn Fn(LocalNotice) + Send + Sync>;

/// hub 送給 UI 的通知。
#[derive(Clone, Debug, PartialEq)]
pub enum LocalNotice {
    /// 快照內容變了。
    Changed,
    /// `Some(模型顯示名)` 開始載入模型，`None` 載入結束。
    Loading(Option<String>),
}

static LOCAL: RwLock<Option<Arc<Local>>> = RwLock::new(None);

/// 設定全程序共用的 hub（正式只在開任何視窗前呼叫一次；測試會換掉前一個）。
pub fn install(local: Arc<Local>) {
    if write(&LOCAL).replace(local).is_some() {
        log("[local] hub replaced");
    }
}

/// 全程序共用的 hub；尚未 `install` 時為 `None`。
pub fn get() -> Option<Arc<Local>> {
    read(&LOCAL).clone()
}

/// 測試用：卸下全程序共用的 hub（它的 runtime 隨測試結束）。
#[cfg(any(test, feature = "test-support"))]
pub fn uninstall_for_test() {
    write(&LOCAL).take();
}
