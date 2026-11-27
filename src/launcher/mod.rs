//! 多帳號一鍵登入：帳號儲存、遊戲安裝位置偵測、登入連線、Steam 票據、用戶端啟動與一鍵編排。

pub mod accounts;
pub mod install;
pub mod launch;
pub mod login;
pub mod orchestrate;
pub mod running;
pub mod steam;

use windows_sys::Win32::System::Registry::{
    HKEY, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
};

pub use accounts::{Account, AccountKind};

/// Steam 帳號登入用的票據；刻意不實作 `Debug`，免得票據被印進 log。
#[derive(Clone)]
pub struct SteamTicket {
    /// 目前登入 Steam 的使用者 SteamID（十進位）。
    pub steam_id: String,
    /// Web API 票據的原始位元組。
    pub ticket: Vec<u8>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// 讀登錄檔字串值；不存在或不是字串回 `None`。
fn reg_string(root: HKEY, key: &str, value: &str) -> Option<String> {
    let (key, value) = (wide(key), wide(value));
    let mut buf = [0u16; 1024];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY：字串皆以 NUL 結尾，size 是 buf 的位元組數。
    let status = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

/// 列出登錄檔機碼底下的子機碼名稱；打不開回空清單。
fn reg_subkeys(root: HKEY, key: &str) -> Vec<String> {
    let mut handle: HKEY = std::ptr::null_mut();
    // SAFETY：路徑以 NUL 結尾；成功開啟的 handle 在最後關閉。
    if unsafe { RegOpenKeyExW(root, wide(key).as_ptr(), 0, KEY_READ, &mut handle) } != 0 {
        return Vec::new();
    }
    let mut names = Vec::new();
    for index in 0.. {
        // 機碼名稱最長 255 字元
        let mut buf = [0u16; 256];
        let mut len = buf.len() as u32;
        // SAFETY：len 是 buf 的字元數，其餘輸出參數不取。
        let status = unsafe {
            RegEnumKeyExW(
                handle,
                index,
                buf.as_mut_ptr(),
                &mut len,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status != 0 {
            break;
        }
        names.push(String::from_utf16_lossy(&buf[..len as usize]));
    }
    // SAFETY：handle 由上面成功開啟。
    unsafe { RegCloseKey(handle) };
    names
}
