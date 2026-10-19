//! App 托管設定視窗與首次設定精靈：⚙ 開窗、儲存與取消套到 overlay、精靈模式藏根視窗、視窗的置頂與前景。
//!
//! 會切換全域介面語言，獨立成一個測試執行檔，檔內以鎖排隊；視窗操作一律換成替身，不碰真的桌面。

mod common;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex, RwLock};

use eframe::egui::accesskit::Role;
use eframe::egui::{self, ViewportCommand, ViewportEvent, ViewportId, WindowLevel};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use common::kittest::{click, has_label, run, tr};
use common::language_lock;
use common::log_capture::{capture_logs, log_lines_with};
use common::settings_env::FakeEnv;
use common::settings_window::custom_cfg;
use common::wizard::configured_cfg as wizard_cfg;
use wizard101_chat_translator::config::services::ServiceSpec;
use wizard101_chat_translator::config::{Config, app_name};
use wizard101_chat_translator::i18n::{self, current_language, language_name, t};
use wizard101_chat_translator::runtime::updater::Release;
use wizard101_chat_translator::ui::app::{App, Inbox, channel};
use wizard101_chat_translator::ui::events::{UiEvent, UiPoster};
use wizard101_chat_translator::ui::fonts_egui;
use wizard101_chat_translator::ui::overlay::frame::Frame;
use wizard101_chat_translator::ui::settings::env::{SettingsEnv, SettingsEvent};
use wizard101_chat_translator::ui::settings::theme;
use wizard101_chat_translator::ui::settings_host::{
    SettingsHost, WindowOps, WizardHost, initial_size,
};

/// 替身找到的設定視窗／精靈 HWND。
const FAKE_HWND: isize = 0x5E77;

thread_local! {
    static RAISED: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
    static DARKENED: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
}

/// 不碰桌面的視窗操作：新視窗一律認成 `FAKE_HWND`，搶前景與深色標題列只記下對象。
fn fake_ops() -> WindowOps {
    RAISED.with(|r| r.borrow_mut().clear());
    DARKENED.with(|r| r.borrow_mut().clear());
    WindowOps {
        windows_before: |_| Vec::new(),
        find_new: |_, known, _| {
            if known.contains(&FAKE_HWND) {
                0
            } else {
                FAKE_HWND
            }
        },
        dark_caption: |hwnd| DARKENED.with(|r| r.borrow_mut().push(hwnd)),
        force_foreground: |hwnd| {
            RAISED.with(|r| r.borrow_mut().push(hwnd));
            true
        },
        cursor_work_area: || ((0, 0, 1920, 1040), 1.0),
    }
}

fn raised() -> Vec<isize> {
    RAISED.with(|r| r.borrow().clone())
}

/// 把工作交給共用的 `FakeEnv`，測試才看得到 host 發起了什麼。
struct SharedEnv(Rc<FakeEnv>);

impl SettingsEnv for SharedEnv {
    fn test_connection(&self, round: u64, spec: ServiceSpec, target: String) {
        self.0.test_connection(round, spec, target);
    }
    fn list_models(&self, round: u64, spec: ServiceSpec) {
        self.0.list_models(round, spec);
    }
    fn check_update(&self, round: u64) {
        self.0.check_update(round);
    }
    fn record_hotkey(&self, round: u64) {
        self.0.record_hotkey(round);
    }
    fn cancel_recording(&self) {
        self.0.cancel_recording();
    }
    fn open_folder(&self, path: &Path) {
        self.0.open_folder(path);
    }
    fn has_cache(&self) -> bool {
        self.0.has_cache()
    }
    fn clear_cache(&self) -> usize {
        self.0.clear_cache()
    }
}

/// 設定完整、透明度 0.5、訊息上限 50 的設定（與 overlay 目前的 0.8／200 不同）。
fn settings_cfg() -> Config {
    let mut cfg = custom_cfg();
    cfg.insert("ui_language".into(), json!(current_language()));
    cfg.insert("overlay_alpha".into(), json!(0.5));
    cfg.insert("max_messages".into(), json!(50));
    cfg.insert("fade_seconds".into(), json!(30));
    cfg
}

type Saved = Rc<RefCell<Vec<Config>>>;

struct Wired {
    env: Rc<FakeEnv>,
    saved: Saved,
    poster: UiPoster,
}

/// 記下每個 pass 結束時根 viewport 的指令與累計的搶前景次數；kittest 建 harness 時就會跑幀，只有這樣看得到第一幀。
#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<Pass>>>);

/// 一個 pass：幀編號、根 viewport 的指令、累計的搶前景次數。
type Pass = (u64, Vec<ViewportCommand>, usize);

impl egui::Plugin for Recorder {
    fn debug_name(&self) -> &'static str {
        "settings_host recorder"
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        let commands = output
            .viewport_output
            .get(&ViewportId::ROOT)
            .map(|v| v.commands.clone())
            .unwrap_or_default();
        let entry = (ctx.cumulative_frame_nr(), commands, raised().len());
        self.0.lock().unwrap().push(entry);
    }
}

impl Recorder {
    /// 每一幀（同一幀的 pass 併在一起）根 viewport 的指令與那一幀結束時累計的搶前景次數。
    fn frames(&self) -> Vec<(Vec<ViewportCommand>, usize)> {
        let mut frames: Vec<Pass> = Vec::new();
        for (nr, commands, raised) in self.0.lock().unwrap().iter() {
            match frames.last_mut() {
                Some(last) if last.0 == *nr => {
                    last.1.extend(commands.iter().cloned());
                    last.2 = *raised;
                }
                _ => frames.push((*nr, commands.clone(), *raised)),
            }
        }
        frames.into_iter().map(|(_, c, r)| (c, r)).collect()
    }
}

fn eframe_harness(
    build: impl FnOnce(&egui::Context, Inbox) -> App + 'static,
) -> (Harness<'static, App>, UiPoster, Recorder) {
    let (poster, inbox) = channel();
    let recorder = Recorder::default();
    let plugin = recorder.clone();
    let harness = Harness::builder()
        .with_size(egui::vec2(1920.0, 1080.0))
        .with_context_setup(move |ctx| ctx.add_plugin(plugin))
        .build_eframe(move |cc| {
            fonts_egui::install(&cc.egui_ctx, &current_language());
            build(&cc.egui_ctx, inbox)
        });
    (harness, poster, recorder)
}

/// 接上設定視窗的 App：`cfg` 是共用設定，`on_save` 記下收到的設定。
///
/// overlay 起始透明度是 `alpha`；`on_save` 回報的框選熱鍵是 `region`（`None`＝沒註冊）。
fn settings_app_with(
    cfg: Config,
    alpha: f32,
    region: Option<&'static str>,
) -> (Harness<'static, App>, Wired) {
    let env = Rc::new(FakeEnv::default());
    let saved = Saved::default();
    let (host_env, sink) = (Rc::clone(&env), Rc::clone(&saved));
    let (harness, poster, _recorder) = eframe_harness(move |ctx, inbox| {
        let mut host = SettingsHost::new(Box::new(SharedEnv(host_env)));
        host.set_window_ops(fake_ops());
        let mut app = App::new(ctx, inbox, Frame::new(alpha, None, |_, _, _, _| {}));
        app.set_settings(
            host,
            Arc::new(RwLock::new(cfg)),
            Box::new(move |cfg| {
                sink.borrow_mut().push(cfg);
                region.map(str::to_owned)
            }),
        );
        app
    });
    (harness, Wired { env, saved, poster })
}

fn settings_app(cfg: Config) -> (Harness<'static, App>, Wired) {
    settings_app_with(cfg, 0.8, Some("ctrl+alt+r"))
}

/// 停在 ⛶ 上等提示框出來，看它是否帶著 `hotkey`。
fn region_tooltip_shows(h: &mut Harness<'_, App>, hotkey: &str) -> bool {
    h.get_by_label("⛶").hover();
    h.run_steps(40);
    has_label(h, &t("tooltip.region", &[("hotkey", hotkey)]))
}

fn open_settings(h: &mut Harness<'_, App>) {
    h.step();
    h.get_by_label("⚙").click();
    run(h);
}

/// 設定視窗開著時再按 ⚙。
///
/// 測試裡設定視窗嵌在同一個 viewport、上緣的縮放帶壓在標題列中段，按在按鈕上緣才不會落到它身上。
fn click_gear_while_open(h: &mut Harness<'_, App>) {
    let gear = h.get_by_label("⚙").rect();
    let at = egui::pos2(gear.center().x, gear.min.y + 12.0);
    h.hover_at(at);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
}

fn root_commands(h: &Harness<'_, App>) -> Vec<ViewportCommand> {
    h.output().viewport_output[&ViewportId::ROOT]
        .commands
        .clone()
}

/// 一幀一幀跑，收集每一幀根 viewport 的指令（kittest 把子視窗嵌在根裡，指令也落在根）。
fn frames(h: &mut Harness<'_, App>, n: usize) -> Vec<Vec<ViewportCommand>> {
    (0..n)
        .map(|_| {
            h.step();
            root_commands(h)
        })
        .collect()
}

fn close_request(h: &mut Harness<'_, App>) {
    h.input_mut()
        .viewports
        .entry(ViewportId::ROOT)
        .or_default()
        .events
        .push(ViewportEvent::Close);
}

#[test]
fn initial_size_fits_the_work_area() {
    let settings = (theme::SETTINGS_SIZE.x, theme::SETTINGS_SIZE.y);
    let wizard = (theme::WIZARD_SIZE.x, theme::WIZARD_SIZE.y);
    // 1366×768、150% 縮放的工作區
    assert_eq!(initial_size(settings, (910.0, 485.0)), (840.0, 437.0));
    assert_eq!(initial_size(wizard, (910.0, 485.0)), (600.0, 437.0));
    assert_eq!(initial_size(settings, (1920.0, 1040.0)), (840.0, 660.0));
    assert_eq!(initial_size(wizard, (1920.0, 1040.0)), (600.0, 760.0));
}

#[test]
fn gear_opens_settings_and_save_applies_to_the_overlay() {
    let _lock = language_lock();
    i18n::set_language("en-US");
    let (mut h, wired) = settings_app(settings_cfg());
    open_settings(&mut h);
    assert!(has_label(&h, &tr("button.save")), "settings window shown");
    click(&mut h, Role::Button, &tr("button.save"));
    let saved = wired.saved.borrow();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0]["overlay_alpha"], json!(0.5));
    assert!(
        !saved[0].contains_key("overlay"),
        "only the window's own keys"
    );
    drop(saved);
    let app = h.state();
    assert_eq!(app.alpha(), 0.5);
    assert_eq!(app.limits(), (50, 30));
    assert!(!has_label(&h, &tr("button.save")), "window closed");
    assert!(
        region_tooltip_shows(&mut h, "ctrl+alt+r"),
        "registered hotkey"
    );
}

#[test]
fn save_without_a_registered_region_hotkey_shows_the_saved_one() {
    let _lock = language_lock();
    i18n::set_language("en-US");
    let mut cfg = settings_cfg();
    cfg.insert("region_hotkey".into(), json!("ctrl+alt+q"));
    let (mut h, _wired) = settings_app_with(cfg, 0.8, None);
    open_settings(&mut h);
    click(&mut h, Role::Button, &tr("button.save"));
    assert!(region_tooltip_shows(&mut h, "ctrl+alt+q"));
}

#[test]
fn cancel_restores_alpha_and_language() {
    let _lock = language_lock();
    i18n::set_language("zh-TW");
    // overlay 起始透明度與設定相同：取消後要回到這個值
    let (mut h, _wired) = settings_app_with(settings_cfg(), 0.5, Some("ctrl+alt+r"));
    assert_eq!(h.state().alpha(), 0.5);
    open_settings(&mut h);
    click(&mut h, Role::Tab, &tr("settings.tab.advanced"));
    h.get_by_role(Role::Slider).set_value_accesskit(0.3);
    run(&mut h);
    assert_eq!(h.state().alpha(), 0.3);
    click(&mut h, Role::Tab, &tr("settings.tab.basic"));
    click(&mut h, Role::ComboBox, &tr("field.ui_language"));
    h.get_by_role_and_label(Role::Button, &language_name("en-US"))
        .click();
    h.step();
    let commands = root_commands(&h);
    assert!(
        commands.contains(&ViewportCommand::Title(app_name())),
        "overlay relabeled: {commands:?}"
    );
    run(&mut h);
    assert_eq!(current_language(), "en-US");
    // kittest 把視窗嵌成 egui 視窗，標題顯示在它的標題列
    let settings_title = t("settings.title", &[("app", &app_name())]);
    assert!(has_label(&h, &settings_title), "retitled");
    // ✕ 與 Alt+F4 走同一條路
    close_request(&mut h);
    run(&mut h);
    assert_eq!(current_language(), "zh-TW");
    assert_eq!(h.state().alpha(), 0.5);
    assert!(!has_label(&h, &tr("button.cancel")), "window closed");
}

#[test]
fn settings_events_reach_the_open_window() {
    let _lock = language_lock();
    i18n::set_language("en-US");
    let (mut h, wired) = settings_app(settings_cfg());
    open_settings(&mut h);
    click(&mut h, Role::Tab, &tr("settings.tab.about"));
    click(&mut h, Role::Button, &tr("button.check_update"));
    let round = *wired.env.checks.borrow().last().expect("check started");
    let release = Release {
        version: "9.9.9".into(),
        url: "https://example.invalid/rel".into(),
    };
    wired
        .poster
        .post(UiEvent::Settings(SettingsEvent::UpdateChecked {
            round,
            result: Ok(Some(release)),
        }));
    run(&mut h);
    let banner = h.state_mut().banners_mut().update_text();
    assert!(banner.is_some_and(|b| b.contains("9.9.9")));
}

#[test]
fn settings_window_does_not_stay_on_top() {
    let _lock = language_lock();
    i18n::set_language("en-US");
    let (mut h, _wired) = settings_app(settings_cfg());
    h.step();
    h.get_by_label("⚙").click();
    let normal = ViewportCommand::WindowLevel(WindowLevel::Normal);
    let seen = frames(&mut h, 4);
    let at: Vec<usize> = (0..seen.len())
        .filter(|&i| seen[i].contains(&normal))
        .collect();
    // kittest 把子視窗的指令也記在根上：第一幀的 Normal 是 overlay 自己降級（不帶顯示），
    // 設定視窗的 Normal 只在下一幀、與 Visible(true) 一起出現（第一幀建立時置頂、藏著）
    assert_eq!(at, [0, 1], "{seen:?}");
    let count = |frame: &[ViewportCommand]| frame.iter().filter(|c| **c == normal).count();
    assert_eq!(count(&seen[0]), 1, "only the overlay's: {seen:?}");
    assert!(
        !seen[0].contains(&ViewportCommand::Visible(true)),
        "{seen:?}"
    );
    assert_eq!(count(&seen[1]), 1, "only the settings window's: {seen:?}");
    assert_eq!(h.state().overlay_level(), WindowLevel::Normal);
    assert!(
        seen[1].contains(&ViewportCommand::Visible(true)),
        "{seen:?}"
    );
}

#[test]
fn releasing_topmost_survives_a_window_closed_in_the_meantime() {
    let _lock = language_lock();
    i18n::set_language("en-US");
    let (mut h, _wired) = settings_app(settings_cfg());
    h.step();
    h.get_by_label("⚙").click();
    h.step();
    // 建立後、放掉置頂之前就被關掉：不再顯示、不再改它，也不搶前景
    close_request(&mut h);
    let seen = frames(&mut h, 3);
    let normal = ViewportCommand::WindowLevel(WindowLevel::Normal);
    assert!(
        seen.iter()
            .all(|c| !c.contains(&normal) && !c.contains(&ViewportCommand::Visible(true))),
        "{seen:?}"
    );
    assert!(raised().is_empty());
    assert!(!has_label(&h, &tr("button.save")));
}

#[test]
fn the_settings_window_takes_the_foreground_when_shown() {
    let _lock = language_lock();
    i18n::set_language("en-US");
    let (mut h, _wired) = settings_app(settings_cfg());
    h.step();
    h.get_by_label("⚙").click();
    h.step();
    assert!(raised().is_empty(), "not while hidden");
    assert_eq!(DARKENED.with(|d| d.borrow().clone()), [FAKE_HWND]);
    h.step();
    assert!(root_commands(&h).contains(&ViewportCommand::Visible(true)));
    assert_eq!(raised(), [FAKE_HWND], "shown and focused in the same frame");
    run(&mut h);
    assert_eq!(raised(), [FAKE_HWND]);
    // 已經開著時再按 ⚙：只把它叫到前景
    click_gear_while_open(&mut h);
    run(&mut h);
    assert_eq!(raised(), [FAKE_HWND, FAKE_HWND]);
}

#[test]
fn reopening_restores_a_minimized_settings_window() {
    let _lock = language_lock();
    i18n::set_language("en-US");
    let (mut h, _wired) = settings_app(settings_cfg());
    open_settings(&mut h);
    let restore = ViewportCommand::Minimized(false);
    assert!(frames(&mut h, 3).iter().all(|c| !c.contains(&restore)));
    // 最小化到工作列後再按 ⚙：只搶前景叫不回來，要先還原
    click_gear_while_open(&mut h);
    let seen = frames(&mut h, 3);
    assert_eq!(
        seen.iter().filter(|c| c.contains(&restore)).count(),
        1,
        "{seen:?}"
    );
}

/// 首次設定模式的 App：`started` 記下 `start` 收到的設定。
fn wizard_app(cfg: Config) -> (Harness<'static, App>, Saved, Recorder) {
    let started = Saved::default();
    let sink = Rc::clone(&started);
    let (harness, _poster, recorder) = eframe_harness(move |ctx, inbox| {
        let mut wizard = WizardHost::new(Box::new(FakeEnv::default()));
        wizard.set_window_ops(fake_ops());
        wizard.open(&cfg);
        App::new(ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {})).with_first_run(
            wizard,
            Box::new(move |app, cfg| {
                app.set_limits(10, 0);
                sink.borrow_mut().push(cfg);
            }),
        )
    });
    (harness, started, recorder)
}

#[test]
fn wizard_mode_keeps_the_root_hidden_until_finish() {
    let _lock = language_lock();
    i18n::set_language("en-US");
    let (mut h, started, recorder) = wizard_app(wizard_cfg());
    let first = recorder.frames().remove(0).0;
    assert!(
        first.contains(&ViewportCommand::Visible(false)),
        "{first:?}"
    );
    run(&mut h);
    click(&mut h, Role::Button, &tr("button.next"));
    click(&mut h, Role::Link, &tr("wizard.skip_test"));
    click(&mut h, Role::Button, &tr("button.next"));
    assert!(started.borrow().is_empty());
    h.get_by_role_and_label(Role::Button, &tr("button.finish"))
        .click();
    let seen = frames(&mut h, 3);
    assert_eq!(started.borrow().len(), 1);
    assert_eq!(started.borrow()[0]["ui_language"], json!("en-US"));
    assert_eq!(h.state().limits(), (10, 0));
    assert!(
        seen.iter()
            .any(|c| c.contains(&ViewportCommand::Visible(true))),
        "{seen:?}"
    );
    assert!(!has_label(&h, &tr("button.finish")), "wizard closed");
}

#[test]
fn wizard_finish_syncs_the_overlay_language() {
    let _lock = language_lock();
    // App 建立時記下的語言與精靈結束時的不同，才分得出有沒有同步
    i18n::set_language("zh-CN");
    let (mut h, started, _recorder) = wizard_app(wizard_cfg());
    i18n::set_language("en-US");
    run(&mut h);
    assert_eq!(h.state().ui_language(), "zh-CN");
    click(&mut h, Role::Button, &tr("button.next"));
    click(&mut h, Role::Link, &tr("wizard.skip_test"));
    click(&mut h, Role::Button, &tr("button.next"));
    h.get_by_role_and_label(Role::Button, &tr("button.finish"))
        .click();
    frames(&mut h, 3);
    assert_eq!(started.borrow()[0]["ui_language"], json!("en-US"));
    assert_eq!(h.state().ui_language(), "en-US");
}

#[test]
fn a_wizard_host_that_was_never_opened_is_logged() {
    let _lock = language_lock();
    i18n::set_language("en-US");
    capture_logs();
    let not_opened = || log_lines_with("[app] first-run wizard host was not opened").len();
    let before = not_opened();
    let (_h, _poster, _recorder) = eframe_harness(|ctx, inbox| {
        let wizard = WizardHost::new(Box::new(FakeEnv::default()));
        App::new(ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
            .with_first_run(wizard, Box::new(|_, _| {}))
    });
    assert_eq!(not_opened(), before + 1);
}

#[test]
fn wizard_cancel_closes_the_root() {
    let _lock = language_lock();
    i18n::set_language("en-US");
    capture_logs();
    let logged = || log_lines_with("[app] wizard cancelled, exiting").len();
    let before = logged();
    let (mut h, started, _recorder) = wizard_app(wizard_cfg());
    run(&mut h);
    close_request(&mut h);
    h.step();
    assert!(root_commands(&h).contains(&ViewportCommand::Close));
    assert!(started.borrow().is_empty());
    assert_eq!(logged(), before + 1);
    // 關閉要求排隊期間還會再跑幾幀：根視窗不可在結束前閃出來
    let seen = frames(&mut h, 5);
    assert!(
        seen.iter()
            .all(|c| !c.contains(&ViewportCommand::Visible(true))),
        "{seen:?}"
    );
}

#[test]
fn the_wizard_takes_the_foreground_when_shown() {
    let _lock = language_lock();
    i18n::set_language("en-US");
    let (mut h, _started, recorder) = wizard_app(wizard_cfg());
    run(&mut h);
    let seen = recorder.frames();
    assert_eq!(seen[0].1, 0, "not while hidden");
    assert!(
        seen[1].0.contains(&ViewportCommand::Visible(true)),
        "{seen:?}"
    );
    assert_eq!(seen[1].1, 1, "shown and focused in the same frame");
    assert_eq!(raised(), [FAKE_HWND]);
    assert_eq!(DARKENED.with(|d| d.borrow().clone()), [FAKE_HWND]);
    let normal = ViewportCommand::WindowLevel(WindowLevel::Normal);
    assert!(
        seen.iter().all(|(c, _)| !c.contains(&normal)),
        "the wizard is never topmost"
    );
}

#[test]
fn the_wizard_language_preview_retitles_its_window() {
    let _lock = language_lock();
    i18n::set_language("en-US");
    let (mut h, _started, _recorder) = wizard_app(wizard_cfg());
    run(&mut h);
    click(&mut h, Role::ComboBox, &tr("field.ui_language"));
    h.get_by_role_and_label(Role::Button, &language_name("zh-TW"))
        .click();
    run(&mut h);
    let title = t("wizard.title", &[("app", &app_name())]);
    assert_eq!(current_language(), "zh-TW");
    let shown = has_label(&h, &title);
    i18n::set_language("en-US");
    assert!(shown, "retitled to {title:?}");
}

#[test]
fn the_overlay_drops_below_the_settings_window_while_it_is_open() {
    let _lock = language_lock();
    i18n::set_language("en-US");
    let (mut h, _wired) = settings_app(settings_cfg());
    h.step();
    assert_eq!(h.state().overlay_level(), WindowLevel::AlwaysOnTop);
    for close in [tr("button.cancel"), tr("button.save")] {
        h.get_by_label("⚙").click();
        h.step();
        let opened = root_commands(&h);
        assert_eq!(h.state().overlay_level(), WindowLevel::Normal);
        assert!(
            opened.contains(&ViewportCommand::WindowLevel(WindowLevel::Normal)),
            "{opened:?}"
        );
        // 開著的期間不會被別的路徑改回置頂
        let open: Vec<_> = frames(&mut h, 3).concat();
        assert!(
            !open.contains(&ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop)),
            "{open:?}"
        );
        h.get_by_role_and_label(Role::Button, &close).click();
        h.step();
        assert_eq!(
            h.state().overlay_level(),
            WindowLevel::AlwaysOnTop,
            "{close}"
        );
        assert!(
            root_commands(&h).contains(&ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop)),
            "{close}"
        );
        run(&mut h);
    }
}
