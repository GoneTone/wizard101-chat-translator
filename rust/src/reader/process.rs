//! 遊戲視窗與程序的辨識：列舉客戶端視窗、PID、前景視窗與 client 區座標。

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::System::ProcessStatus::EnumProcesses;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetClientRect, GetForegroundWindow, GetWindowThreadProcessId,
    IsWindow,
};

/// 遊戲主程式的檔名。
pub const PROCESS_NAME: &str = "WizardGraphicalClient.exe";
const GAME_WINDOW_CLASS: &str = "Wizard Graphical Client";

/// exe 路徑是否為遊戲主程式（依檔名比對，不分大小寫）。
pub fn is_game_process_path(path: &str) -> bool {
    path.to_lowercase().ends_with(&PROCESS_NAME.to_lowercase())
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
}
