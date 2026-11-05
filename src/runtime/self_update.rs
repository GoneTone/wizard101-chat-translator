//! 自我更新的檔案與程序機制：替換執行中的 exe、清除殘留、結束時啟動新版。

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use crate::local::download;
use crate::logs::log;
use crate::util::{lock, with_suffix};

/// 新程序帶這個參數啟動，表示是更新後重啟，要等舊程序放掉單一實例鎖。
pub const AFTER_UPDATE_ARG: &str = "--after-update";

static RELAUNCH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// 下載完成的新版檔案位置。
pub fn new_path(exe: &Path) -> PathBuf {
    with_suffix(exe, ".new")
}

/// 新版下載中的半成品位置（與 `local::download` 的 `.part` 規則一致）。
pub fn part_path(exe: &Path) -> PathBuf {
    download::part_path(&new_path(exe))
}

/// 被換下的舊版檔案位置。
pub fn old_path(exe: &Path) -> PathBuf {
    with_suffix(exe, ".old")
}

/// 把 `.new` 換成執行中的 `exe`（舊檔改名留作 `.old`）；任一步失敗都還原 `exe`。
pub fn replace_exe(exe: &Path) -> io::Result<()> {
    let (new, old) = (new_path(exe), old_path(exe));
    match std::fs::remove_file(&old) {
        Ok(()) => log(format!("[update] removed stale {}", old.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            log(format!(
                "[update] cannot remove stale {}: {e}",
                old.display()
            ));
            return Err(e);
        }
    }
    std::fs::rename(exe, &old)
        .inspect_err(|e| log(format!("[update] cannot move {} aside: {e}", exe.display())))?;
    if let Err(e) = std::fs::rename(&new, exe) {
        log(format!(
            "[update] cannot move {} into place: {e}",
            new.display()
        ));
        if let Err(restore) = std::fs::rename(&old, exe) {
            log(format!(
                "[update] cannot restore {}: {restore}",
                exe.display()
            ));
        }
        return Err(e);
    }
    log(format!("[update] replaced {}", exe.display()));
    Ok(())
}

/// 刪掉上次更新留下的 `.old`、`.new`、`.new.part`；不存在略過。
pub fn remove_leftovers(exe: &Path) {
    for path in [old_path(exe), new_path(exe), part_path(exe)] {
        match std::fs::remove_file(&path) {
            Ok(()) => log(format!("[update] deleted leftover {}", path.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => log(format!(
                "[update] could not delete leftover {}: {e}",
                path.display()
            )),
        }
    }
}

/// 登記程序結束時要啟動的新版 exe。
pub fn request_relaunch(exe: PathBuf) {
    *lock(&RELAUNCH) = Some(exe);
}

/// 有登記就啟動新版（沿用目前權限）；沒登記什麼都不做。
pub fn relaunch_if_requested() {
    let Some(exe) = lock(&RELAUNCH).take() else {
        return;
    };
    match Command::new(&exe).arg(AFTER_UPDATE_ARG).spawn() {
        Ok(child) => log(format!(
            "[update] relaunched {} arg={AFTER_UPDATE_ARG} pid={}",
            exe.display(),
            child.id()
        )),
        Err(e) => log(format!("[update] cannot relaunch {}: {e}", exe.display())),
    }
}
