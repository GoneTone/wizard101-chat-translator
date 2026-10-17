//! Wizard101 聊天翻譯器的程式進入點：收訊（supervisor 執行緒）＋翻譯（tokio）＋ overlay（eframe 主迴圈）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, RwLock};

use serde_json::json;
use wizard101_chat_translator::app_main::{
    build_app, claim_instance, log_startup_summary, open_messages_log, run_ui_then, save_field,
    shutdown,
};
use wizard101_chat_translator::config::{
    Settings, app_dir, bootstrap_language, config_path, is_configured, load_config,
};
use wizard101_chat_translator::i18n::set_language;
use wizard101_chat_translator::logs::{self, Sink, log};
use wizard101_chat_translator::runtime::{acquire_single_instance, focus_running_instance};
use wizard101_chat_translator::ui::app::{self, App};
use wizard101_chat_translator::ui::overlay::frame::Frame;

fn main() {
    if !cfg!(debug_assertions) {
        match logs::open_session_log(&app_dir().join("app.log"), chrono::Utc::now()) {
            Ok(file) => logs::init(Sink::File(file)),
            Err(e) => log(format!("[log] cannot open app.log: {e}")),
        }
    }
    logs::install_panic_hook();
    logs::forward_library_logs();
    // 版本先印：app.log 分段標頭後第一行就是版本
    log(format!("[app] version={}", env!("CARGO_PKG_VERSION")));

    let path = config_path();
    let config_existed = path.exists();
    let mut cfg = load_config(&path);
    // 介面語言要在建立任何視窗之前定案：文案與字型都由它決定
    set_language(&bootstrap_language(&mut cfg, config_existed));
    log_startup_summary(&cfg);

    // 鎖要留到程序結束：一放掉 mutex 就放行下一份
    let Some(_instance_lock) = claim_instance(acquire_single_instance, focus_running_instance)
    else {
        return;
    };
    let messages = open_messages_log(&app_dir().join("messages.log"));

    if !is_configured(&cfg) {
        log("[app] config incomplete; first-run wizard arrives in phase 4");
        return;
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    let settings = Settings::from_config(&cfg);
    let hotkey = cfg["hotkey"].as_str().unwrap_or_default().to_owned();
    let region_hotkey = cfg["region_hotkey"].as_str().unwrap_or_default().to_owned();
    let shared_cfg = Arc::new(RwLock::new(cfg));
    let (poster, inbox) = app::channel();
    let running = build_app(Arc::clone(&shared_cfg), rt.handle(), poster, messages);
    log(format!(
        "[app] running; hotkey={hotkey} opens the input box, region_hotkey={region_hotkey} starts region translation; quit via the overlay ✕"
    ));

    let ui = move || {
        let result = app::run_with_warp_fallback(move |ctx| {
            let geometry_cfg = Arc::clone(&shared_cfg);
            let frame = Frame::new(
                settings.overlay_alpha,
                Some(settings.overlay.clone()),
                move |x, y, width, height| {
                    let value = json!({"x": x, "y": y, "width": width, "height": height});
                    save_field(&geometry_cfg, &config_path(), "overlay", value);
                },
            );
            let mut app = App::new(ctx, inbox, frame).with_bubble_position(
                settings.bubble_position,
                move |x, y| {
                    save_field(
                        &shared_cfg,
                        &config_path(),
                        "bubble_position",
                        json!({"x": x, "y": y}),
                    )
                },
            );
            app.set_limits(settings.max_messages, settings.fade_seconds.into());
            app.set_region_hotkey(&region_hotkey);
            app
        });
        if let Err(e) = result {
            log(format!("[ui] overlay exited with an error: {e}"));
        }
    };
    run_ui_then(ui, || shutdown(running));
}
