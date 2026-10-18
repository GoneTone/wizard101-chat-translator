//! 把全域 log 導到測試執行檔共用的暫存檔，供斷言 log 內容（全域 sink 只有一個，整個執行檔共用）。
#![allow(dead_code)]

use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use wizard101_chat_translator::logs::{self, Sink};

/// Win32 `FILE_FLAG_DELETE_ON_CLOSE`：最後一個 handle 關掉（程序結束）時由系統刪檔。
const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;

/// 開始擷取並回傳 log 檔路徑；要斷言 log 的測試開頭先呼叫。
///
/// static 裡的 `TempDir` 不會被 drop，所以改用 DELETE_ON_CLOSE：跑完不在 %TEMP% 留檔。
pub fn capture_logs() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let exe = std::env::current_exe().unwrap();
        let stem = exe.file_stem().unwrap().to_string_lossy().into_owned();
        let path = std::env::temp_dir().join(format!("{stem}_{}.log", std::process::id()));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
            .open(&path)
            .unwrap();
        logs::init(Sink::File(file));
        path
    })
}

/// 目前的 log 全文；其他執行緒可能正寫到多位元組字元的一半，以 lossy 解碼而不 panic。
pub fn log_text() -> String {
    String::from_utf8_lossy(&std::fs::read(capture_logs()).unwrap()).into_owned()
}

/// log 中含 `marker` 的行（各測試以自己的標記篩選）。
pub fn log_lines_with(marker: &str) -> Vec<String> {
    log_text()
        .lines()
        .filter(|l| l.contains(marker))
        .map(str::to_owned)
        .collect()
}

/// log 中是否有以 `line` 結尾的一行（略過時間戳，比對整行內容）。
pub fn logged(line: &str) -> bool {
    log_lines_with(line).iter().any(|l| l.ends_with(line))
}
