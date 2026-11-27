//! 帳號清單與密碼加密：存在 config 的 `accounts`，密碼以 DPAPI 加密成 hex。

use std::ptr;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

use zeroize::{Zeroize, Zeroizing};

use crate::config::Config;
use crate::logs::log;
use crate::util::{hex, unhex};

/// 帳號類型：官網註冊，或透過 Steam 建立／綁定。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
    Official,
    Steam,
}

/// 一筆帳號；`name` 就是登入帳號名，也是啟動參數 `-U` 末段的比對鍵。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub kind: AccountKind,
    /// DPAPI 加密後的密碼（hex），只有同一 Windows 使用者解得開。
    pub password_dpapi: String,
}

// 手寫 Debug：除錯輸出不帶加密後的密碼
impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// config 裡的全部帳號，依存放順序；形狀不對的那筆略過。
pub fn stored(cfg: &Config) -> Vec<Account> {
    cfg.get("accounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            serde_json::from_value(v.clone())
                .inspect_err(|e| log(format!("[launch] skipped malformed account entry: {e}")))
                .ok()
        })
        .collect()
}

/// 把帳號清單寫回 config。
pub fn store(cfg: &mut Config, accounts: &[Account]) {
    let list = accounts
        .iter()
        .map(|a| serde_json::to_value(a).expect("Account serializes"))
        .collect();
    cfg.insert("accounts".into(), Value::Array(list));
}

/// 依 id 取代清單裡的同一筆帳號，沒有就加在最後。
pub fn upsert(accounts: &mut Vec<Account>, account: Account) {
    match accounts.iter_mut().find(|a| a.id == account.id) {
        Some(slot) => *slot = account,
        None => accounts.push(account),
    }
}

/// 上次在開啟遊戲帳號視窗勾選的帳號 id。
pub fn launch_selection(cfg: &Config) -> Vec<String> {
    cfg.get("launch_selection")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(Into::into))
        .collect()
}

/// 記下這次勾選的帳號 id。
pub fn set_launch_selection(cfg: &mut Config, ids: &[String]) {
    cfg.insert("launch_selection".into(), ids.into());
}

/// 從開啟遊戲帳號視窗開好帳號後要不要自動關窗；沒設定過是 `true`。
pub fn launch_auto_close(cfg: &Config) -> bool {
    cfg.get("launch_auto_close")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

/// 記下開好帳號後要不要自動關窗。
pub fn set_launch_auto_close(cfg: &mut Config, on: bool) {
    cfg.insert("launch_auto_close".into(), on.into());
}

/// 密碼加解密失敗；訊息只含系統錯誤碼，不含密碼。
#[derive(Debug, PartialEq, Eq)]
pub struct CryptError(pub String);

fn blob(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    }
}

/// 取出 DPAPI 配置的輸出，歸零後以 `LocalFree` 釋放。
///
/// # Safety
/// `out` 必須是 DPAPI 剛成功填好的 blob。
unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let raw = unsafe { std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize) };
    let bytes = raw.to_vec();
    raw.zeroize();
    unsafe { LocalFree(out.pbData.cast()) };
    bytes
}

fn last_error(what: &str) -> CryptError {
    CryptError(format!(
        "{what} failed: {}",
        std::io::Error::last_os_error()
    ))
}

/// 以目前 Windows 使用者的 DPAPI 加密密碼，回傳 hex。
pub fn encrypt_password(plain: &str) -> Result<String, CryptError> {
    let input = blob(plain.as_bytes());
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let ok = unsafe {
        CryptProtectData(
            &input,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        return Err(last_error("CryptProtectData"));
    }
    Ok(hex(&unsafe { take(out) }))
}

/// 解開 [`encrypt_password`] 存下的密碼；用完即歸零。
pub fn decrypt_password(stored: &str) -> Result<Zeroizing<String>, CryptError> {
    let bytes = unhex(stored).ok_or_else(|| CryptError("stored password is not hex".into()))?;
    let input = blob(&bytes);
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        return Err(last_error("CryptUnprotectData"));
    }
    String::from_utf8(unsafe { take(out) })
        .map(Zeroizing::new)
        .map_err(|e| {
            e.into_bytes().zeroize();
            CryptError("decrypted password is not UTF-8".into())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_then_decrypt_round_trips() {
        let stored = encrypt_password("hunter2").unwrap();
        assert_eq!(decrypt_password(&stored).unwrap().as_str(), "hunter2");
    }

    #[test]
    fn debug_output_leaves_out_the_encrypted_password() {
        let account = Account {
            id: "a".into(),
            name: "alice".into(),
            kind: AccountKind::Official,
            password_dpapi: "c1a55ed".into(),
        };
        let shown = format!("{account:?}");
        assert!(shown.contains("alice"));
        assert!(!shown.contains("c1a55ed"));
    }

    #[test]
    fn encrypted_blob_is_not_plaintext() {
        let stored = encrypt_password("hunter2").unwrap();
        assert!(!stored.contains("hunter2"));
        assert!(!stored.contains(&crate::util::hex(b"hunter2")));
    }

    #[test]
    fn auto_close_defaults_to_on_and_reads_the_saved_flag() {
        let mut cfg = Config::new();
        assert!(launch_auto_close(&cfg));
        set_launch_auto_close(&mut cfg, false);
        assert!(!launch_auto_close(&cfg));
    }

    #[test]
    fn decrypt_rejects_garbage() {
        assert!(decrypt_password("zz").is_err());
        assert!(decrypt_password("00112233").is_err());
    }
}
