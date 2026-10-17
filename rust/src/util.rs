//! 跨模組共用的小工具：可替換的時鐘與 panic 訊息擷取。

use std::any::Any;
use std::time::{Duration, Instant};

/// 讀取器用的時鐘；測試換成假的就不必真的等。
pub struct Clock {
    /// 目前時刻。
    pub now: Box<dyn Fn() -> Instant + Send>,
    /// 睡指定的時間。
    pub sleep: Box<dyn Fn(Duration) + Send>,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            now: Box::new(Instant::now),
            sleep: Box::new(std::thread::sleep),
        }
    }
}

/// panic payload 裡的訊息（`&str` 或 `String`）；其他型別回 `"panic"`。
pub(crate) fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".to_owned())
}
