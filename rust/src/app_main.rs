//! 主程式接線：翻譯端（翻譯器、快取、兩條翻譯池、範例）、收訊端（supervisor）、熱鍵與貼上、關閉流程。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
use windows_sys::Win32::Foundation::{FALSE, TRUE};
use windows_sys::Win32::System::Console::{CTRL_C_EVENT, SetConsoleCtrlHandler};
use windows_sys::core::BOOL;

use crate::composer::paste::{TypingEnv, paste_clipboard, paste_interceptor};
use crate::config::services::resolve;
use crate::config::{
    Config, ServiceSlot, Settings, app_name, config_summary, default_config, py, save_config,
};
use crate::i18n::{current_language, t};
use crate::input::dispatch::{
    ChatInputTracker, Foreground, on_hotkey, on_region_hotkey, region_hotkey_to_register,
    should_intercept_paste,
};
use crate::input::dispatcher::SharedDispatcher;
use crate::input::hook::{self, KeyboardHook};
use crate::input::hotkeys::{Registration, register_hotkey};
use crate::logs::log;
use crate::reader::mem_reader::WizChatReader;
use crate::reader::memory::{ProcessMemory, WinProcess};
use crate::reader::message_log::{MessageLog, SharedWriter};
use crate::reader::process::game_windows;
use crate::reader::run_loop::{MessageIds, ReaderShared, reader_loop};
use crate::reader::status::StatusBoard;
use crate::reader::supervisor::{JOIN_TIMEOUT, supervise};
use crate::runtime::is_elevated;
use crate::translation::cache::{TranslationCache, fingerprint_of, translate_and_cache};
use crate::translation::context::{ClientContexts, ContextMap};
use crate::translation::errors::TranslateError;
use crate::translation::examples::{ExampleSet, ExampleStore};
use crate::translation::gate::ConcurrencyGate;
use crate::translation::pool::{ResultFn, TranslateFn, TranslationPool};
use crate::translation::translator::{ExampleCoordinator, Translator};
use crate::ui::events::{UiEvent, UiPoster};
use crate::ui::input_box::OutgoingRequest;
use crate::util::{Clock, join_until, lock, read, write};

/// 啟動摘要：回報問題時第一眼掌握環境。
pub fn log_startup_summary(cfg: &Config) {
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    log(format!(
        "[app] startup; build={build}, elevated={}, active_language={}, {}",
        py::py_bool(is_elevated()),
        current_language(),
        config_summary(cfg)
    ));
}

/// 搶單一實例；已有一份在跑就喚起它的視窗並回 `None`（呼叫端應直接結束）。
pub fn claim_instance<L>(
    acquire: impl FnOnce() -> Option<L>,
    focus: impl FnOnce(&str) -> bool,
) -> Option<L> {
    let lock = acquire();
    if lock.is_none() {
        let focused = focus(&app_name());
        log(format!(
            "[app] another instance is already running (focused={}), exiting",
            py::py_bool(focused)
        ));
    }
    lock
}

/// 使用者拖過視窗／泡泡就立刻落地，不等按儲存。
pub fn save_field(settings: &RwLock<Config>, path: &Path, key: &str, value: Value) {
    set_then_save(settings, key, value, |cfg| save_config(path, cfg));
}

/// 寫鎖只包住插入：鍵盤 hook 每次 Ctrl+V 都要讀設定，存檔的 I/O 卡住它會被 Windows 拔掉。
fn set_then_save(
    settings: &RwLock<Config>,
    key: &str,
    value: Value,
    save: impl FnOnce(&Config) -> std::io::Result<()>,
) {
    let cfg = {
        let mut cfg = write(settings);
        cfg.insert(key.to_owned(), value);
        cfg.clone()
    };
    if let Err(e) = save(&cfg) {
        log(format!("[config] could not save {key} to config.json: {e}"));
    }
}

/// 收訊那格目前生效的譯文快取指紋；`examples` 為已套用的範例（無則 `None`）。
pub fn incoming_fingerprint(cfg: &Config, examples: Option<&ExampleSet>) -> String {
    let incoming = resolve(cfg, ServiceSlot::Incoming);
    fingerprint_of(
        &incoming.provider,
        &incoming.model,
        &Settings::from_config(cfg).target_language,
        &incoming.base_url,
        &examples.map_or_else(|| "-".to_owned(), ExampleSet::digest),
    )
}

/// 翻譯端：三個用途各一個翻譯器、系統訊息譯文快取、兩條共用總量閘的翻譯池與範例協調器。
pub struct Translation {
    /// 收訊翻譯器（兩條池都用它）。
    pub incoming: Arc<Translator>,
    /// 發話翻譯器。
    pub outgoing: Arc<Translator>,
    /// 框選翻譯器。
    pub region: Arc<Translator>,
    /// 系統訊息譯文快取。
    pub cache: Arc<TranslationCache>,
    /// 玩家對話的翻譯池（吃上下文）。
    pub pool: Arc<TranslationPool>,
    /// 系統訊息的翻譯池（不吃上下文、走快取）。
    pub system_pool: Arc<TranslationPool>,
    /// 範例協調器。
    pub coordinator: Arc<ExampleCoordinator>,
}

/// 建立翻譯端並為收訊與框選備妥範例；兩條池的結果都交給 `deliver(msg_id, text, failed)`。
pub fn build_translation(
    settings: &Arc<RwLock<Config>>,
    deliver: ResultFn,
    rt: &Handle,
    store: Arc<ExampleStore>,
    cache_path: PathBuf,
) -> Translation {
    let (incoming, outgoing, region, gate, fingerprint) = {
        let cfg = settings.read().unwrap_or_else(|e| e.into_inner());
        let target = Settings::from_config(&cfg);
        let make = |slot| Translator::new(resolve(&cfg, slot), target.target_language.clone());
        let incoming = make(ServiceSlot::Incoming);
        // 與磁碟上次存的指紋一致才不會被清掉
        let stored = store.get(&incoming.examples_fingerprint());
        let fingerprint = incoming_fingerprint(&cfg, stored.as_ref());
        (
            incoming,
            make(ServiceSlot::Outgoing),
            make(ServiceSlot::Region),
            Arc::new(ConcurrencyGate::new(target.max_parallel_translations)),
            fingerprint,
        )
    };
    let cache = Arc::new(TranslationCache::new(cache_path, fingerprint));
    cache.load();

    let on_applied = {
        let (settings, cache, incoming) = (
            Arc::clone(settings),
            Arc::clone(&cache),
            Arc::clone(&incoming),
        );
        // 背景生成時在 tokio worker 上執行；rebind 自帶鎖，故不必繞回 UI 執行緒
        move |tr: &Translator| {
            if std::ptr::eq(tr, Arc::as_ptr(&incoming)) {
                let cfg = settings.read().unwrap_or_else(|e| e.into_inner());
                cache.rebind(&incoming_fingerprint(&cfg, tr.examples().as_ref()));
            }
        }
    };
    let coordinator = ExampleCoordinator::new(store, rt.clone(), Arc::new(on_applied));

    let make_pool = |translate: TranslateFn| {
        let tr = Arc::clone(&incoming);
        Arc::new(TranslationPool::new(
            translate,
            Arc::clone(&deliver),
            Arc::clone(&gate),
            Arc::new(|| t("notice.translate_failed", &[])),
            Arc::new(move || tr.describe()),
            rt.clone(),
        ))
    };
    let pool = make_pool({
        let tr = Arc::clone(&incoming);
        Arc::new(move |line, context| {
            let tr = Arc::clone(&tr);
            Box::pin(async move { tr.translate_incoming(&line, &context).await })
        })
    });
    let system_pool = make_pool({
        let (tr, cache) = (Arc::clone(&incoming), Arc::clone(&cache));
        Arc::new(move |line, _context| {
            let (tr, cache) = (Arc::clone(&tr), Arc::clone(&cache));
            Box::pin(async move { translate_and_cache(&tr, &cache, &line).await })
        })
    });
    coordinator.ensure(&incoming);
    coordinator.ensure(&region);
    Translation {
        incoming,
        outgoing,
        region,
        cache,
        pool,
        system_pool,
        coordinator,
    }
}

/// 輸入端各元件共用的狀態。
#[derive(Default)]
pub struct InputWiring {
    /// 各遊戲視窗的聊天輸入框開關。
    pub tracker: Arc<ChatInputTracker>,
    /// 翻譯輸入框是否開著（熱鍵回呼讀）。
    pub box_open: Arc<AtomicBool>,
}

fn write_hotkey(settings: &RwLock<Config>, key: &str, used: String) {
    write(settings).insert(key.to_owned(), Value::String(used));
}

/// 設定裡的逐字鍵入間隔。
pub fn type_delay(cfg: &Config) -> Duration {
    Duration::from_secs_f64(
        cfg.get("type_delay")
            .and_then(Value::as_f64)
            .unwrap_or_default(),
    )
}

/// 依序註冊輸入框熱鍵、框選熱鍵與 Ctrl+V 攔截器（順序同 Python），實際生效的熱鍵寫回 `settings`（不存檔）。
///
/// 回傳（輸入框熱鍵，框選熱鍵）；框選熱鍵與輸入框熱鍵相同時不註冊、回 `None`。
pub fn install_input(
    settings: &Arc<RwLock<Config>>,
    ui: &UiPoster,
    dispatcher: &SharedDispatcher,
    wiring: &InputWiring,
    typing: Arc<TypingEnv>,
) -> (Registration, Option<Registration>) {
    let defaults = default_config();
    let fallback = |key: &str| defaults[key].as_str().unwrap_or_default().to_owned();
    let mask: Arc<dyn Fn() + Send + Sync> = {
        let inject = Arc::clone(&typing.inject);
        Arc::new(move || {
            let inject = Arc::clone(&inject);
            crate::input::hook::after_dispatch(move || inject.mask())
        })
    };

    let hotkey = read(settings)["hotkey"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let on_box_hotkey = {
        let (ui, open) = (ui.clone(), Arc::clone(&wiring.box_open));
        Arc::new(move || on_hotkey(&Foreground::real(), &open, &ui))
    };
    let (box_hotkey, used) = register_hotkey(
        dispatcher,
        &hotkey,
        on_box_hotkey,
        &fallback("hotkey"),
        Arc::clone(&mask),
    );
    write_hotkey(settings, "hotkey", used);

    let region = region_hotkey_to_register(&read(settings));
    let region = region.map(|region| {
        let ui = ui.clone();
        // 選取層是第 3 段的功能，在那之前永遠沒在選取
        let selecting = AtomicBool::new(false);
        let (registration, used) = register_hotkey(
            dispatcher,
            &region,
            Arc::new(move || on_region_hotkey(&Foreground::real(), &selecting, &ui)),
            &fallback("region_hotkey"),
            Arc::clone(&mask),
        );
        write_hotkey(settings, "region_hotkey", used);
        registration
    });

    let should_intercept = {
        let settings = Arc::clone(settings);
        move || should_intercept_paste(&read(&settings), &Foreground::real())
    };
    let on_paste = {
        let (settings, tracker) = (Arc::clone(settings), Arc::clone(&wiring.tracker));
        let foreground = typing.foreground;
        move || {
            on_paste_hotkey(&settings, &tracker, Arc::clone(&typing), foreground);
        }
    };
    let paste = paste_interceptor(
        Box::new(should_intercept),
        Box::new(on_paste),
        Box::new(move || mask()),
    );
    lock(dispatcher).add_blocking(Box::new(paste));
    (box_hotkey, region)
}

/// 攔到遊戲內的 Ctrl+V：記下當下的前景視窗，交給背景執行緒鍵入剪貼簿（回傳該執行緒）。
///
/// 那個客戶端的聊天輸入框開著時走單行模式（換行改空格），其他地方換行照打。
pub fn on_paste_hotkey(
    settings: &RwLock<Config>,
    tracker: &ChatInputTracker,
    typing: Arc<TypingEnv>,
    foreground: fn() -> isize,
) -> JoinHandle<()> {
    let hwnd = foreground();
    let delay = type_delay(&read(settings));
    let single_line = tracker.is_open(hwnd);
    std::thread::spawn(move || {
        paste_clipboard(&typing, hwnd, delay, single_line);
    })
}

/// 翻譯輸入框的 `translate`：在 tokio 上翻成發話語言，結果以 `UiEvent::OutgoingDone` 送回。
///
/// 上下文取譯文要打回的那個客戶端目前分頁的聊天；取消時只記 log、不送結果。
pub fn outgoing_translate(
    translator: Arc<Translator>,
    contexts: ContextMap,
    rt: Handle,
    ui: UiPoster,
) -> Box<dyn Fn(OutgoingRequest)> {
    Box::new(move |req| {
        let context = lock(&contexts)
            .get(&req.target)
            .map(|client| client.current().snapshot())
            .unwrap_or_default();
        let (tr, ui) = (Arc::clone(&translator), ui.clone());
        rt.spawn(async move {
            let chars = req.text.chars().count();
            let result = match tr
                .translate_outgoing(&req.text, &context, Some(&req.cancel))
                .await
            {
                Ok(translated) => Ok(translated),
                Err(TranslateError::Cancelled) => {
                    log(format!(
                        "[input] outgoing translation cancelled (chars={chars})"
                    ));
                    return;
                }
                Err(err) => {
                    log(format!(
                        "[input] outgoing translation failed (chars={chars}, {}): {}: {err}",
                        tr.describe(),
                        err.class_name()
                    ));
                    Err(t("input.failed", &[("error", &err.to_string())]))
                }
            };
            ui.post(UiEvent::OutgoingDone {
                session: req.session,
                result,
            });
        });
    })
}

static CONSOLE_QUIT: OnceLock<UiPoster> = OnceLock::new();

/// 主控台的 Ctrl+C 改投遞 `UiEvent::Quit`，走與 ✕ 相同的關閉流程；其他主控台事件照預設處理。
pub fn install_console_quit(ui: UiPoster) {
    if CONSOLE_QUIT.set(ui).is_err() {
        return;
    }
    // SAFETY: 回呼是 static 函式，程序存活期間都有效。
    if unsafe { SetConsoleCtrlHandler(Some(console_ctrl), TRUE) } == 0 {
        log(format!(
            "[app] cannot install console Ctrl+C handler: {}",
            std::io::Error::last_os_error()
        ));
    }
}

unsafe extern "system" fn console_ctrl(ctrl: u32) -> BOOL {
    static POSTED: AtomicBool = AtomicBool::new(false);
    match CONSOLE_QUIT.get() {
        Some(ui) if quit_on_ctrl_c(ctrl, ui, &POSTED) => TRUE,
        _ => FALSE,
    }
}

/// 第二次 Ctrl+C 回 false 交給預設處理（結束程序）：UI 卡住時才關得掉。
fn quit_on_ctrl_c(ctrl: u32, ui: &UiPoster, posted: &AtomicBool) -> bool {
    if ctrl != CTRL_C_EVENT {
        return false;
    }
    if posted.swap(true, Ordering::SeqCst) {
        log("[app] Ctrl+C received again; terminating without the normal shutdown");
        return false;
    }
    log("[app] Ctrl+C received; closing via the normal shutdown");
    ui.post(UiEvent::Quit);
    true
}

/// `build_app` 接好線、關閉流程需要的執行期物件。
pub struct Running {
    /// 程式關閉的訊號。
    pub stop: CancellationToken,
    /// 翻譯端。
    pub translation: Translation,
    /// supervisor 執行緒（關閉時等它把每條收訊執行緒收完）；起不了時為 `None`。
    pub supervisor: Option<JoinHandle<()>>,
    /// 低階鍵盤 hook；裝不上時為 `None`。
    pub keyboard: Option<KeyboardHook>,
    /// 各客戶端（hwnd）的聊天上下文。
    pub contexts: ContextMap,
    /// 輸入端共用狀態。
    pub input: InputWiring,
    /// 譯文與貼上共用的鍵入環境。
    pub typing: Arc<TypingEnv>,
}

/// 建構翻譯端、註冊熱鍵並裝上鍵盤 hook、啟動 supervisor 執行緒；`messages` 是 messages.log 的串流，每個客戶端各建一個 `MessageLog` 共寫。
pub fn build_app(
    settings: Arc<RwLock<Config>>,
    rt: &Handle,
    ui: UiPoster,
    messages: SharedWriter,
) -> Running {
    let deliver: ResultFn = {
        let ui = ui.clone();
        Arc::new(move |msg_id, translated, failed| {
            ui.post(UiEvent::UpdateMessage {
                msg_id,
                translated,
                failed,
            })
        })
    };
    let translation = build_translation(
        &settings,
        deliver,
        rt,
        Arc::new(ExampleStore::default()),
        crate::translation::cache::default_path(),
    );
    let dispatcher = SharedDispatcher::default();
    let input = InputWiring::default();
    let typing = {
        let d = Arc::clone(&dispatcher);
        // 複製出來立刻放鎖：鍵入時注入的按鍵會回到 hook，hook 要拿同一把鎖
        Arc::new(TypingEnv::real(Arc::new(move || {
            lock(&d).pressed().clone()
        })))
    };
    install_input(&settings, &ui, &dispatcher, &input, Arc::clone(&typing));
    let keyboard = hook::install(dispatcher)
        .inspect_err(|err| log(format!("[hotkey] cannot install keyboard hook: {err}")))
        .ok();
    let stop = CancellationToken::new();
    let board = Arc::new(StatusBoard::new(
        ui.clone(),
        Arc::clone(&translation.pool),
        Arc::clone(&translation.system_pool),
    ));
    let shared = Arc::new(ReaderShared {
        settings,
        ui,
        stop: stop.clone(),
        pool: Arc::clone(&translation.pool),
        system_pool: Arc::clone(&translation.system_pool),
        board: Arc::clone(&board),
        msg_ids: Arc::new(MessageIds::default()),
        cache: Arc::clone(&translation.cache),
    });
    let contexts = ContextMap::default();
    let spawn_contexts = Arc::clone(&contexts);
    let spawn = move |hwnd: isize, slot: u32| {
        let mut reader = WizChatReader::new(
            hwnd,
            Box::new(|pid| Ok(Box::new(WinProcess::open(pid)?) as Box<dyn ProcessMemory>)),
        );
        reader.slot = slot;
        reader.message_log = Some(Box::new(MessageLog::new(Arc::clone(&messages), slot)));
        let shared = Arc::clone(&shared);
        let client = Arc::new(ClientContexts::new());
        lock(&spawn_contexts).insert(hwnd, Arc::clone(&client));
        let name = format!("reader-{slot}");
        std::thread::Builder::new()
            .name(name.clone())
            .spawn(move || reader_loop(shared, hwnd, slot, reader, client))
            .inspect_err(|e| log(format!("[reader] cannot spawn {name}: {e}")))
            .ok()
    };
    let supervisor = {
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("reader-supervisor".into())
            .spawn(move || supervise(&stop, spawn, game_windows, &board))
            .inspect_err(|e| log(format!("[reader] cannot spawn reader-supervisor: {e}")))
            .ok()
    };
    Running {
        stop,
        translation,
        supervisor,
        keyboard,
        contexts,
        input,
        typing,
    }
}

/// 等 supervisor 的上限：多 2 秒讓它自己的 `JOIN_TIMEOUT` 先跑完，「still alive」的診斷 log 才寫得出來。
pub const SUPERVISOR_JOIN_BUDGET: Duration = JOIN_TIMEOUT.saturating_add(Duration::from_secs(2));

/// 關閉流程本體（不退出程序）：設停止 → 關翻譯池 → 卸鍵盤 hook → 等 supervisor 收完 reader（最多 `join_budget`）→ 落盤快取。
pub fn shutdown_steps(
    stop: &CancellationToken,
    close_pools: impl FnOnce(),
    unhook_keyboard: impl FnOnce(),
    supervisor: Option<JoinHandle<()>>,
    join_budget: Duration,
    flush_cache: impl FnOnce(),
) {
    log("[app] shutting down, waiting for readers to unhook");
    stop.cancel();
    close_pools();
    unhook_keyboard();
    if let Some(supervisor) = supervisor {
        let deadline = Instant::now() + join_budget;
        if !join_until(supervisor, deadline, &Clock::default()) {
            log(format!(
                "[app] reader supervisor still running after {:.1}s; continuing shutdown",
                join_budget.as_secs_f64()
            ));
        }
    }
    // 放在池與 reader 之後，飛行中的翻譯才有機會先寫完 put()
    flush_cache();
    log("[app] shutdown complete");
}

/// 乾淨關閉後直接結束程序：卡在 HTTP 請求的翻譯 task 不該把程序多留 60 秒。
pub fn shutdown(running: Running) -> ! {
    let Running {
        stop,
        translation,
        supervisor,
        keyboard,
        ..
    } = running;
    shutdown_steps(
        &stop,
        || {
            translation.pool.shutdown();
            translation.system_pool.shutdown();
        },
        || {
            if let Some(hook) = keyboard {
                hook.uninstall();
            }
        },
        supervisor,
        SUPERVISOR_JOIN_BUDGET,
        || translation.cache.flush(),
    );
    std::process::exit(0)
}

/// 跑 UI 直到結束（含 panic），之後一律走 `then`（關閉流程）。
///
/// winit 會把 UI 回呼裡的 panic 拋出 `run_native`；不攔下就跳過 unhook 與快取落盤。
pub fn run_ui_then(ui: impl FnOnce(), then: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(ui)).is_err() {
        log("[ui] overlay panicked; shutting down");
    }
    then();
}

/// messages.log 的串流；開不了檔時丟棄寫入（windowed 模式沒有主控台，不能讓它把程式帶掉）。
pub fn open_messages_log(path: &Path) -> SharedWriter {
    match crate::logs::open_session_log(path, chrono::Utc::now()) {
        Ok(file) => Arc::new(Mutex::new(crate::logs::TimestampedWriter::new(file))),
        Err(e) => {
            log(format!("[log] cannot open messages.log: {e}"));
            Arc::new(Mutex::new(std::io::sink())) as Arc<Mutex<dyn Write + Send>>
        }
    }
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::System::Console::CTRL_BREAK_EVENT;

    use super::*;

    #[test]
    fn only_ctrl_c_posts_quit() {
        let (ui, rx) = UiPoster::for_test();
        let posted = AtomicBool::new(false);
        assert!(!quit_on_ctrl_c(CTRL_BREAK_EVENT, &ui, &posted));
        assert!(rx.try_recv().is_err());
        assert!(quit_on_ctrl_c(CTRL_C_EVENT, &ui, &posted));
        assert_eq!(rx.try_recv(), Ok(UiEvent::Quit));
    }

    #[test]
    fn second_ctrl_c_falls_through_to_default_termination() {
        let (ui, rx) = UiPoster::for_test();
        let posted = AtomicBool::new(false);
        assert!(quit_on_ctrl_c(CTRL_C_EVENT, &ui, &posted));
        assert!(!quit_on_ctrl_c(CTRL_C_EVENT, &ui, &posted));
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), [UiEvent::Quit]);
    }

    #[test]
    fn save_field_releases_the_settings_lock_before_saving() {
        let settings = RwLock::new(Config::new());
        set_then_save(&settings, "k", Value::from(1), |cfg| {
            assert!(settings.try_write().is_ok(), "lock held during save");
            assert_eq!(cfg["k"], 1);
            Ok(())
        });
        assert_eq!(read(&settings)["k"], 1);
    }
}
