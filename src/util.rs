//! 跨模組共用的小工具：可替換的時鐘、限時 join、panic 訊息擷取、不怕 poison 的上鎖、內容指紋、原子寫檔與殘檔清掃。

use std::any::Any;
use std::fmt::Write as _;
use std::fs;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use sha1::{Digest, Sha1};

use crate::logs::log;

/// 等執行緒結束時的輪詢間隔（std 的 join 沒有逾時）。
const JOIN_POLL: Duration = Duration::from_millis(10);

/// 等待與計時用的時鐘；測試換成假的就不必真的等。
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

/// 依序串接 `parts` 的 sha1 前 12 碼（小寫 hex），當內容指紋用。
pub fn sha1_head(parts: &[&[u8]]) -> String {
    let mut hasher = Sha1::new();
    for part in parts {
        hasher.update(part);
    }
    hex(&hasher.finalize())[..12].to_string()
}

/// 位元組轉小寫 hex 字串。
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// 一個隨機的 `u64`：借用標準庫每次不同種子的雜湊，只求分散與不撞，不需密碼學強度。
pub fn random_u64() -> u64 {
    RandomState::new().build_hasher().finish()
}

/// 先寫同目錄暫存檔再 rename，讀者永遠看不到寫到一半的檔案；會建父資料夾，失敗時清掉暫存檔。
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp_name = path.as_os_str().to_owned();
    // 每次呼叫獨立的暫存檔，重疊的寫入才不會互相截斷
    static SEQ: AtomicU64 = AtomicU64::new(0);
    tmp_name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = PathBuf::from(tmp_name);
    // 寫入與 rename 之間當機會留下暫存檔，下次啟動由 `sweep_stale_temps` 清掉
    let result = write_synced(&tmp, text).and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// 寫檔並 fsync：先落盤再 rename，斷電後才不會留下空檔。
fn write_synced(path: &Path, text: &str) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// 刪掉 `dir` 裡已結束程序留下的 `write_atomic` 暫存檔（`<file>.<pid>.<n>.tmp`）。
pub fn sweep_stale_temps(dir: &Path, is_alive: impl Fn(u32) -> bool) {
    remove_dead_pid_files(
        dir,
        temp_pid,
        is_alive,
        "[app] could not delete stale temp file",
    );
}

/// `<file>.<pid>.<n>.tmp` 裡的 pid；其他檔名回 `None`。
fn temp_pid(name: &str) -> Option<u32> {
    let mut parts = name.strip_suffix(".tmp")?.rsplitn(3, '.');
    parts.next()?.parse::<u64>().ok()?;
    let pid = parts.next()?.parse().ok()?;
    parts.next().filter(|file| !file.is_empty())?;
    Some(pid)
}

/// 刪掉 `dir` 裡 `pid_of` 認得出 pid、而且該程序已結束的檔案；刪不掉時以 `failure` 開頭記 log。
pub fn remove_dead_pid_files(
    dir: &Path,
    pid_of: impl Fn(&str) -> Option<u32>,
    is_alive: impl Fn(u32) -> bool,
    failure: &str,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(&pid_of) else {
            continue;
        };
        if !is_alive(pid)
            && let Err(e) = fs::remove_file(entry.path())
            && e.kind() != io::ErrorKind::NotFound
        {
            log(format!("{failure} {}: {e}", name.to_string_lossy()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_pid_reads_only_write_atomic_temp_names() {
        assert_eq!(temp_pid("config.json.4242.7.tmp"), Some(4242));
        assert_eq!(temp_pid("config.json"), None);
        assert_eq!(temp_pid("notes.tmp"), None);
        assert_eq!(temp_pid("4242.7.tmp"), None);
        assert_eq!(temp_pid("config.json.x.7.tmp"), None);
        assert_eq!(temp_pid("config.json.4242.x.tmp"), None);
    }

    #[test]
    fn sweep_removes_only_temps_of_dead_processes() {
        let dir = tempfile::tempdir().unwrap();
        let names = [
            "config.json",
            "config.json.111.0.tmp",
            "config.json.222.3.tmp",
            "other.tmp",
        ];
        for name in names {
            fs::write(dir.path().join(name), "x").unwrap();
        }
        sweep_stale_temps(dir.path(), |pid| pid == 111);
        let mut left: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, ["config.json", "config.json.111.0.tmp", "other.tmp"]);
    }

    #[test]
    fn write_atomic_leaves_no_temp_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_atomic(&path, "{}").unwrap();
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(names.len(), 1);
        assert_eq!(fs::read_to_string(path).unwrap(), "{}");
    }
}
