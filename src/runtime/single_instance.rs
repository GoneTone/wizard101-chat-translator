//! 單一實例（命名 mutex）、喚起既有視窗與管理員權限檢查。

use std::ptr::null;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM,
};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::IsUserAnAdmin;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextW, IsWindowVisible, SW_RESTORE, ShowWindow,
};

use crate::logs::log;
use crate::py::pyrepr::repr_str;
use crate::ui::win32::force_foreground;

/// 第二份實例會搶著對遊戲掛 hook，也會同時寫同一份 config.json 與 log。
const SINGLE_INSTANCE_MUTEX: &str = "wizard101-chat-translator.single-instance";

/// 持有單一實例 mutex；Drop 時關閉 handle 而釋放。
#[derive(Debug)]
pub struct InstanceLock(HANDLE);

impl Drop for InstanceLock {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// 搶不到單一實例 mutex 的原因。
#[derive(Debug, PartialEq, Eq)]
pub enum AcquireError {
    AlreadyRunning,
    /// `CreateMutexW` 本身失敗（Win32 錯誤碼）。
    CreateFailed(u32),
}

/// 搶下單一實例的 mutex。呼叫端須留住回傳值直到程序結束。
pub fn acquire_single_instance() -> Result<InstanceLock, AcquireError> {
    acquire_named(SINGLE_INSTANCE_MUTEX)
}

/// 建立 mutex 失敗的錯誤碼分類：拒絕存取代表較高權限的那份已持有它。
fn create_error(code: u32) -> AcquireError {
    if code == ERROR_ACCESS_DENIED {
        AcquireError::AlreadyRunning
    } else {
        AcquireError::CreateFailed(code)
    }
}

fn acquire_named(name: &str) -> Result<InstanceLock, AcquireError> {
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let handle = unsafe { CreateMutexW(null(), 0, wide.as_ptr()) };
    if handle.is_null() {
        return Err(create_error(unsafe { GetLastError() }));
    }
    // 必須緊接在 CreateMutexW 之後讀，中間不能插入其他 Win32 呼叫
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe { CloseHandle(handle) };
        return Err(AcquireError::AlreadyRunning);
    }
    Ok(InstanceLock(handle))
}

/// `buf` 前 `n` 個字元是否與 `title` 完全相同（`buf` 比 `title` 至少長 2，截斷會讓 `n` 變大）。
fn is_exact_title(buf: &[u16], n: usize, title: &[u16]) -> bool {
    n == title.len() && buf[..n] == *title
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
    let (title, found) = unsafe { &mut *(lparam as *mut (Vec<u16>, Vec<HWND>)) };
    if unsafe { IsWindowVisible(hwnd) } != 0 {
        // 多留一格才分得出「剛好等長」與「被截斷的更長標題」
        let mut buf = vec![0u16; title.len() + 2];
        let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        if is_exact_title(&buf, n.max(0) as usize, title) {
            found.push(hwnd);
        }
    }
    1
}

/// 把既有實例的視窗帶到前景；找不到標題完全相同的可見視窗時回 `false`。
pub fn focus_running_instance(title: &str) -> bool {
    let mut ctx = (
        title.encode_utf16().collect::<Vec<u16>>(),
        Vec::<HWND>::new(),
    );
    if unsafe { EnumWindows(Some(collect), &mut ctx as *mut _ as LPARAM) } == 0 {
        log(format!("[app] window scan failed: error={}", unsafe {
            GetLastError()
        }));
        return false;
    }
    let Some(&hwnd) = ctx.1.first() else {
        log(format!(
            "[app] no window titled {} to focus",
            repr_str(title)
        ));
        return false;
    };
    unsafe { ShowWindow(hwnd, SW_RESTORE) };
    force_foreground(hwnd as isize);
    true
}

/// 本程序是否以系統管理員權限執行。
pub fn is_elevated() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique(tag: &str) -> String {
        format!(
            "wizard101-chat-translator.test.{tag}.{}",
            std::process::id()
        )
    }

    #[test]
    fn second_acquire_fails_while_first_held() {
        let name = unique("held");
        let first = acquire_named(&name);
        assert!(first.is_ok());
        assert_eq!(
            acquire_named(&name).unwrap_err(),
            AcquireError::AlreadyRunning
        );
    }

    #[test]
    fn access_denied_means_an_elevated_copy_holds_the_mutex() {
        assert_eq!(
            create_error(ERROR_ACCESS_DENIED),
            AcquireError::AlreadyRunning
        );
        assert_eq!(create_error(8), AcquireError::CreateFailed(8));
    }

    #[test]
    fn title_must_match_exactly() {
        let title: Vec<u16> = "App".encode_utf16().collect();
        let read = |s: &str| {
            let mut buf = vec![0u16; title.len() + 2];
            let w: Vec<u16> = s.encode_utf16().take(buf.len() - 1).collect();
            buf[..w.len()].copy_from_slice(&w);
            (buf, w.len())
        };
        let check = |s: &str| {
            let (buf, n) = read(s);
            is_exact_title(&buf, n, &title)
        };
        assert!(check("App"));
        assert!(!check("App - settings"));
        assert!(!check("Ap"));
        assert!(!check("Apx"));
    }

    #[test]
    fn released_on_drop() {
        let name = unique("drop");
        drop(acquire_named(&name).unwrap());
        assert!(acquire_named(&name).is_ok());
    }

    #[test]
    fn focus_reports_failure_when_no_window_matches() {
        assert!(!focus_running_instance(&unique("no-such-window")));
    }
}
