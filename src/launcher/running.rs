//! 執行中程序偵測：從遊戲命令列取出帳號名，並判斷某份安裝是否有程式開著。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::UI::Shell::CommandLineToArgvW;

use crate::game::process::{
    PROCESS_NAME, is_game_process_path, process_command_line, process_exe_path, process_ids,
};
use crate::logs::log;
use crate::util::lock;

/// 介面開著時重新掃描執行中帳號的間隔（秒）；掃描要列舉程序，不能每幀做。
pub const RUNNING_REFRESH_SECS: f64 = 3.0;

/// 官方啟動器的執行檔名。
const LAUNCHER_NAME: &str = "WizardLauncher.exe";

/// 依用戶端的切法拆命令列。
fn split_args(cmdline: &str) -> Vec<String> {
    // 空字串會被 CommandLineToArgvW 換成本程式路徑
    if cmdline.trim().is_empty() {
        return Vec::new();
    }
    let wide: Vec<u16> = cmdline.encode_utf16().chain([0]).collect();
    let mut count = 0i32;
    // SAFETY：wide 以 NUL 結尾；回傳的陣列以 LocalFree 一次釋放。
    unsafe {
        let argv = CommandLineToArgvW(wide.as_ptr(), &mut count);
        if argv.is_null() {
            return Vec::new();
        }
        let args = (0..count as usize)
            .map(|i| {
                let p = *argv.add(i);
                let len = (0..).take_while(|&n| *p.add(n) != 0).count();
                String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
            })
            .collect();
        LocalFree(argv.cast());
        args
    }
}

/// `exes` 裡是否有 `root` 底下的遊戲用戶端或官方啟動器（不分大小寫）。
fn busy_paths(root: &Path, exes: &[String]) -> bool {
    let root = PathBuf::from(root.to_string_lossy().to_lowercase());
    let names = [PROCESS_NAME, LAUNCHER_NAME].map(str::to_lowercase);
    exes.iter().any(|exe| {
        // Path::starts_with 比整段路徑元件，`Wizard101-Test` 不會被當成 `Wizard101` 底下
        let exe = PathBuf::from(exe.to_lowercase());
        exe.starts_with(&root)
            && exe
                .file_name()
                .is_some_and(|n| names.iter().any(|name| n == name.as_str()))
    })
}

/// `root` 底下是否有遊戲用戶端或官方啟動器在執行。
pub fn install_busy(root: &Path) -> bool {
    let exes: Vec<String> = process_ids()
        .unwrap_or_default()
        .into_iter()
        .filter_map(process_exe_path)
        .collect();
    let busy = busy_paths(root, &exes);
    log(format!(
        "[patch] install busy check: root={} processes={} busy={busy}",
        root.display(),
        exes.len()
    ));
    busy
}

/// 從遊戲命令列取出帳號名（`-U ..{UserID} {ck2} {帳號}` 的第三段）；沒有就回 `None`，絕不回傳 ck2。
pub fn account_in_command_line(cmdline: &str) -> Option<String> {
    let args = split_args(cmdline);
    let u = args.iter().position(|a| a == "-U")?;
    // 其他啟動器參數順序可能不同，UserID 段不對就不當帳號名
    if !args.get(u + 1)?.starts_with("..") {
        return None;
    }
    args.get(u + 3).cloned()
}

/// 目前執行中遊戲用戶端的帳號名（不論是誰開的）。
pub fn running_accounts() -> Vec<String> {
    let names: Vec<String> = process_ids()
        .unwrap_or_default()
        .into_iter()
        .filter(|&pid| process_exe_path(pid).is_some_and(|p| is_game_process_path(&p)))
        .filter_map(|pid| account_in_command_line(&process_command_line(pid)?))
        .collect();
    // 設定頁會定時輪詢，只在清單變動時記錄
    static LAST: Mutex<Option<Vec<String>>> = Mutex::new(None);
    let mut last = lock(&LAST);
    if last.as_ref() != Some(&names) {
        log(format!(
            "[launch] {} running account(s): {names:?}",
            names.len()
        ));
        *last = Some(names.clone());
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ck2() -> String {
        "Ab+/".repeat(22)
    }

    fn line() -> String {
        format!(
            r#""D:\Games\Wizard101\Bin\WizardGraphicalClient.exe" -L login.us.wizard101.com 12000 -A en-US -ST -P 0 -PT 0 -U ..1234567890123 {} alice"#,
            ck2()
        )
    }

    #[test]
    fn install_busy_matches_client_and_launcher_under_root() {
        let root = Path::new(r"D:\Games\Wizard101");
        let busy = |exe: &str| busy_paths(root, &[exe.to_string()]);
        assert!(busy(r"D:\Games\Wizard101\Bin\WizardGraphicalClient.exe"));
        assert!(busy(r"d:\games\WIZARD101\bin\wizardgraphicalclient.EXE"));
        assert!(busy(r"D:\Games\Wizard101\WizardLauncher.exe"));
        assert!(busy(r"D:\Games\Wizard101\PatchClient\WizardLauncher.exe"));
        assert!(!busy(r"D:\Games\Wizard101\Bin\Other.exe"));
        assert!(!busy(
            r"D:\Games\Wizard101-Test\Bin\WizardGraphicalClient.exe"
        ));
        assert!(!busy(r"C:\Steam\Wizard101\Bin\WizardGraphicalClient.exe"));
        assert!(busy_paths(
            Path::new("c:/program files (x86)/steam/steamapps/common/Wizard101"),
            &[
                r"C:\Windows\explorer.exe".into(),
                r"C:\Program Files (x86)\Steam\steamapps\common\Wizard101\Bin\WizardGraphicalClient.exe".into(),
            ]
        ));
        assert!(!busy_paths(root, &[]));
    }

    #[test]
    fn account_in_command_line_extracts_name() {
        assert_eq!(account_in_command_line(&line()).as_deref(), Some("alice"));
    }

    #[test]
    fn account_in_command_line_ignores_ck2() {
        let name = account_in_command_line(&line()).unwrap();
        assert_ne!(name, ck2());
        assert!(!name.contains(&ck2()));
    }

    #[test]
    fn account_in_command_line_handles_missing_u() {
        assert_eq!(
            account_in_command_line(
                r#""C:\W\Bin\WizardGraphicalClient.exe" -L login.us.wizard101.com 12000"#
            ),
            None
        );
        assert_eq!(account_in_command_line("x.exe -U ..1 ck2"), None);
        assert_eq!(account_in_command_line(""), None);
        assert_eq!(
            account_in_command_line("x.exe -U alice 1234 ck2 extra"),
            None
        );
    }
}
