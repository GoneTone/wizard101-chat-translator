//! 持久化 hook 的還原狀態：髒退出（硬砍／當機）後，下次對同一個仍在執行的遊戲程序啟動時自動修復殘留的 hook。
//!
//! 狀態＝依序寫回的（位址，原始 bytes）＋當時的模組基址，依 PID 存成 `hookstate-<pid>.json`。
//! PID 可能被重用給新的遊戲程序（ASLR 後基址不同），故基址不符即視為過期不套用。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::hooks::MODULE;
use super::memory::ProcessMemory;
use crate::logs::log;
use crate::util::{hex, write_atomic};

/// 一組依序寫回的還原操作（位址，原始 bytes）。
pub type RestoreOps = Vec<(u64, Vec<u8>)>;

fn state_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("hookstate-{pid}.json"))
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// 存還原操作與模組基址；`dir` 為 state 目錄（實機用 `config::local_state_dir()`）。
pub fn save_state(dir: &Path, pid: u32, base: u64, ops: &[(u64, Vec<u8>)]) -> io::Result<()> {
    let ops: Vec<Value> = ops
        .iter()
        .map(|(addr, b)| json!({ "addr": addr, "bytes": hex(b) }))
        .collect();
    write_atomic(
        &state_path(dir, pid),
        &json!({ "base": base, "ops": ops }).to_string(),
    )
}

fn parse(text: &str) -> Result<(u64, RestoreOps), String> {
    let data: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let bad = || "malformed hook state".to_owned();
    let ops = data["ops"]
        .as_array()
        .ok_or_else(bad)?
        .iter()
        .map(|e| Some((e["addr"].as_u64()?, unhex(e["bytes"].as_str()?)?)))
        .collect::<Option<_>>()
        .ok_or_else(bad)?;
    Ok((data["base"].as_u64().ok_or_else(bad)?, ops))
}

/// 讀還原操作，回傳 `(base, ops)`；無檔或壞檔回 `(None, [])`。
pub fn load_state(dir: &Path, pid: u32) -> (Option<u64>, RestoreOps) {
    let path = state_path(dir, pid);
    let text = match fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (None, Vec::new()),
        Err(e) => Err(e.to_string()),
    };
    match text.and_then(|t| parse(&t)) {
        Ok((base, ops)) => (Some(base), ops),
        Err(err) => {
            log(format!(
                "[reader] hook state unreadable, leaked hooks cannot be repaired \
                 (pid={pid}, file={}): {err}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            (None, Vec::new())
        }
    }
}

/// 刪除該 PID 的狀態檔（乾淨關閉、或修復完成後呼叫）。
pub fn clear_state(dir: &Path, pid: u32) {
    match fs::remove_file(state_path(dir, pid)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            log(format!(
                "[reader] could not delete hook state (pid={pid}): {e}"
            ));
        }
        _ => {}
    }
}

/// 刪除已不在執行的 PID 的殘留狀態檔。
pub fn sweep(dir: &Path, is_alive: impl Fn(u32) -> bool) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|n| n.strip_prefix("hookstate-")?.strip_suffix(".json"))
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        if !is_alive(pid)
            && let Err(e) = fs::remove_file(entry.path())
            && e.kind() != io::ErrorKind::NotFound
        {
            log(format!(
                "[reader] could not delete stale hook state {}: {e}",
                name.to_string_lossy()
            ));
        }
    }
}

/// 若有上次髒退出遺留的 hook，模組基址相同時依檔案順序寫回原始 bytes；套用或過期都刪檔。
///
/// 回傳成功寫回的筆數。
pub fn repair(dir: &Path, mem: &dyn ProcessMemory, pid: u32, slot: usize) -> usize {
    let (saved_base, ops) = load_state(dir, pid);
    if ops.is_empty() {
        return 0;
    }
    let current_base = match mem.module_base(MODULE) {
        Ok(base) => Some(base),
        Err(e) => {
            log(format!(
                "[reader] cannot read module base, ignoring hook state \
                 (slot={slot}, pid={pid}): {e}"
            ));
            None
        }
    };
    let mut written = 0;
    if current_base.is_some() && current_base == saved_base {
        let mut failed = 0;
        for (addr, original) in &ops {
            match mem.write(*addr, original) {
                Ok(()) => written += 1,
                Err(e) => {
                    failed += 1;
                    log(format!(
                        "[reader] hook repair write failed at {addr:#x} \
                         (slot={slot}, len={}): {e}",
                        original.len()
                    ));
                }
            }
        }
        let outcome = if failed == 0 {
            "no game restart needed"
        } else {
            "restart the game if chat stays silent"
        };
        log(format!(
            "[reader] repaired hooks leaked by previous dirty exit \
             (writes={written}, failed={failed}, slot={slot}, pid={pid}), {outcome}"
        ));
    } else if let (Some(current), Some(saved)) = (current_base, saved_base) {
        log(format!(
            "[reader] stale hook state ignored (slot={slot}, pid={pid} \
             was reused: saved_base={saved:#x}, current_base={current:#x})"
        ));
    }
    clear_state(dir, pid);
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tmp();
        let ops = vec![
            (0x1_4000_0000, vec![0x48, 0x89, 0x5c, 0x24]),
            (0x1_4123_4567, vec![0xe9, 0x00, 0x11, 0x22, 0x33, 0x44]),
        ];
        save_state(dir.path(), 4321, 0x1_4000_0000, &ops).unwrap();
        assert_eq!(load_state(dir.path(), 4321), (Some(0x1_4000_0000), ops));
    }

    #[test]
    fn load_missing_returns_empty() {
        assert_eq!(load_state(tmp().path(), 9999), (None, vec![]));
    }

    #[test]
    fn clear_removes_file_and_missing_is_noop() {
        let dir = tmp();
        clear_state(dir.path(), 4321);
        save_state(dir.path(), 4321, 0x1000, &[(0x2000, vec![0xab])]).unwrap();
        clear_state(dir.path(), 4321);
        assert_eq!(load_state(dir.path(), 4321), (None, vec![]));
    }

    #[test]
    fn empty_ops_roundtrip() {
        let dir = tmp();
        save_state(dir.path(), 4321, 0x1000, &[]).unwrap();
        assert_eq!(load_state(dir.path(), 4321), (Some(0x1000), vec![]));
    }

    #[test]
    fn reads_uppercase_hex() {
        assert_eq!(unhex("0aFF"), Some(vec![0x0a, 0xff]));
        assert_eq!(hex(&[0x0a, 0xff]), "0aff");
    }
}
