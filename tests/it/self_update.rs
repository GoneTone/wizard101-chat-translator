//! 自我更新：替換 exe、清除殘留、更新後等單一實例鎖。

use std::cell::Cell;
use std::fs::{self, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::Duration;

use tempfile::TempDir;
use wizard101_chat_translator::runtime::self_update::{
    new_path, old_path, part_path, remove_leftovers, replace_exe,
};
use wizard101_chat_translator::runtime::{AcquireError, acquire_waiting};

use crate::common::reader_fixture::VirtualClock;

fn exe_in(dir: &TempDir) -> PathBuf {
    let exe = dir.path().join("app.exe");
    fs::write(&exe, "old").unwrap();
    exe
}

#[test]
fn replace_swaps_in_the_new_exe_and_keeps_the_old_one() {
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    fs::write(new_path(&exe), "new").unwrap();
    replace_exe(&exe).unwrap();
    assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
    assert_eq!(fs::read_to_string(old_path(&exe)).unwrap(), "old");
    assert!(!new_path(&exe).exists());
}

#[test]
fn replace_overwrites_a_stale_old_file() {
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    fs::write(new_path(&exe), "new").unwrap();
    fs::write(old_path(&exe), "stale").unwrap();
    replace_exe(&exe).unwrap();
    assert_eq!(fs::read_to_string(old_path(&exe)).unwrap(), "old");
}

#[test]
fn replace_restores_the_exe_when_the_new_file_is_missing() {
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    assert!(replace_exe(&exe).is_err());
    assert_eq!(fs::read_to_string(&exe).unwrap(), "old");
    assert!(!old_path(&exe).exists());
}

#[test]
fn replace_fails_without_touching_exe_when_old_cannot_be_removed() {
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    fs::write(new_path(&exe), "new").unwrap();
    let _held = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .share_mode(0)
        .open(old_path(&exe))
        .unwrap();
    assert!(replace_exe(&exe).is_err());
    assert_eq!(fs::read_to_string(&exe).unwrap(), "old");
    assert_eq!(fs::read_to_string(new_path(&exe)).unwrap(), "new");
}

#[test]
fn remove_leftovers_deletes_all_three_and_ignores_missing() {
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    fs::write(old_path(&exe), "x").unwrap();
    fs::write(part_path(&exe), "x").unwrap();
    remove_leftovers(&exe);
    assert!(!old_path(&exe).exists() && !new_path(&exe).exists() && !part_path(&exe).exists());
    assert!(exe.exists());
}

#[test]
fn part_path_appends_part_to_the_new_path() {
    let exe = PathBuf::from("C:/x/app.exe");
    assert_eq!(part_path(&exe), PathBuf::from("C:/x/app.exe.new.part"));
}

#[test]
fn acquire_waiting_retries_until_the_lock_frees() {
    let clock = VirtualClock::new();
    let calls = Cell::new(0);
    let result = acquire_waiting(
        &clock.clock(Duration::ZERO),
        Duration::from_secs(10),
        || {
            calls.set(calls.get() + 1);
            if calls.get() < 3 {
                Err(AcquireError::AlreadyRunning)
            } else {
                Ok(7)
            }
        },
    );
    assert_eq!(result, Ok(7));
    assert_eq!(calls.get(), 3);
}

#[test]
fn acquire_waiting_gives_up_after_the_deadline() {
    let clock = VirtualClock::new();
    let result: Result<u8, _> =
        acquire_waiting(&clock.clock(Duration::ZERO), Duration::from_secs(1), || {
            Err(AcquireError::AlreadyRunning)
        });
    assert_eq!(result, Err(AcquireError::AlreadyRunning));
    assert!(clock.elapsed() >= Duration::from_secs(1));
}

#[test]
fn acquire_waiting_returns_create_failures_immediately() {
    let clock = VirtualClock::new();
    let calls = Cell::new(0);
    let result: Result<u8, _> = acquire_waiting(
        &clock.clock(Duration::ZERO),
        Duration::from_secs(10),
        || {
            calls.set(calls.get() + 1);
            Err(AcquireError::CreateFailed(5))
        },
    );
    assert_eq!(result, Err(AcquireError::CreateFailed(5)));
    assert_eq!(calls.get(), 1);
}
