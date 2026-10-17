//! 設定介面的耗時工作（測試連線、抓模型清單、檢查更新、錄熱鍵）與結果的輪次比對。
//!
//! 介面只透過 `SettingsEnv` 發起工作，結果以 `UiEvent::Settings` 回到 UI 執行緒；測試換成假的 env。

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows_sys::w;

use crate::config::services::ServiceSpec;
use crate::input::dispatcher::SharedDispatcher;
use crate::input::recorder::{Recording, record};
use crate::logs::log;
use crate::runtime::updater::{LATEST_API, Release, check_for_update, http_client};
use crate::translation::cache::TranslationCache;
use crate::translation::errors::TranslateError;
use crate::translation::examples::ExampleStore;
use crate::translation::translator::{self, TestOutcome, Translator, test_translate};
use crate::ui::events::{UiEvent, UiPoster};
use crate::util::lock;

/// 設定介面要用的外部動作；耗時的在背景跑，結果以 `UiEvent::Settings` 投遞，帶著呼叫時給的 `round`。
pub trait SettingsEnv {
    /// 用 `spec` 實際翻一句測試文字（目標語言 `target`），完成後投遞 `TestDone`。
    fn test_connection(&self, round: u64, spec: ServiceSpec, target: String);
    /// 向端點取模型清單，完成後投遞 `ModelsDone`。
    fn list_models(&self, round: u64, spec: ServiceSpec);
    /// 手動檢查更新，完成後投遞 `UpdateChecked`。
    fn check_update(&self, round: u64);
    /// 開始錄熱鍵（先停掉前一次），第一次有鍵放開時投遞 `HotkeyRecorded`。
    fn record_hotkey(&self, round: u64);
    /// 停止錄製；收到 `HotkeyRecorded` 後也要呼叫，錄製器才會從分派器移除。
    fn cancel_recording(&self);
    /// 用檔案總管開啟資料夾。
    fn open_folder(&self, path: &Path);
    /// 有沒有譯文快取；沒有時關於頁不畫譯文快取那一列。
    fn has_cache(&self) -> bool;
    /// 清掉譯文快取，回傳清掉的筆數。
    fn clear_cache(&self) -> usize;
}

/// `SettingsEnv` 背景工作的結果。
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsEvent {
    /// 測試連線結束。
    TestDone {
        round: u64,
        result: Result<TestOutcome, TranslateError>,
    },
    /// 模型清單取回（或失敗）。
    ModelsDone {
        round: u64,
        result: Result<Vec<String>, TranslateError>,
    },
    /// `Ok(None)` 是已是最新版；`Err` 是失敗原因。
    UpdateChecked {
        round: u64,
        result: Result<Option<Release>, String>,
    },
    /// 錄到的熱鍵；`esc` 表示使用者取消。
    HotkeyRecorded { round: u64, combo: String },
}

/// 正式環境的 `SettingsEnv`：工作丟到 tokio runtime，熱鍵錄製掛在鍵盤分派器上。
pub struct RealEnv {
    rt: Handle,
    ui: UiPoster,
    dispatcher: SharedDispatcher,
    cache: Option<Arc<TranslationCache>>,
    recording: Mutex<Option<Recording>>,
}

impl RealEnv {
    /// `cache` 為 `None` 時關於頁不提供清除譯文快取。
    pub fn new(
        rt: Handle,
        ui: UiPoster,
        dispatcher: SharedDispatcher,
        cache: Option<Arc<TranslationCache>>,
    ) -> Self {
        Self {
            rt,
            ui,
            dispatcher,
            cache,
            recording: Mutex::new(None),
        }
    }

    fn post_later(&self, work: impl Future<Output = SettingsEvent> + Send + 'static) {
        let ui = self.ui.clone();
        self.rt
            .spawn(async move { ui.post(UiEvent::Settings(work.await)) });
    }
}

impl SettingsEnv for RealEnv {
    fn test_connection(&self, round: u64, spec: ServiceSpec, target: String) {
        self.post_later(async move {
            let store = Arc::new(ExampleStore::default());
            let result = test_translate(Translator::new(spec, target), Some(store)).await;
            SettingsEvent::TestDone { round, result }
        });
    }

    fn list_models(&self, round: u64, spec: ServiceSpec) {
        self.post_later(async move {
            let result = translator::list_models(&spec).await;
            SettingsEvent::ModelsDone { round, result }
        });
    }

    fn check_update(&self, round: u64) {
        self.post_later(async move {
            let result = check_for_update(&http_client(), LATEST_API, env!("CARGO_PKG_VERSION"))
                .await
                .map_err(|e| {
                    log(format!("[update] manual check failed: {e}"));
                    e.to_string()
                });
            SettingsEvent::UpdateChecked { round, result }
        });
    }

    fn record_hotkey(&self, round: u64) {
        let mut slot = lock(&self.recording);
        // 先停掉前一次：它的 drop 會鎖分派器，只能在 UI 執行緒做
        drop(slot.take());
        let ui = self.ui.clone();
        let done = Box::new(move |combo| {
            ui.post(UiEvent::Settings(SettingsEvent::HotkeyRecorded {
                round,
                combo,
            }))
        });
        *slot = Some(record(&self.dispatcher, done));
    }

    fn cancel_recording(&self) {
        drop(lock(&self.recording).take());
    }

    fn open_folder(&self, path: &Path) {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let code = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                w!("open"),
                wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        } as isize;
        // ShellExecuteW 回傳值大於 32 才是成功
        if code <= 32 {
            log(format!(
                "[ui] open log folder failed: path={} error={} (code={code})",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
    }

    fn has_cache(&self) -> bool {
        self.cache.is_some()
    }

    fn clear_cache(&self) -> usize {
        let Some(cache) = &self.cache else {
            return 0;
        };
        let count = cache.clear();
        log(format!(
            "[settings] translation cache cleared by user ({count} entries)"
        ));
        count
    }
}

static NEXT_ROUND: AtomicU64 = AtomicU64::new(1);

/// 一個按鈕觸發的背景工作的輪次：結果回來時比對編號，過期或作廢的結果丟掉。
///
/// 編號全域遞增，不同元件的結果就算走同一種事件也不會撞號。
#[derive(Debug)]
pub struct Round {
    /// log 用的工作名稱。
    label: &'static str,
    current: Option<u64>,
    invalidated: bool,
}

impl Round {
    /// `label` 是 log 用的工作名稱（如 `"test connection"`）。
    pub fn new(label: &'static str) -> Self {
        Self {
            label,
            current: None,
            invalidated: false,
        }
    }

    /// 開始新的一輪（舊的一輪就此過期），回傳本輪編號。
    pub fn start(&mut self) -> u64 {
        let round = NEXT_ROUND.fetch_add(1, Ordering::Relaxed);
        self.current = Some(round);
        self.invalidated = false;
        round
    }

    /// 作廢還沒回來的結果（等待期間表單被改過）；按鈕仍忙到結果回來為止。
    pub fn invalidate(&mut self) {
        self.invalidated = self.current.is_some();
    }

    /// 放棄本輪：立刻解除忙碌，之後回來的結果一律丟掉（給不會再有結果的工作，如取消錄熱鍵）。
    pub fn cancel(&mut self) {
        self.current = None;
        self.invalidated = false;
    }

    /// 結果回來：只有本輪且沒被作廢才回 `true`；本輪的結果一律解除忙碌。
    pub fn finish(&mut self, round: u64) -> bool {
        let current = self.current == Some(round);
        if current {
            self.current = None;
        }
        if !current || self.invalidated {
            log(format!(
                "[ui] {} result discarded: state changed meanwhile",
                self.label
            ));
            return false;
        }
        true
    }

    /// 本輪還在等結果。
    pub fn busy(&self) -> bool {
        self.current.is_some()
    }
}
