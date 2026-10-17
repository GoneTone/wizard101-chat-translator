//! 遊戲視窗與程序的辨識：列舉客戶端視窗、PID、前景視窗與 client 區座標。

use std::path::Path;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::System::ProcessStatus::{EnumProcesses, GetModuleFileNameExW};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetClientRect, GetForegroundWindow, GetWindowThreadProcessId,
    IsWindow, IsWindowVisible,
};

use crate::logs::log;

/// 遊戲主程式的檔名。
pub const PROCESS_NAME: &str = "WizardGraphicalClient.exe";
const GAME_WINDOW_CLASS: &str = "Wizard Graphical Client";

/// exe 路徑是否為遊戲主程式（依檔名比對，不分大小寫）。
pub fn is_game_process_path(path: &str) -> bool {
    path.to_lowercase().ends_with(&PROCESS_NAME.to_lowercase())
}

/// PID 的主模組路徑；開不了程序（權限、已結束）回 None。
pub fn process_exe_path(pid: u32) -> Option<String> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let mut buf = vec![0u16; 1024];
    let n = unsafe { GetModuleFileNameExW(handle, null_mut(), buf.as_mut_ptr(), buf.len() as u32) };
    unsafe { CloseHandle(handle) };
    (n != 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
}

/// exe 路徑是遊戲主程式時回其安裝根目錄（`...\Bin\` 的上一層），否則 None。
pub fn install_root(exe: &str) -> Option<String> {
    if !is_game_process_path(exe) {
        return None;
    }
    Some(
        Path::new(exe)
            .parent()?
            .parent()?
            .to_string_lossy()
            .into_owned(),
    )
}

/// 遊戲程序的安裝根目錄（只用於診斷 log）；開不了程序或不是遊戲回 None。
pub fn install_path_of(pid: u32) -> Option<String> {
    install_root(&process_exe_path(pid)?)
}

unsafe extern "system" fn collect_game_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    let found = unsafe { &mut *(lparam as *mut Vec<isize>) };
    let target: Vec<u16> = GAME_WINDOW_CLASS.encode_utf16().collect();
    // 與 wizwalker 相同只讀前 23 字元：前綴相同的較長類別名稱也算
    let mut buf = [0u16; 24];
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if buf[..n.max(0) as usize] == target[..] {
        found.push(hwnd as isize);
    }
    1
}

unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    unsafe { &mut *(lparam as *mut Vec<isize>) }.push(hwnd as isize);
    1
}

/// `hwnds` 中第一個可見、且 exe 是遊戲主程式的視窗；`exe_of` 回 `None`（查詢失敗）就跳過該視窗。
pub fn first_game_window(
    hwnds: &[isize],
    is_visible: impl Fn(isize) -> bool,
    exe_of: impl Fn(isize) -> Option<String>,
) -> Option<isize> {
    hwnds.iter().copied().find(|&hwnd| {
        is_visible(hwnd) && exe_of(hwnd).is_some_and(|exe| is_game_process_path(&exe))
    })
}

/// 可見、且屬於遊戲程序的第一個頂層視窗；找不到回 `None`。
///
/// 給 ⛶ 按鈕用：按下時遊戲多半不在前景，不能像熱鍵那樣讀前景視窗。
pub fn find_game_window() -> Option<isize> {
    let mut hwnds = Vec::<isize>::new();
    if unsafe { EnumWindows(Some(collect_window), &mut hwnds as *mut _ as LPARAM) } == 0 {
        // 使用者從 app.log 才分得出「遊戲沒開」與「列舉本身失敗」
        log(format!(
            "[reader] window scan failed: {}",
            std::io::Error::last_os_error()
        ));
        return None;
    }
    first_game_window(
        &hwnds,
        |hwnd| unsafe { IsWindowVisible(hwnd as HWND) } != 0,
        |hwnd| process_exe_path(window_pid(hwnd)),
    )
}

/// 目前所有遊戲客戶端的視窗 handle（以視窗類別名辨識，同 wizwalker）。
pub fn game_windows() -> Vec<isize> {
    let mut found = Vec::<isize>::new();
    unsafe { EnumWindows(Some(collect_game_window), &mut found as *mut _ as LPARAM) };
    found
}

/// 視窗 handle 是否仍指向一個存在的視窗（客戶端關掉後為 `false`）。
pub fn window_exists(hwnd: isize) -> bool {
    unsafe { IsWindow(hwnd as HWND) != 0 }
}

/// 視窗所屬程序的 PID；視窗不存在時回 0。
pub fn window_pid(hwnd: isize) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd as HWND, &mut pid) };
    pid
}

/// PID 是否仍在執行；判斷不了就當活著，不誤刪殘留狀態檔。
pub fn pid_alive(pid: u32) -> bool {
    let mut pids = vec![0u32; 1024];
    loop {
        let bytes = (pids.len() * size_of::<u32>()) as u32;
        let mut needed = 0u32;
        if unsafe { EnumProcesses(pids.as_mut_ptr(), bytes, &mut needed) } == 0 {
            return true;
        }
        // 緩衝被填滿時無從得知是否還有更多，加大重來
        if needed < bytes {
            return pids[..needed as usize / size_of::<u32>()].contains(&pid);
        }
        pids.resize(pids.len() * 2, 0);
    }
}

/// 目前前景視窗的 handle，取不到回 0。
pub fn foreground_window() -> isize {
    unsafe { GetForegroundWindow() as isize }
}

/// 把視窗 client 區座標換成螢幕座標；視窗不存在時回 `None`。
pub fn client_to_screen(hwnd: isize, x: i32, y: i32) -> Option<(i32, i32)> {
    let mut point = POINT { x, y };
    (unsafe { ClientToScreen(hwnd as HWND, &mut point) } != 0).then_some((point.x, point.y))
}

/// 視窗 client 區的寬高（像素）；視窗不存在時回 `None`。
pub fn client_size(hwnd: isize) -> Option<(i32, i32)> {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    (unsafe { GetClientRect(hwnd as HWND, &mut rect) } != 0).then_some((rect.right, rect.bottom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_process_path_matches_file_name_case_insensitively() {
        assert!(is_game_process_path(
            r"C:\Games\Wizard101\Bin\WizardGraphicalClient.exe"
        ));
        assert!(is_game_process_path(
            r"d:\w101\bin\wizardgraphicalclient.EXE"
        ));
        assert!(!is_game_process_path(
            r"C:\Games\Wizard101\Bin\WizardGraphicalClient.exe.bak"
        ));
        assert!(!is_game_process_path(""));
    }

    #[test]
    fn process_queries_handle_missing_targets() {
        assert!(pid_alive(std::process::id()));
        assert!(!window_exists(0));
        assert_eq!(window_pid(0), 0);
        assert_eq!(client_size(0), None);
        assert_eq!(client_to_screen(0, 0, 0), None);
    }

    const GAME: &str = r"C:\Games\Wizard101\Bin\WizardGraphicalClient.exe";
    const FIREFOX: &str = r"C:\Program Files\Mozilla Firefox\firefox.exe";

    #[test]
    fn find_game_window_returns_the_first_matching_visible_window() {
        let exe = |hwnd| Some(if hwnd == 1 { FIREFOX } else { GAME }.to_owned());
        assert_eq!(first_game_window(&[1, 2, 3], |_| true, exe), Some(2));
    }

    #[test]
    fn find_game_window_returns_none_when_the_game_is_not_running() {
        let exe = |_| Some(FIREFOX.to_owned());
        assert_eq!(first_game_window(&[1, 2], |_| true, exe), None);
    }

    #[test]
    fn find_game_window_skips_invisible_windows() {
        let exe = |_| Some(GAME.to_owned());
        assert_eq!(first_game_window(&[1], |_| false, exe), None);
    }

    #[test]
    fn find_game_window_ignores_a_single_window_query_failure() {
        let exe = |hwnd| (hwnd != 1).then(|| GAME.to_owned());
        assert_eq!(first_game_window(&[1, 2], |_| true, exe), Some(2));
    }
}
