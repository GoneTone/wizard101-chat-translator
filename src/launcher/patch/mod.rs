//! 遊戲啟動前的自動更新：清單解析、CRC 與檢查紀錄。

pub mod cache;
pub mod crc;
pub mod fetch;
pub mod list;
pub mod server;
pub mod url;

use super::wire::WireError;

/// 更新檢查失敗的原因；任一種都不開遊戲。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchFailure {
    /// Steam 版本過舊，需由 Steam 更新。
    SteamOutdated,
    /// 遊戲執行中，無法更新。
    GameOpen,
    /// 連不上更新伺服器或逾時。
    Connect(String),
    /// 回應格式不符預期。
    Protocol(String),
    /// 下載或驗證檔案失敗。
    Download(String),
    /// 磁碟空間不足。
    NoSpace { need: u64, free: u64 },
    /// 本機檔案讀寫失敗。
    Io(String),
}

impl From<WireError> for PatchFailure {
    fn from(e: WireError) -> Self {
        match e {
            WireError::Io(m) => PatchFailure::Connect(m),
            WireError::Protocol(m) => PatchFailure::Protocol(m),
        }
    }
}
