//! 視窗的 Win32 補強：取 HWND、點擊不奪焦點、前景切換與螢幕查詢。

use std::sync::atomic::{AtomicIsize, Ordering};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint,
};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetCursorPos, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, IsWindow, SWP_NOACTIVATE, SWP_NOZORDER, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, WS_EX_NOACTIVATE,
};

use crate::logs::log;
pub use crate::reader::process::foreground_window;

pub use crate::ui::geometry::Rect;

/// 視窗的 HWND；不是 Win32 視窗或取不到時回 0。
pub fn hwnd_of(window: &impl HasWindowHandle) -> isize {
    match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => 0,
    }
}

/// 讀出視窗的 extended style。
pub fn ex_style(hwnd: isize) -> u32 {
    unsafe { GetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE) as u32 }
}

/// 讓視窗接得到滑鼠但點擊不奪焦點（`WS_EX_NOACTIVATE`）；已設好時不動。
///
/// winit 在顯示／隱藏等狀態變更時會整個重寫 extended style，所以呼叫端要在每次變更後再補一次。
pub fn make_non_activating(hwnd: isize) {
    let style = ex_style(hwnd);
    if style & WS_EX_NOACTIVATE != 0 {
        return;
    }
    unsafe {
        SetWindowLongPtrW(
            hwnd as HWND,
            GWL_EXSTYLE,
            (style | WS_EX_NOACTIVATE) as isize,
        )
    };
    // 每幀都會重試，同一個視窗只記一次
    static FAILED: AtomicIsize = AtomicIsize::new(0);
    if ex_style(hwnd) & WS_EX_NOACTIVATE == 0 && FAILED.swap(hwnd, Ordering::Relaxed) != hwnd {
        log(format!("[ui] non-activating setup failed: hwnd={hwnd:#x}"));
    }
}

/// 掛上目前前景視窗的執行緒再切前景，才繞得過前景鎖；視窗不存在或切換失敗回 `false`。
pub fn force_foreground(hwnd: isize) -> bool {
    let hwnd = hwnd as HWND;
    unsafe {
        if IsWindow(hwnd) == 0 {
            return false;
        }
        let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
        let this_thread = GetCurrentThreadId();
        AttachThreadInput(this_thread, fg_thread, 1);
        let switched = SetForegroundWindow(hwnd) != 0;
        AttachThreadInput(this_thread, fg_thread, 0);
        switched
    }
}

/// 游標的螢幕座標（實體像素）。
pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

/// 視窗外框矩形（實體像素）；取不到時全為 0。
pub fn window_rect(hwnd: isize) -> Rect {
    let mut r: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetWindowRect(hwnd as HWND, &mut r) };
    to_rect(r)
}

/// 以實體像素擺放視窗，不改疊序、不啟用；失敗（如 HWND 無效）回 `false`。
pub fn set_window_rect(hwnd: isize, (x, y, w, h): Rect) -> bool {
    let place = || unsafe {
        SetWindowPos(
            hwnd as HWND,
            std::ptr::null_mut(),
            x,
            y,
            w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
    if place() == 0 {
        return false;
    }
    // 移到不同 DPI 的螢幕時 winit 會按邏輯尺寸改大小，再擺一次才維持實體尺寸
    if window_rect(hwnd) != (x, y, w, h) {
        place();
    }
    true
}

fn to_rect(r: RECT) -> Rect {
    (r.left, r.top, r.right - r.left, r.bottom - r.top)
}

fn monitor_info(x: i32, y: i32) -> MONITORINFO {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        GetMonitorInfoW(monitor, &mut info);
    }
    info
}

/// 含 (x, y) 那顆螢幕的工作區（去掉工作列）。
pub fn work_area_at(x: i32, y: i32) -> Rect {
    to_rect(monitor_info(x, y).rcWork)
}

/// 含 (x, y) 那顆螢幕的整個矩形（含工作列）。
pub fn monitor_rect_at(x: i32, y: i32) -> Rect {
    to_rect(monitor_info(x, y).rcMonitor)
}

unsafe extern "system" fn collect_monitor(
    _monitor: HMONITOR,
    _hdc: HDC,
    rect: *mut RECT,
    lparam: LPARAM,
) -> i32 {
    let found = unsafe { &mut *(lparam as *mut Vec<Rect>) };
    found.push(to_rect(unsafe { *rect }));
    1
}

/// 所有螢幕的整個矩形。
pub fn all_monitors() -> Vec<Rect> {
    let mut found = Vec::<Rect>::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect_monitor),
            &mut found as *mut _ as LPARAM,
        )
    };
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_area_lies_within_its_monitor() {
        let (mx, my, mw, mh) = monitor_rect_at(0, 0);
        let (wx, wy, ww, wh) = work_area_at(0, 0);
        assert!(mw > 0 && mh > 0);
        assert!(wx >= mx && wy >= my && wx + ww <= mx + mw && wy + wh <= my + mh);
    }

    #[test]
    fn all_monitors_includes_the_nearest_one() {
        assert!(all_monitors().contains(&monitor_rect_at(0, 0)));
    }
}
