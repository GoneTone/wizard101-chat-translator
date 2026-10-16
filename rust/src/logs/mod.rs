//! 診斷輸出：一行訊息加上 UTC 時戳後寫到目前的 sink（打包版為 exe 旁的 `app.log`）。
//!
//! 訊息一律英文、帶 `[模組]` 前綴，內容要有足夠 context；API 金鑰絕不可寫入。

mod files;

pub use files::{
    TimestampedWriter, open_session_log, session_header, trim_log_sessions, utc_stamp,
};

use std::fs::File;
use std::io::{self, Write};
use std::sync::Mutex;

/// `log` 的輸出目的地。
pub enum Sink {
    /// 標準錯誤輸出（未呼叫 [`init`] 前的預設）。
    Stderr,
    /// 以 CRLF 換行寫入的 log 檔。
    File(File),
}

static SINK: Mutex<Sink> = Mutex::new(Sink::Stderr);

/// 切換 `log` 的輸出目的地。
pub fn init(sink: Sink) {
    *SINK.lock().unwrap_or_else(|e| e.into_inner()) = sink;
}

/// 寫一行帶 UTC 時戳的訊息；整行一次寫出，多執行緒下不會互相切斷。
pub fn log(message: impl AsRef<str>) {
    let stamp = utc_stamp(chrono::Utc::now());
    let message = message.as_ref();
    let mut sink = SINK.lock().unwrap_or_else(|e| e.into_inner());
    // 寫入失敗無處可報，丟棄以免 log 本身帶掉程式
    let _ = match &mut *sink {
        Sink::Stderr => io::stderr().write_all(format!("{stamp} {message}\n").as_bytes()),
        Sink::File(file) => file.write_all(format!("{stamp} {message}\r\n").as_bytes()),
    };
}

/// 掛上 panic hook：以 `[app] panic: ...` 與 backtrace 寫進 log。
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        log(format!("[app] panic: {info}\n{backtrace}"));
    }));
}
