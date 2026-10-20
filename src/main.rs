//! Wizard101 聊天翻譯器的程式進入點：收訊（supervisor 執行緒）＋翻譯（tokio）＋ overlay（eframe 主迴圈）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, RwLock};

use serde_json::json;
use tokio::runtime::Handle;
use wizard101_chat_translator::app_main::{
    Running, WinRegionDesktop, apply_settings, build_app, claim_instance, install_console_quit,
    log_startup_summary, open_messages_log, outgoing_translate, region_capturer, region_runner,
    report_save, run_ui_then, save_field, shutdown, type_delay,
};
use wizard101_chat_translator::composer::paste::type_into_window;
use wizard101_chat_translator::config::{
    Config, Settings, app_dir, bootstrap_language, config_path, is_configured, load_config,
    local_state_dir, save_config,
};
use wizard101_chat_translator::i18n::set_language;
use wizard101_chat_translator::input::dispatch::GameInputEvents;
use wizard101_chat_translator::input::dispatcher::SharedDispatcher;
use wizard101_chat_translator::input::hook::{self, KeyboardHook};
use wizard101_chat_translator::logs::{self, Sink, log};
use wizard101_chat_translator::reader::message_log::SharedWriter;
use wizard101_chat_translator::reader::process::{foreground_window, pid_alive};
use wizard101_chat_translator::region::ocr::{self, Models, register_models};
use wizard101_chat_translator::region::pipeline::RegionPipeline;
use wizard101_chat_translator::runtime::updater::{
    LATEST_API, UPDATE_CHECK_INTERVAL, schedule_update_checks,
};
use wizard101_chat_translator::runtime::{
    AcquireError, acquire_single_instance, focus_running_instance,
};
use wizard101_chat_translator::ui::app::{self, App, OnSave};
use wizard101_chat_translator::ui::events::{UiEvent, UiPoster};
use wizard101_chat_translator::ui::input_box::InputBox;
use wizard101_chat_translator::ui::overlay::frame::Frame;
use wizard101_chat_translator::ui::region::flow::RegionFlow;
use wizard101_chat_translator::ui::run;
use wizard101_chat_translator::ui::settings::env::RealEnv;
use wizard101_chat_translator::ui::settings_host::{SettingsHost, WizardHost};
use wizard101_chat_translator::ui::win32::cursor_pos;
use wizard101_chat_translator::util::{read, sweep_stale_temps, write};

fn main() {
    // rten 預設用滿實體核心、框選 OCR 時與遊戲搶 CPU；它只在第一次建立執行緒池時讀這個變數
    let ocr_threads = ocr::inference_threads(
        std::env::var_os("RTEN_NUM_THREADS").is_some(),
        std::thread::available_parallelism().map_or(1, usize::from),
    );
    if let Some(threads) = ocr_threads {
        // SAFETY: 還沒建立任何執行緒，沒有人同時讀寫環境變數
        unsafe { std::env::set_var("RTEN_NUM_THREADS", threads.to_string()) };
    }
    // 先搶鎖再開 app.log：第二份程式不可重寫執行中那份正在用的檔案
    let lock = acquire_single_instance();
    if !matches!(lock, Err(AcquireError::AlreadyRunning)) && !cfg!(debug_assertions) {
        match logs::open_session_log(&app_dir().join("app.log"), chrono::Utc::now()) {
            Ok(file) => logs::init(Sink::File(file)),
            Err(e) => log(format!("[log] cannot open app.log: {e}")),
        }
    }
    logs::install_panic_hook();
    logs::forward_library_logs();
    // 版本先印：app.log 分段標頭後第一行就是版本
    log(format!("[app] version={}", env!("CARGO_PKG_VERSION")));
    log(format!(
        "[region] OCR inference threads: RTEN_NUM_THREADS={} ({})",
        std::env::var("RTEN_NUM_THREADS").unwrap_or_default(),
        if ocr_threads.is_some() {
            "default, half the logical cores"
        } else {
            "set by user"
        }
    ));
    if let Err(AcquireError::CreateFailed(code)) = &lock {
        log(format!("[app] CreateMutex failed: error={code}"));
    }

    let path = config_path();
    let config_existed = path.exists();
    let mut cfg = load_config(&path);
    // 介面語言要在建立任何視窗之前定案：文案與字型都由它決定
    set_language(&bootstrap_language(&mut cfg, config_existed));

    // 鎖要留到程序結束：一放掉 mutex 就放行下一份
    let Some(_instance_lock) = claim_instance(|| lock.ok(), focus_running_instance) else {
        return;
    };
    for dir in [app_dir(), local_state_dir()] {
        sweep_stale_temps(&dir, pid_alive);
    }
    let messages = open_messages_log(&app_dir().join("messages.log"));
    register_models(Models {
        det: include_bytes!("../assets/ocr/PP-OCRv6_det_small.onnx"),
        rec: include_bytes!("../assets/ocr/PP-OCRv6_rec_small.onnx"),
        cls: include_bytes!("../assets/ocr/ch_ppocr_mobile_v2.0_cls_mobile.onnx"),
        keys: include_bytes!("../assets/ocr/ppocrv6_keys.txt"),
    });
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    let dispatcher = SharedDispatcher::default();
    let (poster, inbox) = app::channel();
    let hotkeys_failed = {
        let ui = poster.clone();
        move || ui.post(UiEvent::HotkeysUnavailable)
    };
    let keyboard = Rc::new(RefCell::new(
        hook::install(Arc::clone(&dispatcher), hotkeys_failed.clone())
            .inspect_err(|err| {
                log(format!("[hotkey] cannot install keyboard hook: {err}"));
                hotkeys_failed();
            })
            .ok(),
    ));

    let configured = is_configured(&cfg);
    let settings = Settings::from_config(&cfg);
    let shared_cfg = Arc::new(RwLock::new(cfg));
    install_console_quit(poster.clone());
    // 關閉流程只跑一次：正常關閉在視窗還在時由 App 觸發，UI 意外結束（panic、錯誤）才由 run_ui_then 補跑
    let running = Rc::new(RefCell::new(None));
    let startup = Startup {
        cfg: Arc::clone(&shared_cfg),
        rt: rt.handle().clone(),
        ui: poster.clone(),
        messages,
        dispatcher: Arc::clone(&dispatcher),
        keyboard: Rc::clone(&keyboard),
        running: Rc::clone(&running),
    };
    let wizard_rt = rt.handle().clone();
    if !configured {
        log("[app] config incomplete, launching first-run wizard");
    }

    let ui = move || {
        let result = run::run_with_warp_fallback(configured, move |ctx| {
            let geometry_cfg = Arc::clone(&shared_cfg);
            let geometry_ui = startup.ui.clone();
            let frame = Frame::new(
                settings.overlay_alpha,
                Some(settings.overlay.clone()),
                move |x, y, width, height| {
                    let value = json!({"x": x, "y": y, "width": width, "height": height});
                    save_field(
                        &geometry_cfg,
                        &config_path(),
                        "overlay",
                        value,
                        &geometry_ui,
                    );
                },
            );
            let mut app = App::new(ctx, inbox, frame);
            if configured {
                start_main(&mut app, startup);
                return app;
            }
            let env = RealEnv::new(wizard_rt, poster, dispatcher, None);
            let mut wizard = WizardHost::new(Box::new(env));
            wizard.open(&read(&shared_cfg));
            app.with_first_run(
                wizard,
                Box::new(move |app, cfg| {
                    let cfg = {
                        let mut shared = write(&startup.cfg);
                        *shared = cfg;
                        shared.clone()
                    };
                    let saved = save_config(&config_path(), &cfg);
                    match &saved {
                        Ok(()) => log("[app] wizard completed, config saved"),
                        Err(e) => log(format!(
                            "[app] wizard completed but config.json could not be saved: {e}"
                        )),
                    }
                    report_save(&startup.ui, saved.is_ok());
                    start_main(app, startup);
                }),
            )
        });
        if let Err(e) = result {
            log(format!("[ui] overlay exited with an error: {e}"));
        }
    };
    run_ui_then(ui, || {
        if let Some(running) = running.borrow_mut().take() {
            shutdown(running)
        }
        // 精靈中途取消：主流程沒接手，hook 還在這裡
        if let Some(keyboard) = keyboard.borrow_mut().take() {
            keyboard.uninstall();
        }
    });
}

/// 接線要用、在精靈完成前就備妥的東西；設定完整時開窗即交給 [`start_main`]，否則等精靈完成。
struct Startup {
    cfg: Arc<RwLock<Config>>,
    rt: Handle,
    ui: UiPoster,
    messages: SharedWriter,
    dispatcher: SharedDispatcher,
    /// 主流程接手前留在這裡；精靈取消時由收尾卸除。
    keyboard: Rc<RefCell<Option<KeyboardHook>>>,
    running: Rc<RefCell<Option<Running>>>,
}

/// 依共用設定接上其餘功能並開始執行：翻譯與收訊、輸入框、框選、泡泡、設定視窗、更新檢查與關閉流程。
fn start_main(app: &mut App, startup: Startup) {
    let Startup {
        cfg: shared_cfg,
        rt,
        ui,
        messages,
        dispatcher,
        keyboard,
        running,
    } = startup;
    log_startup_summary(&read(&shared_cfg));
    let built = build_app(
        Arc::clone(&shared_cfg),
        &rt,
        ui.clone(),
        messages,
        Arc::clone(&dispatcher),
        keyboard.borrow_mut().take(),
    );
    let settings = Settings::from_config(&read(&shared_cfg));
    let send = {
        let (input_cfg, typing) = (Arc::clone(&shared_cfg), Arc::clone(&built.typing));
        // 在背景打字，UI 不凍結；TYPING 鎖保證不與貼上交錯
        move |text: String, hwnd: isize| {
            let delay = type_delay(&read(&input_cfg));
            let typing = Arc::clone(&typing);
            let _ = std::thread::Builder::new()
                .name("type-outgoing".into())
                .spawn(move || type_into_window(&typing, hwnd, &text, delay))
                .inspect_err(|e| log(format!("[composer] cannot spawn type-outgoing: {e}")));
        }
    };
    let input_box = InputBox::new(
        outgoing_translate(
            Arc::clone(&built.translation.outgoing),
            Arc::clone(&built.contexts),
            rt.clone(),
            ui.clone(),
        ),
        Box::new(send),
        Arc::clone(&built.input.box_open),
        foreground_window,
        cursor_pos,
    );
    let region_flow = RegionFlow::new(
        Box::new(WinRegionDesktop),
        region_capturer(ui.clone()),
        region_runner(
            RegionPipeline::new(
                Arc::clone(&built.translation.region),
                Arc::new(ocr::recognize),
            ),
            rt.clone(),
            ui.clone(),
        ),
        Arc::clone(&built.input.selecting),
    );
    let input_events = GameInputEvents::new(Arc::clone(&built.input.tracker), foreground_window);
    app.set_input(input_box, input_events, Arc::clone(&shared_cfg));
    app.set_region(region_flow);
    let bubble_cfg = Arc::clone(&shared_cfg);
    let bubble_ui = ui.clone();
    app.set_bubble_position(settings.bubble_position, move |x, y| {
        save_field(
            &bubble_cfg,
            &config_path(),
            "bubble_position",
            json!({"x": x, "y": y}),
            &bubble_ui,
        )
    });

    let env = RealEnv::new(
        rt.clone(),
        ui.clone(),
        dispatcher,
        Some(Arc::clone(&built.translation.cache)),
    );
    let on_save: OnSave = {
        let (running, settings, ui) = (Rc::clone(&running), Arc::clone(&shared_cfg), ui.clone());
        Box::new(move |new_cfg| {
            let mut running = running.borrow_mut();
            // shutdown 結束程序，所以 App 還在跑時 running 一定還在
            let running = running.as_mut().expect("running until shutdown exits");
            apply_settings(running, &settings, new_cfg, &ui)
        })
    };
    app.set_settings(
        SettingsHost::new(Box::new(env)),
        Arc::clone(&shared_cfg),
        on_save,
    );
    schedule_update_checks(
        &rt,
        built.stop.clone(),
        LATEST_API.to_owned(),
        UPDATE_CHECK_INTERVAL,
        Arc::new(move |release| ui.post(UiEvent::UpdateFound(release))),
    );
    let closing = Rc::clone(&running);
    app.set_on_close(move || {
        if let Some(running) = closing.borrow_mut().take() {
            shutdown(running)
        }
    });

    let (hotkey, region_hotkey) = {
        let cfg = read(&shared_cfg);
        let field = |key: &str| cfg[key].as_str().unwrap_or_default().to_owned();
        (field("hotkey"), field("region_hotkey"))
    };
    app.set_limits(settings.max_messages, settings.fade_seconds.into());
    app.set_region_hotkey(&region_hotkey);
    *running.borrow_mut() = Some(built);
    log(format!(
        "[app] running; hotkey={hotkey} opens the input box, region_hotkey={region_hotkey} starts region translation; quit via the overlay ✕"
    ));
}
