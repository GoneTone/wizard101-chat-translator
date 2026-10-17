//! 系統剪貼簿讀寫（`CF_UNICODETEXT`）。

use windows_sys::Win32::Foundation::{GetLastError, GlobalFree, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

use crate::logs::log;

/// 以視窗 `owner` 的名義把 `text` 寫進系統剪貼簿；失敗時記 log 並回 `false`。
///
/// 擁有者不可為 NULL：那樣 `EmptyClipboard` 會把擁有者設成 NULL，`SetClipboardData` 可能失敗。
pub fn set_text(owner: isize, text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    match unsafe { write(owner as HWND, &wide) } {
        Ok(()) => true,
        Err((step, code)) => {
            log(format!(
                "[ui] clipboard write failed: step={step} error={code}"
            ));
            false
        }
    }
}

/// 讀取系統剪貼簿的文字；`Err` 是開不了剪貼簿，`Ok(None)` 是裡面沒有文字。
pub fn get_text() -> Result<Option<String>, String> {
    unsafe { with_open(std::ptr::null_mut(), || read()) }
        .map_err(|(step, code)| format!("{step} error={code}"))
}

/// 開啟剪貼簿跑 `f`，不論結果都會關閉。
unsafe fn with_open<T>(owner: HWND, f: impl FnOnce() -> T) -> Result<T, (&'static str, u32)> {
    if unsafe { OpenClipboard(owner) } == 0 {
        return Err(("open", unsafe { GetLastError() }));
    }
    let result = f();
    unsafe { CloseClipboard() };
    Ok(result)
}

unsafe fn read() -> Option<String> {
    unsafe {
        let mem = GetClipboardData(u32::from(CF_UNICODETEXT));
        if mem.is_null() {
            return None;
        }
        let src = GlobalLock(mem) as *const u16;
        if src.is_null() {
            return None;
        }
        // 別的程式放的文字可能沒有結尾 NUL，長度以記憶體區塊大小為界
        let max = GlobalSize(mem) / 2;
        let len = (0..max).take_while(|&i| *src.add(i) != 0).count();
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(src, len));
        GlobalUnlock(mem);
        Some(text)
    }
}

unsafe fn write(owner: HWND, wide: &[u16]) -> Result<(), (&'static str, u32)> {
    let fail = |step| Err((step, unsafe { GetLastError() }));
    unsafe {
        with_open(owner, || {
            if EmptyClipboard() == 0 {
                return fail("empty");
            }
            let mem = GlobalAlloc(GMEM_MOVEABLE, size_of_val(wide));
            let dst = if mem.is_null() {
                std::ptr::null_mut()
            } else {
                GlobalLock(mem) as *mut u16
            };
            if dst.is_null() {
                let result = fail("alloc");
                GlobalFree(mem);
                return result;
            }
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
        })
    }
    .and_then(|result| result)
}
