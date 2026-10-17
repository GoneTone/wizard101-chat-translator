//! 主程式接線：翻譯端（翻譯器、快取、兩條翻譯池、範例）、收訊端（supervisor）與關閉流程。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use crate::config::services::resolve;
use crate::config::{Config, ServiceSlot, Settings, app_name, config_summary, py, save_config};
use crate::i18n::{current_language, t};
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
use crate::translation::context::ClientContexts;
use crate::translation::examples::{ExampleSet, ExampleStore};
use crate::translation::gate::ConcurrencyGate;
use crate::translation::pool::{ResultFn, TranslateFn, TranslationPool};
use crate::translation::translator::{ExampleCoordinator, Translator};
use crate::ui::events::{UiEvent, UiPoster};

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
    let mut cfg = settings.write().unwrap_or_else(|e| e.into_inner());
    cfg.insert(key.to_owned(), value);
    if let Err(e) = save_config(path, &cfg) {
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

/// `build_app` 接好線、關閉流程需要的執行期物件。
pub struct Running {
    /// 程式關閉的訊號。
    pub stop: CancellationToken,
    /// 翻譯端。
    pub translation: Translation,
    /// supervisor 執行緒（關閉時等它把每條收訊執行緒收完）。
    pub supervisor: JoinHandle<()>,
}

/// 建構翻譯端並啟動 supervisor 執行緒；`messages` 是 messages.log 的串流，每個客戶端各建一個 `MessageLog` 共寫。
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
    let spawn = move |hwnd: isize, slot: u32| {
        let mut reader = WizChatReader::new(
            hwnd,
            Box::new(|pid| Ok(Box::new(WinProcess::open(pid)?) as Box<dyn ProcessMemory>)),
        );
        reader.slot = slot;
        reader.message_log = Some(Box::new(MessageLog::new(Arc::clone(&messages), slot)));
        let shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name(format!("reader-{slot}"))
            .spawn(move || reader_loop(shared, hwnd, slot, reader, Arc::new(ClientContexts::new())))
            .expect("spawn reader thread")
    };
    let supervisor = {
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("reader-supervisor".into())
            .spawn(move || supervise(&stop, spawn, game_windows, &board))
            .expect("spawn supervisor thread")
    };
    Running {
        stop,
        translation,
        supervisor,
    }
}

/// 關閉流程本體（不退出程序）：設停止 → 關翻譯池 → 等 supervisor 收完 reader → 落盤快取。
pub fn shutdown_steps(
    stop: &CancellationToken,
    close_pools: impl FnOnce(),
    supervisor: JoinHandle<()>,
    flush_cache: impl FnOnce(),
) {
    log("[app] shutting down, waiting for readers to unhook");
    stop.cancel();
    close_pools();
    // 多 2 秒讓 supervisor 自己的 JOIN_TIMEOUT 先跑完，「still alive」的診斷 log 才寫得出來
    let deadline = Instant::now() + JOIN_TIMEOUT + Duration::from_secs(2);
    while !supervisor.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if supervisor.is_finished() {
        let _ = supervisor.join();
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
    } = running;
    shutdown_steps(
        &stop,
        || {
            translation.pool.shutdown();
            translation.system_pool.shutdown();
        },
        supervisor,
        || translation.cache.flush(),
    );
    std::process::exit(0)
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
