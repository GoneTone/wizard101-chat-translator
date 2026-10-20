//! 視窗的 Win32 補強：取 HWND、依標題找視窗、點擊不奪焦點、關掉重導向表面與系統外框、不上工作列、深色標題列、泡泡模式攔最小化、前景切換與螢幕查詢。

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Once, OnceLock};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_CAPTION_COLOR, DWMWA_CLOAK, DWMWA_TRANSITIONS_FORCEDISABLED,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DwmFlush, DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    IMAGE_DIRECTORY_ENTRY_IAT, IMAGE_NT_HEADERS64,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{PAGE_READWRITE, VirtualProtect};
use windows_sys::Win32::System::SystemServices::IMAGE_DOS_HEADER;
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::{RAWINPUTDEVICE, RIDEV_REMOVE, RegisterRawInputDevices};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumThreadWindows, GW_OWNER, GWL_EXSTYLE, GWLP_HWNDPARENT, GetCursorPos, GetForegroundWindow,
    GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, HMENU,
    IsWindow, IsWindowVisible, STYLESTRUCT, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, WM_NCCALCSIZE,
    WM_NCDESTROY, WM_STYLECHANGING, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
};
use windows_sys::core::PCWSTR;

use crate::logs::log;
pub use crate::reader::process::foreground_window;
use crate::ui::overlay::bubble::MinimizeGuard;
use crate::ui::palette;

pub use crate::ui::geometry::Rect;

/// HID usage page「Generic Desktop」與其中的鍵盤 usage。
const HID_USAGE_PAGE_GENERIC: u16 = 0x01;
const HID_USAGE_GENERIC_KEYBOARD: u16 = 0x06;

/// 取消本程序的鍵盤 raw input 註冊（`RIDEV_REMOVE` 必須搭配空的目標視窗）。
fn raw_keyboard_removal() -> RAWINPUTDEVICE {
    RAWINPUTDEVICE {
        usUsagePage: HID_USAGE_PAGE_GENERIC,
        usUsage: HID_USAGE_GENERIC_KEYBOARD,
        dwFlags: RIDEV_REMOVE,
        hwndTarget: std::ptr::null_mut(),
    }
}

/// 取消 winit 建立事件迴圈時註冊的鍵盤 raw input，滑鼠的保留；要在事件迴圈建立之後呼叫。
///
/// 註冊了鍵盤 raw input 時，本程序的視窗在前景期間 Windows 不呼叫我們的 `WH_KEYBOARD_LL` hook（錄熱鍵收不到鍵）。
/// winit 的鍵盤事件來自 `WM_KEYDOWN` 等一般訊息，raw input 只餵 `DeviceEvent::Key`，egui 用不到。
pub fn stop_raw_keyboard_input() {
    let device = raw_keyboard_removal();
    let size = std::mem::size_of::<RAWINPUTDEVICE>() as u32;
    // SAFETY：傳入一個區域變數的 RAWINPUTDEVICE，數量與大小相符。
    if unsafe { RegisterRawInputDevices(&device, 1, size) } != 0 {
        log(
            "[ui] raw keyboard input unregistered so the keyboard hook keeps receiving keys while our windows are focused",
        );
    } else {
        log(format!(
            "[ui] could not unregister raw keyboard input; the keyboard hook may miss keys while our windows are focused: {}",
            std::io::Error::last_os_error()
        ));
    }
}

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

unsafe extern "system" fn frameless_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    // 搶在 winit 之前：它對無邊框視窗把工作區頂端讓出 1px 給系統陰影
    if msg == WM_NCCALCSIZE && wparam != 0 {
        return 0;
    }
    if msg == WM_NCDESTROY {
        unsafe { RemoveWindowSubclass(hwnd, Some(frameless_proc), 0) };
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

unsafe extern "system" fn minimize_guard_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    let guard = data as *const MinimizeGuard;
    if unsafe { &*guard }.intercept(msg, wparam) {
        return 0;
    }
    if msg == WM_NCDESTROY {
        unsafe {
            RemoveWindowSubclass(hwnd, Some(minimize_guard_proc), 0);
            drop(Arc::from_raw(guard));
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// 讓 `guard` 先看過視窗收到的訊息，泡泡模式下的最小化交給它吞掉；每個視窗只呼叫一次。
pub fn guard_minimize(hwnd: isize, guard: Arc<MinimizeGuard>) {
    let data = Arc::into_raw(guard);
    if unsafe { SetWindowSubclass(hwnd as HWND, Some(minimize_guard_proc), 0, data as usize) } == 0
    {
        let error = std::io::Error::last_os_error();
        drop(unsafe { Arc::from_raw(data) });
        log(format!(
            "[ui] bubble minimize guard not installed: hwnd={hwnd:#x} error={error}"
        ));
        return;
    }
    log(format!(
        "[ui] bubble minimize guard installed: hwnd={hwnd:#x}"
    ));
}

/// 設定一個 4 位元組的 DWM 視窗屬性，回傳 HRESULT。
fn set_dwm_attribute(hwnd: isize, attribute: i32, value: u32) -> i32 {
    unsafe {
        DwmSetWindowAttribute(
            hwnd as HWND,
            attribute as u32,
            (&value as *const u32).cast(),
            size_of::<u32>() as u32,
        )
    }
}

/// 關掉 DWM 在視窗顯示與關閉時的淡入淡出動畫；失敗只記 log。
pub fn disable_transitions(hwnd: isize) {
    let hr = set_dwm_attribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, 1);
    if hr != 0 {
        log(format!(
            "[ui] disabling window transitions failed (hwnd={hwnd:#x}, hresult={:#010x})",
            hr as u32
        ));
    }
}

/// 系統標題列改用深色、底色同側欄；失敗只記 log（Win10 不支援自訂標題列底色）。
pub fn dark_caption(hwnd: isize) {
    let bg = palette::BG;
    let colorref = u32::from_le_bytes([bg.r(), bg.g(), bg.b(), 0]);
    for (name, attribute, value) in [
        ("dark mode", DWMWA_USE_IMMERSIVE_DARK_MODE, 1),
        ("caption color", DWMWA_CAPTION_COLOR, colorref),
    ] {
        let hr = set_dwm_attribute(hwnd, attribute, value);
        if hr != 0 {
            log(format!(
                "[ui] dark caption {name} not applied (hwnd={hwnd:#x}, hresult={:#010x})",
                hr as u32
            ));
        }
    }
}

/// 讓工作區蓋滿整個視窗，拿掉 Windows 11 畫的圓角、1px 邊框與陰影；重複呼叫無妨。
///
/// egui-winit 對無邊框視窗一律開 winit 的 undecorated shadow 且沒開放關閉（`has_shadow` 只管 macOS），
/// 留下的 1px 非工作區讓 DWM 當成一般視窗加框。子類別化後 winit 每次重算外框也都經過這裡。
pub fn remove_system_frame(hwnd: isize) {
    let hwnd_ = hwnd as HWND;
    if unsafe { SetWindowSubclass(hwnd_, Some(frameless_proc), 0, 0) } == 0 {
        log(format!(
            "[ui] system frame left on: hwnd={hwnd:#x} error={}",
            std::io::Error::last_os_error()
        ));
        return;
    }
    let flags = SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
    unsafe { SetWindowPos(hwnd_, std::ptr::null_mut(), 0, 0, 0, 0, flags) };
    log(format!("[ui] system frame removed: hwnd={hwnd:#x}"));
}

unsafe extern "system" fn no_taskbar_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    // winit 每次套用狀態（如顯示）都整個重寫 extended style 並加回 APPWINDOW，只能在這裡攔
    if msg == WM_STYLECHANGING && wparam as i32 == GWL_EXSTYLE {
        let change = unsafe { &mut *(lparam as *mut STYLESTRUCT) };
        change.styleNew &= !WS_EX_APPWINDOW;
    }
    if msg == WM_NCDESTROY {
        unsafe { RemoveWindowSubclass(hwnd, Some(no_taskbar_proc), 0) };
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// 讓視窗由 `owner` 擁有且永不帶 `WS_EX_APPWINDOW`，就沒有工作列按鈕；要在顯示之前呼叫。
///
/// egui 的 `with_taskbar(false)` 只在建立（還隱藏）時刪一次工作列按鈕，顯示時又會加回來。
pub fn hide_from_taskbar(hwnd: isize, owner: isize) {
    let hwnd_ = hwnd as HWND;
    if unsafe { SetWindowSubclass(hwnd_, Some(no_taskbar_proc), 0, 0) } == 0 {
        log(format!(
            "[ui] taskbar button left on: hwnd={hwnd:#x} error={}",
            std::io::Error::last_os_error()
        ));
        return;
    }
    unsafe {
        SetWindowLongPtrW(hwnd_, GWLP_HWNDPARENT, owner);
        SetWindowLongPtrW(
            hwnd_,
            GWL_EXSTYLE,
            (ex_style(hwnd) & !WS_EX_APPWINDOW) as isize,
        );
    }
}

/// 視窗的擁有者；沒有時回 0。
pub fn owner_of(hwnd: isize) -> isize {
    unsafe { GetWindow(hwnd as HWND, GW_OWNER) as isize }
}

/// winit 的視窗類別名稱；egui 的每個 viewport 都用它建立。
const WINIT_CLASS: &str = "Window Class";

type CreateWindowExFn = unsafe extern "system" fn(
    u32,
    PCWSTR,
    PCWSTR,
    u32,
    i32,
    i32,
    i32,
    i32,
    HWND,
    HMENU,
    HINSTANCE,
    *const c_void,
) -> HWND;

static CREATE_WINDOW_EX: OnceLock<CreateWindowExFn> = OnceLock::new();

/// `class` 是否為 winit 的視窗類別（小於 0x10000 的是 class atom，不是字串）。
fn is_winit_class(class: PCWSTR) -> bool {
    if (class as usize) < 0x10000 {
        return false;
    }
    let len = (0..).take_while(|&i| unsafe { *class.add(i) } != 0).count();
    let name = unsafe { std::slice::from_raw_parts(class, len) };
    WINIT_CLASS.encode_utf16().eq(name.iter().copied())
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn create_window_ex(
    ex_style: u32,
    class: PCWSTR,
    title: PCWSTR,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    parent: HWND,
    menu: HMENU,
    instance: HINSTANCE,
    param: *const c_void,
) -> HWND {
    let ex_style = if is_winit_class(class) {
        ex_style | WS_EX_NOREDIRECTIONBITMAP
    } else {
        ex_style
    };
    let original = CREATE_WINDOW_EX
        .get()
        .expect("set before the import is patched");
    unsafe {
        original(
            ex_style, class, title, style, x, y, width, height, parent, menu, instance, param,
        )
    }
}

/// 讓之後建立的 winit 視窗不配 GDI 重導向表面（`WS_EX_NOREDIRECTIONBITMAP`），畫面只走 DirectComposition。
///
/// 重導向表面每次顯示都被系統填成白色（RGB 白、alpha 0），疊在半透明底板下會透出一塊灰。
/// 這個樣式只能在建立時給，winit 雖有對應屬性但 egui-winit 沒開放，所以改寫本程式對
/// `CreateWindowExW` 的匯入；只做一次，重複呼叫無妨。等 egui-winit 開放後就改走 ViewportBuilder。
pub fn disable_redirection_bitmap() {
    static PATCHED: Once = Once::new();
    PATCHED.call_once(|| match unsafe { patch_create_window_import() } {
        Ok(slots) => log(format!(
            "[ui] winit windows created without a redirection bitmap (patched import slots={slots})"
        )),
        Err(e) => log(format!(
            "[ui] redirection bitmap left enabled: {e}; transparent windows may show a gray block"
        )),
    });
}

/// 把本程式匯入表裡指向 `CreateWindowExW` 的格子換成 [`create_window_ex`]，回傳換了幾格。
unsafe fn patch_create_window_import() -> Result<usize, String> {
    let user32 = unsafe { GetModuleHandleW(windows_sys::w!("user32.dll")) };
    let real = unsafe { GetProcAddress(user32, c"CreateWindowExW".as_ptr().cast()) }
        .ok_or("CreateWindowExW not found in user32.dll")?;
    let _ = CREATE_WINDOW_EX.set(unsafe {
        std::mem::transmute::<unsafe extern "system" fn() -> isize, CreateWindowExFn>(real)
    });
    let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
    let dos = unsafe { &*(base as *const IMAGE_DOS_HEADER) };
    let nt = (base + dos.e_lfanew as usize) as *const IMAGE_NT_HEADERS64;
    let dirs = unsafe { std::ptr::addr_of!((*nt).OptionalHeader.DataDirectory).read_unaligned() };
    let iat = dirs[IMAGE_DIRECTORY_ENTRY_IAT as usize];
    let slots = unsafe {
        std::slice::from_raw_parts_mut(
            (base + iat.VirtualAddress as usize) as *mut usize,
            iat.Size as usize / size_of::<usize>(),
        )
    };
    let mut patched = 0;
    // 不同版本的 windows-sys 可能各佔一格：全部換掉才蓋得到 winit 那格
    for slot in slots.iter_mut().filter(|s| **s == real as usize) {
        let mut old = 0;
        let at = slot as *mut usize as *const c_void;
        if unsafe { VirtualProtect(at, size_of::<usize>(), PAGE_READWRITE, &mut old) } == 0 {
            return Err(format!(
                "VirtualProtect failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        *slot = create_window_ex as CreateWindowExFn as usize;
        unsafe { VirtualProtect(at, size_of::<usize>(), old, &mut old) };
        patched += 1;
    }
    if patched == 0 {
        return Err("CreateWindowExW import slot not found".into());
    }
    Ok(patched)
}

/// 切到前景：前景是本程式時直接切；否則先送遮罩鍵取得前景權再切，仍被擋才掛上前景視窗的執行緒；視窗不存在或切換失敗回 `false`。
pub fn force_foreground(hwnd: isize) -> bool {
    let hwnd = hwnd as HWND;
    unsafe {
        if IsWindow(hwnd) == 0 {
            return false;
        }
        let mut fg_pid = 0u32;
        let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), &mut fg_pid);
        if fg_pid == GetCurrentProcessId() {
            return SetForegroundWindow(hwnd) != 0;
        }
        // 本程式送出最後一個輸入事件後前景鎖就放行；被擋下的直接切換會讓工作列按鈕閃爍，
        // 掛前景執行緒則要等遊戲回應、常要上百毫秒，只當退路
        use crate::input::inject::{Injector, SendInputInjector};
        SendInputInjector.mask();
        if SetForegroundWindow(hwnd) != 0 && GetForegroundWindow() == hwnd {
            return true;
        }
        log(format!(
            "[ui] foreground lock not released by input; attaching (hwnd={:#x})",
            hwnd as isize
        ));
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

/// 等 DWM 完成下一次合成：剛呈現的畫面要到那時才真的上螢幕。
pub fn wait_for_composition() {
    // SAFETY：無參數；DWM 停用時回傳錯誤，照樣往下走即可。
    let result = unsafe { DwmFlush() };
    if result < 0 {
        log(format!("[ui] DwmFlush failed: hresult={result:#x}"));
    }
}

/// DWM 隱形：不合成到螢幕，但 `IsWindowVisible` 仍為真（eframe 不會改走隱藏視窗的節流重繪）。
pub fn set_cloaked(hwnd: isize, cloaked: bool) {
    let result = set_dwm_attribute(hwnd, DWMWA_CLOAK, cloaked.into());
    if result < 0 {
        log(format!(
            "[ui] DWM cloak={cloaked} failed: hwnd={hwnd:#x} hresult={result:#x}"
        ));
    }
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

/// 含 (x, y) 那顆螢幕的縮放比例（每 DIP 的實體像素數）；取不到時為 1.0。
pub fn scale_at(x: i32, y: i32) -> f32 {
    let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    let hr = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    if hr < 0 || dpi_x == 0 {
        return 1.0;
    }
    dpi_x as f32 / 96.0
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

unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    let found = unsafe { &mut *(lparam as *mut Vec<isize>) };
    found.push(hwnd as isize);
    1
}

/// 目前執行緒建立的所有頂層視窗（含隱藏的）。
pub fn thread_windows() -> Vec<isize> {
    let mut found = Vec::<isize>::new();
    unsafe {
        EnumThreadWindows(
            GetCurrentThreadId(),
            Some(collect_window),
            &mut found as *mut _ as LPARAM,
        )
    };
    found
}

/// 視窗標題；取不到時為空字串。
pub fn window_title(hwnd: isize) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd as HWND, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

/// 目前執行緒上標題為 `title` 的可見視窗（略過 `except`）；找不到回 0。
pub fn find_thread_window(title: &str, except: &[isize]) -> isize {
    thread_windows()
        .into_iter()
        .find(|h| {
            !except.contains(h)
                && unsafe { IsWindowVisible(*h as HWND) } != 0
                && window_title(*h) == title
        })
        .unwrap_or(0)
}

/// 從建立 viewport 前後的執行緒視窗清單認出它的 HWND：新出現、且不在 `known` 裡的那個；
/// 沒有新視窗（不是建立的那一幀）才退回 `by_title` —— 同標題的視窗不只一個，`by_title` 也要略過 `known`。
pub fn pick_new_window(
    before: &[isize],
    after: &[isize],
    known: &[isize],
    by_title: impl FnOnce() -> isize,
) -> isize {
    after
        .iter()
        .copied()
        .find(|h| !known.contains(h) && !before.contains(h))
        .unwrap_or_else(by_title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_keyboard_removal_targets_only_the_keyboard() {
        let device = raw_keyboard_removal();
        assert_eq!((device.usUsagePage, device.usUsage), (0x01, 0x06));
        assert_eq!(device.dwFlags, RIDEV_REMOVE);
        assert!(
            device.hwndTarget.is_null(),
            "RIDEV_REMOVE needs a null target"
        );
    }

    #[test]
    fn is_winit_class_matches_only_the_winit_class_name() {
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
        assert!(is_winit_class(wide("Window Class").as_ptr()));
        assert!(!is_winit_class(wide("Window Class2").as_ptr()));
        assert!(!is_winit_class(wide("Winit Thread Event Target").as_ptr()));
        assert!(!is_winit_class(0xc027 as PCWSTR));
    }

    #[test]
    fn work_area_lies_within_its_monitor() {
        let (mx, my, mw, mh) = monitor_rect_at(0, 0);
        let (wx, wy, ww, wh) = work_area_at(0, 0);
        assert!(mw > 0 && mh > 0);
        assert!(wx >= mx && wy >= my && wx + ww <= mx + mw && wy + wh <= my + mh);
    }

    #[test]
    fn find_thread_window_is_zero_on_a_thread_without_windows() {
        assert!(thread_windows().is_empty());
        assert_eq!(find_thread_window("Wizard101 Chat Translator", &[0]), 0);
    }

    #[test]
    fn pick_new_window_skips_known_windows() {
        let by_title = || 7;
        assert_eq!(pick_new_window(&[1, 7], &[1, 7, 9], &[1], by_title), 9);
        assert_eq!(pick_new_window(&[7], &[1, 7], &[1], by_title), 7);
        assert_eq!(pick_new_window(&[1, 9], &[1, 9], &[1], by_title), 7);
        // 同標題的兩個視窗（5、9）在同一段期間出現：已知的 5 不會被當成新視窗
        assert_eq!(pick_new_window(&[1], &[1, 5, 9], &[1, 5], by_title), 9);
        assert_eq!(pick_new_window(&[1], &[1, 5], &[1, 5], || 0), 0);
    }

    #[test]
    fn scale_at_reads_a_plausible_monitor_scale() {
        // Windows 的縮放下限是 100%，上限 500%
        let scale = scale_at(0, 0);
        assert!((1.0..=5.0).contains(&scale), "{scale}");
    }

    #[test]
    fn all_monitors_includes_the_nearest_one() {
        assert!(all_monitors().contains(&monitor_rect_at(0, 0)));
    }

    #[test]
    fn dark_caption_on_a_missing_window_fails_without_panicking() {
        for attribute in [DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_CAPTION_COLOR] {
            assert_ne!(set_dwm_attribute(0, attribute, 1), 0);
        }
        dark_caption(0);
    }
}
