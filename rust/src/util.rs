//! 跨模組共用的小工具：可替換的時鐘、限時 join、panic 訊息擷取與不怕 poison 的上鎖。

use std::any::Any;
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// 等執行緒結束時的輪詢間隔（std 的 join 沒有逾時）。
const JOIN_POLL: Duration = Duration::from_millis(10);

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

/// 等執行緒結束到 `deadline` 為止：結束了就 join 並回 true，逾時回 false（執行緒留著不管）。
pub fn join_until(thread: JoinHandle<()>, deadline: Instant, clock: &Clock) -> bool {
    loop {
        if thread.is_finished() {
            let _ = thread.join();
            return true;
        }
        let remaining = deadline.saturating_duration_since((clock.now)());
        if remaining.is_zero() {
            return false;
        }
        (clock.sleep)(remaining.min(JOIN_POLL));
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

/// 上鎖；持有者 panic 過（poison）也照樣拿到內容。
pub fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 取讀鎖；poison 也照樣拿到內容。
pub fn read<T: ?Sized>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}

/// 取寫鎖；poison 也照樣拿到內容。
pub fn write<T: ?Sized>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}
