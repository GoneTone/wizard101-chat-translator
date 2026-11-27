//! 多帳號一鍵登入：帳號儲存、遊戲位置偵測、登入連線、Steam 票據、用戶端啟動與一鍵編排。

pub mod accounts;
pub mod install;
pub mod launch;
pub mod login;
pub mod running;

use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};

pub use accounts::{Account, AccountKind};

/// Steam 帳號登入用的票據；刻意不實作 `Debug`，免得票據被印進 log。
#[derive(Clone)]
pub struct SteamTicket {
    /// 目前登入 Steam 的使用者 SteamID（十進位）。
    pub steam_id: String,
    /// Web API 票據的原始位元組。
    pub ticket: Vec<u8>,
}

/// 讀 `HKEY_CURRENT_USER` 底下的字串值；不存在或不是字串回 `None`。
fn hkcu_string(key: &str, value: &str) -> Option<String> {
    let key: Vec<u16> = key.encode_utf16().chain([0]).collect();
    let value: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let mut buf = [0u16; 1024];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY：字串皆以 NUL 結尾，size 是 buf 的位元組數。
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
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
