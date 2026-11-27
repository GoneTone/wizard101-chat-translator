//! Steam 票據：在本程序載入遊戲的 `steam_api64.dll`，取目前登入 Steam 使用者的 Web API 票據。

use std::ffi::{OsStr, c_char, c_void};
use std::os::windows::ffi::OsStrExt;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

use super::SteamTicket;
use super::install::{Install, STEAM_API_DLL};
use crate::game::process::{process_exe_path, process_ids};
use crate::logs::log;

const WIZARD101_APP_ID: &str = "799960";
const TICKET_IDENTITY: &[u8] = b"Wizard101\0";
/// GetTicketForWebApiResponse_t 的 callback 編號。
const TICKET_CALLBACK: i32 = 168;
const RESULT_OK: i32 = 1;
const MAX_TICKET: i32 = 2560;
const TICKET_TIMEOUT: Duration = Duration::from_secs(30);

/// 取票據失敗的原因；不含票據內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SteamError {
    /// Steam 用戶端沒在執行。
    NotRunning,
    /// 載不到 dll 或需要的函式（安裝不完整或 SDK 版本不符）。
    InterfaceUnavailable,
    /// Steam 開著但初始化失敗，帶 SDK 回的錯誤碼。
    InitFailed(i32),
    /// Steam 沒有回有效的使用者或票據（多半是沒登入）。
    NoSession,
    /// 等票據逾時。
    Timeout,
}

#[repr(C)]
struct CallbackMsg {
    _user: i32,
    callback: i32,
    param: *const u8,
    param_size: i32,
}

/// 用到的 flat API；全部在載入時一次解析，缺任何一個就當介面不可用。
struct Api {
    init: unsafe extern "C" fn(*mut c_char) -> i32,
    shutdown: unsafe extern "C" fn(),
    user: unsafe extern "C" fn() -> *mut c_void,
    steam_id: unsafe extern "C" fn(*mut c_void) -> u64,
    ticket: unsafe extern "C" fn(*mut c_void, *const c_char) -> u32,
    cancel: unsafe extern "C" fn(*mut c_void, u32),
    pipe: unsafe extern "C" fn() -> i32,
    dispatch_init: unsafe extern "C" fn(),
    run_frame: unsafe extern "C" fn(i32),
    next_callback: unsafe extern "C" fn(i32, *mut CallbackMsg) -> bool,
    free_callback: unsafe extern "C" fn(i32),
}

impl Api {
    // transmute 的目標型別就是各欄位宣告的函式型別
    #[allow(clippy::missing_transmute_annotations)]
    fn load(module: HMODULE) -> Option<Self> {
        macro_rules! sym {
            ($name:literal) => {{
                // SAFETY：名稱以 NUL 結尾；函式型別照 steam_api_flat.h 宣告。
                let f = unsafe { GetProcAddress(module, concat!($name, "\0").as_ptr()) }?;
                unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, _>(f) }
            }};
        }
        Some(Self {
            init: sym!("SteamAPI_InitFlat"),
            shutdown: sym!("SteamAPI_Shutdown"),
            user: sym!("SteamAPI_SteamUser_v023"),
            steam_id: sym!("SteamAPI_ISteamUser_GetSteamID"),
            ticket: sym!("SteamAPI_ISteamUser_GetAuthTicketForWebApi"),
            cancel: sym!("SteamAPI_ISteamUser_CancelAuthTicket"),
            pipe: sym!("SteamAPI_GetHSteamPipe"),
            dispatch_init: sym!("SteamAPI_ManualDispatch_Init"),
            run_frame: sym!("SteamAPI_ManualDispatch_RunFrame"),
            next_callback: sym!("SteamAPI_ManualDispatch_GetNextCallback"),
            free_callback: sym!("SteamAPI_ManualDispatch_FreeLastCallback"),
        })
    }
}

/// 一次 Steam 認證會話：票據在 drop 前有效，登入完成後再 drop（取消票據、關閉 Steam API）。
pub struct SteamSession {
    api: Api,
    user: *mut c_void,
    handle: u32,
    pub ticket: SteamTicket,
}

// SAFETY：Steam flat API 可跨執行緒呼叫；session 只在一個工作裡依序使用與釋放。
unsafe impl Send for SteamSession {}

impl Drop for SteamSession {
    fn drop(&mut self) {
        // SAFETY：user／handle 來自同一次成功初始化。
        unsafe {
            (self.api.cancel)(self.user, self.handle);
            (self.api.shutdown)();
        }
        self.ticket.ticket.fill(0);
        log("[launch] steam session closed");
    }
}

fn steam_running() -> bool {
    process_ids()
        .unwrap_or_default()
        .into_iter()
        .any(|pid| process_exe_path(pid).is_some_and(|p| p.to_lowercase().ends_with(r"\steam.exe")))
}

/// 暫時把 Steam 的 App ID 環境變數設成 Wizard101，離開時還原。
struct AppIdEnv(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl AppIdEnv {
    fn set() -> Self {
        let saved = ["SteamAppId", "SteamGameId"]
            .map(|k| (k, std::env::var_os(k)))
            .into();
        for k in ["SteamAppId", "SteamGameId"] {
            // SAFETY：Windows 的環境區塊由系統加鎖；Steam 帳號逐一處理，不會並行改同一組變數。
            unsafe { std::env::set_var(k, WIZARD101_APP_ID) };
        }
        Self(saved)
    }
}

impl Drop for AppIdEnv {
    fn drop(&mut self) {
        for (k, v) in &self.0 {
            // SAFETY：同 [`AppIdEnv::set`]。
            unsafe {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
}

/// 等 GetTicketForWebApi 的 callback，回票據位元組。
///
/// # Safety
/// `api` 已成功初始化並啟用手動派送，`pipe` 是其 HSteamPipe。
unsafe fn wait_ticket(api: &Api, pipe: i32, handle: u32) -> Result<Vec<u8>, SteamError> {
    let started = Instant::now();
    while started.elapsed() < TICKET_TIMEOUT {
        unsafe { (api.run_frame)(pipe) };
        let mut msg = CallbackMsg {
            _user: 0,
            callback: 0,
            param: std::ptr::null(),
            param_size: 0,
        };
        while unsafe { (api.next_callback)(pipe, &mut msg) } {
            // 回應內容：0 票據 handle、4 結果、8 票據長度、12 起票據本體
            let read = |at: usize| unsafe { msg.param.add(at).cast::<i32>().read_unaligned() };
            let ours =
                msg.callback == TICKET_CALLBACK && msg.param_size >= 12 && read(0) as u32 == handle;
            let result = ours.then(|| {
                let (status, len) = (read(4), read(8));
                if status != RESULT_OK
                    || !(1..=MAX_TICKET).contains(&len)
                    || len + 12 > msg.param_size
                {
                    log(format!(
                        "[launch] steam ticket rejected: result={status}, length={len}"
                    ));
                    return Err(SteamError::NoSession);
                }
                Ok(unsafe { std::slice::from_raw_parts(msg.param.add(12), len as usize) }.to_vec())
            });
            unsafe { (api.free_callback)(pipe) };
            if let Some(result) = result {
                return result;
            }
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    Err(SteamError::Timeout)
}

/// 取目前登入 Steam 使用者的 Wizard101 Web API 票據；會阻塞到 Steam 回應或逾時。
pub fn acquire_ticket(install: &Install) -> Result<SteamSession, SteamError> {
    let dll = install.root.join("Bin").join(STEAM_API_DLL);
    let wide: Vec<u16> = OsStr::new(&dll).encode_wide().chain([0]).collect();
    // SAFETY：wide 以 NUL 結尾；dll 不卸載，Steam API 可能仍有背景執行緒。
    let module = unsafe { LoadLibraryW(wide.as_ptr()) };
    let Some(api) = (!module.is_null()).then(|| Api::load(module)).flatten() else {
        log(format!(
            "[launch] steam api unavailable at {}: {}",
            dll.display(),
            std::io::Error::last_os_error()
        ));
        return Err(SteamError::InterfaceUnavailable);
    };
    let code = {
        let _env = AppIdEnv::set();
        let mut message = [0 as c_char; 1024];
        // SAFETY：緩衝區即 SteamErrMsg（1024 bytes）。
        unsafe { (api.init)(message.as_mut_ptr()) }
    };
    if code != 0 {
        let running = steam_running();
        log(format!(
            "[launch] SteamAPI_InitFlat failed: code={code}, steam_running={running}"
        ));
        return Err(if running {
            SteamError::InitFailed(code)
        } else {
            SteamError::NotRunning
        });
    }
    // 從這裡起任何失敗都要 Shutdown，否則 Steam 會一直顯示在玩 Wizard101
    let shutdown = |e: SteamError| {
        // SAFETY：init 已成功。
        unsafe { (api.shutdown)() };
        Err(e)
    };
    // SAFETY：init 已成功，以下依 flat API 規範呼叫。
    let user = unsafe { (api.user)() };
    if user.is_null() {
        log("[launch] steam user interface unavailable");
        return shutdown(SteamError::InterfaceUnavailable);
    }
    let steam_id = unsafe { (api.steam_id)(user) };
    unsafe { (api.dispatch_init)() };
    let handle = unsafe { (api.ticket)(user, TICKET_IDENTITY.as_ptr().cast()) };
    let pipe = unsafe { (api.pipe)() };
    if steam_id == 0 || handle == 0 || pipe == 0 {
        log(format!(
            "[launch] steam returned no session: has_id={}, handle={handle}, pipe={pipe}",
            steam_id != 0
        ));
        return shutdown(SteamError::NoSession);
    }
    match unsafe { wait_ticket(&api, pipe, handle) } {
        Ok(ticket) => {
            log(format!(
                "[launch] steam ticket acquired ({} bytes)",
                ticket.len()
            ));
            Ok(SteamSession {
                api,
                user,
                handle,
                ticket: SteamTicket {
                    steam_id: steam_id.to_string(),
                    ticket,
                },
            })
        }
        Err(e) => {
            log(format!("[launch] steam ticket failed: {e:?}"));
            // SAFETY：handle 來自本次初始化。
            unsafe { (api.cancel)(user, handle) };
            shutdown(e)
        }
    }
}
