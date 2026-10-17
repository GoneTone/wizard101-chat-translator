//! 系統剪貼簿寫入（`CF_UNICODETEXT`）。

use windows_sys::Win32::Foundation::{GetLastError, GlobalFree};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

use crate::logs::log;

/// 把 `text` 寫進系統剪貼簿；失敗時記 log 並回 `false`。
pub fn set_text(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    match unsafe { write(&wide) } {
        Ok(()) => true,
        Err((step, code)) => {
            log(format!(
                "[ui] clipboard write failed: step={step} error={code}"
            ));
            false
        }
    }
}

unsafe fn write(wide: &[u16]) -> Result<(), (&'static str, u32)> {
    let fail = |step| Err((step, unsafe { GetLastError() }));
    if unsafe { OpenClipboard(std::ptr::null_mut()) } == 0 {
        return fail("open");
    }
    let result = unsafe {
        if EmptyClipboard() == 0 {
            fail("empty")
        } else {
            let mem = GlobalAlloc(GMEM_MOVEABLE, size_of_val(wide));
            let dst = if mem.is_null() {
                std::ptr::null_mut()
            } else {
                GlobalLock(mem) as *mut u16
            };
            if dst.is_null() {
                let result = fail("alloc");
                GlobalFree(mem);
                result
            } else {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
                GlobalUnlock(mem);
                // 成功後記憶體歸系統所有，只有失敗才要自己釋放
                if SetClipboardData(u32::from(CF_UNICODETEXT), mem).is_null() {
                    let result = fail("set");
                    GlobalFree(mem);
                    result
                } else {
                    Ok(())
                }
            }
        }
    };
    unsafe { CloseClipboard() };
    result
}
