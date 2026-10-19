//! 系統剪貼簿讀取；寫入一律走 `egui::Context::copy_text`。

use arboard::{Clipboard, Error};

/// 讀取系統剪貼簿的文字；`Err` 是開不了剪貼簿，`Ok(None)` 是裡面沒有文字。
pub fn get_text() -> Result<Option<String>, String> {
    match Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
        Ok(text) => Ok(Some(text)),
        Err(Error::ContentNotAvailable) => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}
