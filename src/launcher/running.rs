//! 執行中帳號偵測：讀遊戲程序的命令列，取出 `-U` 末段的帳號名。

use std::sync::Mutex;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::UI::Shell::CommandLineToArgvW;

use crate::game::process::{
    is_game_process_path, process_command_line, process_exe_path, process_ids,
};
use crate::logs::log;
use crate::util::lock;

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

/// 從遊戲命令列取出帳號名（`-U ..{UserID} {ck2} {帳號}` 的第三段）；沒有就回 `None`，絕不回傳 ck2。
pub fn account_in_command_line(cmdline: &str) -> Option<String> {
    let args = split_args(cmdline);
    let u = args.iter().position(|a| a == "-U")?;
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
    }
}
