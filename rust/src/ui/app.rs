//! egui 應用程式外殼：overlay 是根 viewport、泡泡、翻譯輸入框與框選的視窗是 immediate viewport，每幀先取完 UI 事件再繪製。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Instant;

use eframe::NativeOptions;
use eframe::egui::{self, Color32, UiBuilder, ViewportBuilder, ViewportCommand, ViewportId};
use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, wgpu};

use serde_json::Value;

use crate::config::{Config, Settings, app_name};
use crate::i18n::current_language;
use crate::input::dispatch::GameInputEvents;
use crate::logs::log;
use crate::ui::events::{UiEvent, UiPoster};
use crate::ui::geometry::Rect;
use crate::ui::icon::app_icon;
use crate::ui::input_box::InputBox;
use crate::ui::input_host::{Desktop, InputHost};
use crate::ui::overlay::banners::Banners;
use crate::ui::overlay::bubble::{Action, BUBBLE_SIZE, Bubble, bubble_alpha, initial_position};
use crate::ui::overlay::frame::{BAR_HEIGHT, Clicked, Frame, Win32Shell};
use crate::ui::overlay::message_list::MessageList;
use crate::ui::overlay::model::OverlayModel;
use crate::ui::overlay::selection::Selection;
use crate::ui::overlay::tab_bar;
use crate::ui::region::flow::RegionFlow;
use crate::ui::region_host::RegionHost;
use crate::ui::settings::window::SettingsAction;
use crate::ui::settings::wizard::WizardAction;
use crate::ui::settings_host::{SettingsHost, WizardHost};
use crate::ui::{fonts_egui, win32};
use crate::util::read;

/// 設定接上之前的訊息上限（對應 config 預設值）。
const DEFAULT_MAX_MESSAGES: usize = 200;

/// UI 執行緒這一端：事件接收端與重繪用的 Context（`App::new` 時才綁上）。
pub struct Inbox {
    rx: Receiver<UiEvent>,
    ctx: Arc<OnceLock<egui::Context>>,
}

/// 建立 UI 事件通道；UI 起來前投遞的事件留在 channel，等第一幀一起處理。
pub fn channel() -> (UiPoster, Inbox) {
    let ctx = Arc::new(OnceLock::<egui::Context>::new());
    let wake = Arc::clone(&ctx);
    let (poster, rx) = UiPoster::new(Arc::new(move || {
        if let Some(ctx) = wake.get() {
            ctx.request_repaint();
        }
    }));
    (poster, Inbox { rx, ctx })
}

/// 設定視窗按儲存：收到視窗管的那些鍵（不是完整設定），回傳實際生效的輸入框與框選熱鍵。
pub type OnSave = Box<dyn FnMut(Config) -> (String, Option<String>)>;
/// 精靈完成後接上其餘功能並開始執行；收到完整設定。
pub type OnStart = Box<dyn FnOnce(&mut App, Config)>;

/// 設定視窗、⚙ 開窗時取快照的共用設定與儲存回呼。
struct SettingsWiring {
    host: SettingsHost,
    cfg: Arc<RwLock<Config>>,
    on_save: OnSave,
}

/// 首次設定精靈與完成後的接線。
struct FirstRun {
    host: WizardHost,
    start: OnStart,
}

/// overlay、泡泡、翻譯輸入框、框選與設定視窗的狀態；所有方法都在 UI 執行緒呼叫。
pub struct App {
    rx: Receiver<UiEvent>,
    hwnd: isize,
    frame: Frame,
    minimized: bool,
    shown: bool,
    status: Option<&'static str>,
    banners: Banners,
    region_hotkey: String,
    model: OverlayModel,
    list: MessageList,
    selection: Selection,
    bubble: Option<Bubble>,
    bubble_position: (Option<i32>, Option<i32>),
    on_bubble_move: Box<dyn FnMut(i32, i32)>,
    on_close: Option<Box<dyn FnOnce()>>,
    input: Option<InputHost>,
    region: Option<RegionHost>,
    /// 上次套用的介面語言；儲存設定時不同才 relabel。
    ui_language: String,
    settings: Option<SettingsWiring>,
    first_run: Option<FirstRun>,
}

impl App {
    /// 綁上 Context 讓投遞端能喚醒重繪；`frame` 是 overlay 的外框（含底板不透明度與起始位置）。
    pub fn new(ctx: &egui::Context, inbox: Inbox, frame: Frame) -> Self {
        let _ = inbox.ctx.set(ctx.clone());
        ctx.all_styles_mut(|style| style.interaction.tooltip_delay = 0.5);
        Self {
            rx: inbox.rx,
            hwnd: 0,
            frame,
            minimized: false,
            shown: true,
            status: None,
            banners: Banners::default(),
            region_hotkey: String::new(),
            model: OverlayModel::new(DEFAULT_MAX_MESSAGES, 0),
            list: MessageList::new(),
            selection: Selection::default(),
            bubble: None,
            bubble_position: (None, None),
            on_bubble_move: Box::new(|_, _| {}),
            on_close: None,
            input: None,
            region: None,
            ui_language: current_language(),
            settings: None,
            first_run: None,
        }
    }

    /// 接上翻譯輸入框與遊戲聊天框事件；`settings` 用來讀 `auto_show_input`。
    pub fn with_input(
        mut self,
        input_box: InputBox,
        events: GameInputEvents,
        settings: Arc<RwLock<Config>>,
    ) -> Self {
        self.set_input(input_box, events, settings);
        self
    }

    /// 同 [`Self::with_input`]，給建好之後才接線（精靈完成後）。
    pub fn set_input(
        &mut self,
        input_box: InputBox,
        events: GameInputEvents,
        settings: Arc<RwLock<Config>>,
    ) {
        self.input = Some(InputHost::new(input_box, events, settings));
    }

    /// 換掉輸入框視窗的桌面操作（測試與 probe 用：不搶前景、擺在螢幕外）；沒接輸入框時不動。
    pub fn set_input_desktop(&mut self, desktop: Desktop) {
        if let Some(host) = &mut self.input {
            host.desktop = desktop;
        }
    }

    /// 接上框選翻譯（熱鍵、⛶ 與辨識結果都交給它）；卡片背景不透明度跟著 overlay。
    pub fn with_region(mut self, flow: RegionFlow) -> Self {
        self.set_region(flow);
        self
    }

    /// 同 [`Self::with_region`]，給建好之後才接線。
    pub fn set_region(&mut self, flow: RegionFlow) {
        self.region = Some(RegionHost::new(flow));
    }

    /// 換掉框選視窗的桌面操作（測試與 probe 用：不搶前景、假的工作區）；沒接框選時不動。
    pub fn set_region_desktop(&mut self, desktop: Desktop) {
        if let Some(host) = &mut self.region {
            host.desktop = desktop;
        }
    }

    /// 框選流程（probe 用：不經滑鼠直接完成框選）；沒接框選時是 `None`。
    pub fn region_flow_mut(&mut self) -> Option<&mut RegionFlow> {
        self.region.as_mut().map(|host| &mut host.flow)
    }

    /// 泡泡位置：`saved`（設定裡的實體像素，未設為 None）縮小時沿用，拖曳放開時呼叫 `on_move`。
    pub fn with_bubble_position(
        mut self,
        saved: (Option<i32>, Option<i32>),
        on_move: impl FnMut(i32, i32) + 'static,
    ) -> Self {
        self.set_bubble_position(saved, on_move);
        self
    }

    /// 同 [`Self::with_bubble_position`]，給建好之後才接線。
    pub fn set_bubble_position(
        &mut self,
        saved: (Option<i32>, Option<i32>),
        on_move: impl FnMut(i32, i32) + 'static,
    ) {
        self.bubble_position = saved;
        self.on_bubble_move = Box::new(on_move);
    }

    /// 根視窗收到關閉要求（✕、Alt+F4、工作列）時、在 eframe 銷毀視窗之前呼叫 `on_close` 一次。
    ///
    /// 關閉流程在這裡跑，視窗就留到 reader 卸完 hook：期間重開的第二份還找得到它並喚起。
    pub fn with_on_close(mut self, on_close: impl FnOnce() + 'static) -> Self {
        self.set_on_close(on_close);
        self
    }

    /// 同 [`Self::with_on_close`]，給建好之後才接線。
    pub fn set_on_close(&mut self, on_close: impl FnOnce() + 'static) {
        self.on_close = Some(Box::new(on_close));
    }

    /// 接上設定視窗：⚙ 以 `settings` 當下的內容開窗，按儲存時呼叫 `on_save` 再套用 overlay 端的設定。
    pub fn with_settings(
        mut self,
        host: SettingsHost,
        settings: Arc<RwLock<Config>>,
        on_save: OnSave,
    ) -> Self {
        self.set_settings(host, settings, on_save);
        self
    }

    /// 同 [`Self::with_settings`]，給建好之後才接線。
    pub fn set_settings(
        &mut self,
        host: SettingsHost,
        settings: Arc<RwLock<Config>>,
        on_save: OnSave,
    ) {
        self.settings = Some(SettingsWiring {
            host,
            cfg: settings,
            on_save,
        });
    }

    /// 首次設定模式：精靈（呼叫端先 `open`）開著時根視窗藏著；完成時關掉精靈、呼叫 `start`，再顯示根視窗。
    pub fn with_first_run(mut self, wizard: WizardHost, start: OnStart) -> Self {
        if !wizard.is_open() {
            log("[app] first-run wizard host was not opened; the overlay stays hidden");
        }
        self.first_run = Some(FirstRun {
            host: wizard,
            start,
        });
        self
    }

    /// ⚙：以目前的共用設定開設定視窗；已開著就叫到前景。
    pub fn open_settings(&mut self) {
        match &mut self.settings {
            Some(wiring) => {
                let cfg = read(&wiring.cfg).clone();
                wiring.host.open(&cfg);
            }
            None => log("[ui] settings button clicked but no settings window is wired"),
        }
    }

    /// overlay 端套用儲存後的設定（`cfg` 可以只含設定視窗管的鍵），順序同 Python `apply_settings` 的後半；
    /// `region_hotkey` 是實際註冊到的框選熱鍵，`None`（沒註冊）時顯示設定裡的值。
    pub fn apply_settings(
        &mut self,
        ctx: &egui::Context,
        cfg: &Config,
        region_hotkey: Option<String>,
    ) {
        let values = Settings::from_config(cfg);
        let region_hotkey = region_hotkey.unwrap_or_else(|| {
            let saved = cfg.get("region_hotkey").and_then(Value::as_str);
            saved.unwrap_or_default().to_owned()
        });
        self.set_region_hotkey(&region_hotkey);
        if let Some(region) = &mut self.region {
            region.set_alpha(values.overlay_alpha);
        }
        self.set_limits(values.max_messages, values.fade_seconds.into());
        self.frame.set_alpha(values.overlay_alpha);
        log(format!(
            "[ui] overlay settings applied: alpha={} max_messages={} fade_seconds={} region_hotkey={region_hotkey}",
            values.overlay_alpha, values.max_messages, values.fade_seconds
        ));
        // 預覽時通常已 relabel 過，這裡是沒經過預覽路徑的保底
        if let Some(language) = cfg.get("ui_language").and_then(Value::as_str)
            && language != self.ui_language
        {
            self.ui_language = language.to_owned();
            self.relabel(ctx);
        }
    }

    /// overlay 底板目前的不透明度（含預覽中的值）。
    pub fn alpha(&self) -> f32 {
        self.frame.alpha()
    }

    /// 目前的訊息上限與淡出秒數。
    pub fn limits(&self) -> (usize, u64) {
        self.model.limits()
    }

    /// 套用訊息上限與淡出秒數（0＝不淡出）。
    pub fn set_limits(&mut self, max_messages: usize, fade_seconds: u64) {
        self.model.set_limits(max_messages, fade_seconds);
    }

    /// 目前狀態列的 state key。
    pub fn status(&self) -> Option<&'static str> {
        self.status
    }

    /// 錯誤與更新橫幅（更新檢查經 `offer_update` 提示新版）。
    pub fn banners_mut(&mut self) -> &mut Banners {
        &mut self.banners
    }

    /// 設定裡的框選熱鍵，顯示在 ⛶ 的提示框；下一次顯示提示就生效。
    pub fn set_region_hotkey(&mut self, hotkey: &str) {
        self.region_hotkey = hotkey.to_owned();
    }

    /// 介面語言變更後重新翻譯常駐文字：狀態、橫幅、選單與提示存的是 key、每幀才翻譯，
    /// 這裡只需換字型與視窗標題。已經顯示的訊息是聊天內容，不回溯改寫。
    pub fn relabel(&mut self, ctx: &egui::Context) {
        let language = current_language();
        fonts_egui::install(ctx, &language);
        ctx.send_viewport_cmd(ViewportCommand::Title(app_name()));
        log(format!("[ui] overlay relabeled language={language}"));
    }

    /// overlay 的 HWND；第一幀之前是 0。
    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    /// 縮成泡泡（隱藏 overlay，訊息照常累積）或展開回 overlay；已在該狀態時不動。
    pub fn set_minimized(&mut self, minimized: bool) {
        if self.minimized == minimized {
            return;
        }
        if minimized {
            self.selection.clear("minimized to bubble");
        } else {
            self.bubble = None;
        }
        self.minimized = minimized;
        self.model.set_minimized(minimized);
        let what = if minimized {
            "minimized to bubble"
        } else {
            "expanded"
        };
        let messages = self.model.current_view().map_or(0, |v| v.messages.len());
        let follow = if self.list.follow() { "True" } else { "False" };
        log(format!("[ui] {what} (messages={messages} follow={follow})"));
    }

    /// 開出泡泡：起點記成這次執行期間的泡泡位置（不寫回設定），之後縮小都沿用，overlay 移動也不跟著跑。
    fn open_bubble(&mut self, overlay: Rect, size: i32, monitors: &[Rect]) {
        let (x, y) = initial_position(self.bubble_position, overlay, size, monitors);
        self.bubble_position = (Some(x), Some(y));
        self.bubble = Some(Bubble::new((x, y), Instant::now()));
    }

    fn apply(&mut self, ctx: &egui::Context, event: UiEvent) {
        match event {
            UiEvent::SetStatus(key) => self.status = Some(key),
            UiEvent::SetError { key, args } => self.banners.set_error(key, args),
            UiEvent::ClearError => self.banners.clear_error(),
            UiEvent::UpdateFound(release) => self.banners.offer_update(release),
            UiEvent::SetForegroundSlot(slot) => self.model.set_foreground_slot(slot),
            UiEvent::AddMessage {
                slot,
                tabs,
                original,
                translated,
                msg_id,
                pending,
                color,
            } => self.model.add_message(
                slot,
                &tabs,
                &original,
                &translated,
                msg_id,
                pending,
                color.as_deref(),
                Instant::now(),
            ),
            UiEvent::UpdateMessage {
                msg_id,
                translated,
                failed,
            } => self.model.update_message(msg_id, &translated, failed),
            UiEvent::SetTabs { slot, state } => self.model.set_tabs(slot, state),
            event @ (UiEvent::ShowInputBox { .. }
            | UiEvent::GameInputOpened { .. }
            | UiEvent::GameInputClosed { .. }
            | UiEvent::OutgoingDone { .. }) => {
                if let Some(host) = &mut self.input {
                    host.apply(event);
                }
            }
            UiEvent::RegionHotkey { hwnd } => {
                if let Some(host) = &mut self.region {
                    host.toggle(hwnd);
                }
            }
            UiEvent::RegionDone { session, result } => {
                if let Some(host) = &mut self.region {
                    host.finish(session, result);
                }
            }
            UiEvent::Settings(event) => match (&mut self.first_run, &mut self.settings) {
                (Some(first_run), _) if first_run.host.is_open() => first_run.host.apply(event),
                (_, Some(wiring)) => wiring.host.apply(event),
                _ => {}
            },
            // 與 ✕ 同一條路：關掉根 viewport
            UiEvent::Quit => ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close),
        }
    }

    /// 所有已知 viewport 的 HWND（overlay、泡泡、輸入框、選取畫面、框、卡片、設定視窗、精靈；0 不列）：找新視窗時一律略過。
    fn viewport_hwnds(&self) -> Vec<isize> {
        let bubble = self.bubble.as_ref().map_or(0, Bubble::hwnd);
        let input = self.input.as_ref().map_or(0, InputHost::hwnd);
        let region = self.region.as_ref().map_or([0; 3], RegionHost::hwnds);
        let settings = self.settings.as_ref().map_or(0, |s| s.host.hwnd());
        let wizard = self.first_run.as_ref().map_or(0, |w| w.host.hwnd());
        [self.hwnd, bubble, input]
            .into_iter()
            .chain(region)
            .chain([settings, wizard])
            .filter(|&h| h != 0)
            .collect()
    }

    /// 標題列以下：分頁列（沒有分頁時隱藏）、底部橫幅與其間的訊息清單（含框選與右鍵選單）。
    fn content(&mut self, ui: &mut egui::Ui) {
        let mut rect = ui.max_rect();
        rect.min.y += BAR_HEIGHT;
        let mut content = ui.new_child(UiBuilder::new().max_rect(rect));
        let (tabs, current) = self.model.current_tabs();
        tab_bar::show(&mut content, tabs, current);
        rect.min.y = content.min_rect().max.y.max(rect.min.y);
        let rect = self.banners.show(ui, rect);
        self.selection
            .show(ui, &mut self.list, &self.model, &mut Win32Shell(self.hwnd));
        self.list
            .show(ui, rect, &self.model, self.status, self.selection.span());
    }

    /// 縮小期間每幀顯示泡泡、處理它的點擊／拖曳／關閉，並每 300ms 檢查一次是否該自動展開。
    fn show_bubble(&mut self, ctx: &egui::Context) {
        let ppp = ctx.pixels_per_point();
        if self.bubble.is_none() {
            let size = (BUBBLE_SIZE as f32 * ppp).round() as i32;
            self.open_bubble(win32::window_rect(self.hwnd), size, &win32::all_monitors());
        }
        let known = self.viewport_hwnds();
        let Some(bubble) = self.bubble.as_mut() else {
            return;
        };
        let size = BUBBLE_SIZE as f32;
        let mut builder = ViewportBuilder::default()
            .with_title(app_name())
            .with_inner_size([size, size])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top();
        // 只是建立時的概略位置（混合 DPI 下會偏），找到 HWND 後再以實體像素擺正
        if let Some((x, y)) = bubble.pending() {
            builder = builder.with_position([x as f32 / ppp, y as f32 / ppp]);
        }
        let (hwnd, unread) = (bubble.hwnd(), self.model.unread());
        let alpha = bubble_alpha(self.frame.alpha());
        let before = if hwnd == 0 {
            win32::thread_windows()
        } else {
            Vec::new()
        };
        let action =
            ctx.show_viewport_immediate(ViewportId::from_hash_of("bubble"), builder, |ui, _| {
                bubble.show(ui, &mut Win32Shell(hwnd), unread, alpha)
            });
        if hwnd == 0 {
            let found = win32::pick_new_window(&before, &win32::thread_windows(), &known, || {
                win32::find_thread_window(&app_name(), &known)
            });
            if found != 0 {
                win32::remove_system_frame(found);
            }
            bubble.attach(found, win32::foreground_window());
        }
        let now = Instant::now();
        let expand = bubble.watch(now, win32::foreground_window(), &Win32Shell(bubble.hwnd()));
        ctx.request_repaint_after(bubble.next_poll().saturating_duration_since(now));
        match action {
            Some(action) => self.bubble_action(ctx, action),
            None if expand => self.set_minimized(false),
            None => {}
        }
    }

    /// 首次設定模式（精靈開著，或已取消、等根視窗關閉）時根視窗藏著；完成時 `first_run` 才清掉。
    fn in_first_run(&self) -> bool {
        self.first_run.is_some()
    }

    /// 每幀畫出開著的設定視窗與精靈，並處理它們要 App 做的事。
    fn show_settings(&mut self, ctx: &egui::Context) {
        let known = self.viewport_hwnds();
        let actions = match &mut self.settings {
            Some(wiring) => wiring.host.show(ctx, &known),
            None => Vec::new(),
        };
        for action in actions {
            self.settings_action(ctx, action);
        }
        let known = self.viewport_hwnds();
        let actions = match &mut self.first_run {
            Some(first_run) => first_run.host.show(ctx, &known),
            None => Vec::new(),
        };
        for action in actions {
            self.wizard_action(ctx, action);
        }
    }

    fn settings_action(&mut self, ctx: &egui::Context, action: SettingsAction) {
        match action {
            SettingsAction::PreviewAlpha(alpha) => self.frame.set_alpha(alpha),
            SettingsAction::PreviewLanguage => self.relabel(ctx),
            SettingsAction::UpdateFound(release) => self.banners.set_update(release),
            SettingsAction::Save(cfg) => {
                let Some(wiring) = &mut self.settings else {
                    return;
                };
                let (_, region_hotkey) = (wiring.on_save)(cfg.clone());
                self.apply_settings(ctx, &cfg, region_hotkey);
                self.close_settings();
            }
            SettingsAction::Cancel => self.close_settings(),
        }
    }

    fn close_settings(&mut self) {
        if let Some(wiring) = &mut self.settings {
            wiring.host.close();
        }
    }

    fn wizard_action(&mut self, ctx: &egui::Context, action: WizardAction) {
        match action {
            WizardAction::PreviewLanguage => self.relabel(ctx),
            WizardAction::Finish(cfg) => {
                let Some(FirstRun { mut host, start }) = self.first_run.take() else {
                    return;
                };
                host.close();
                let language = cfg.get("ui_language").and_then(Value::as_str);
                self.ui_language = language.map_or_else(current_language, str::to_owned);
                start(self, cfg);
                ctx.request_repaint();
            }
            WizardAction::Cancel => {
                log("[app] wizard cancelled, exiting");
                // `first_run` 留著：關閉要求排隊的那幾幀根視窗也要藏著，不閃出來
                if let Some(first_run) = &mut self.first_run {
                    first_run.host.close();
                }
                // 這時還沒接上 on_close：根視窗一關程序就結束
                ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
            }
        }
    }

    fn bubble_action(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Expand => self.set_minimized(false),
            Action::Moved(x, y) => {
                log(format!("[ui] bubble moved: +{x}+{y}"));
                self.bubble_position = (Some(x), Some(y));
                (self.on_bubble_move)(x, y);
            }
            // 與 ✕ 同一條路：關掉根 viewport
            Action::Close => ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close),
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested())
            && let Some(on_close) = self.on_close.take()
        {
            on_close();
        }
        while let Ok(event) = self.rx.try_recv() {
            self.apply(ctx, event);
        }
        let now = Instant::now();
        self.model.prune(now);
        // 只在下一則該淡出時醒來，閒置時不空轉重繪
        if let Some(wait) = self.model.next_fade(now) {
            ctx.request_repaint_after(wait);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.hwnd == 0 {
            self.hwnd = win32::hwnd_of(frame);
            if self.hwnd != 0 {
                win32::remove_system_frame(self.hwnd);
            }
        }
        let visible = !self.minimized && !self.in_first_run();
        if self.shown != visible {
            self.shown = visible;
            ctx.send_viewport_cmd(ViewportCommand::Visible(self.shown));
            // winit 處理 Visible 時會重寫 extended style 洗掉 NOACTIVATE，要保證下一幀補回
            ctx.request_repaint();
        }
        if self.hwnd != 0 {
            win32::make_non_activating(self.hwnd);
        }
        let mut shell = Win32Shell(self.hwnd);
        match self.frame.show(
            ui,
            &mut shell,
            &app_name(),
            self.status,
            &self.region_hotkey,
        ) {
            Some(Clicked::Region) => {
                if let Some(host) = &mut self.region {
                    host.start_from_button();
                }
            }
            Some(Clicked::Settings) => self.open_settings(),
            Some(Clicked::Minimize) => self.set_minimized(true),
            // 與 Alt+F4、工作列關閉同一條路：關掉根 viewport
            Some(Clicked::Close) => ctx.send_viewport_cmd(ViewportCommand::Close),
            None => {}
        }
        self.content(ui);
        if self.minimized {
            self.show_bubble(&ctx);
        }
        let known = self.viewport_hwnds();
        if let Some(host) = &mut self.input {
            host.show(&ctx, self.hwnd, &known);
        }
        let known = self.viewport_hwnds();
        if let Some(host) = &mut self.region {
            host.set_alpha(self.frame.alpha());
            host.show(&ctx, self.hwnd, &known);
        }
        self.show_settings(&ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        Color32::TRANSPARENT.to_normalized_gamma_f32()
    }
}

/// overlay 根視窗的 eframe 設定：DX12 經 DirectComposition 才有每像素透明；沒有硬體 adapter 時 wgpu 自動退到 WARP。
pub fn native_options() -> NativeOptions {
    let mut wgpu_options = WgpuConfiguration::default();
    if let WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends = wgpu::Backends::DX12;
        setup
            .instance_descriptor
            .backend_options
            .dx12
            .presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
    }
    NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title(app_name())
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(true)
            .with_active(false)
            // 不給的話 eframe 會裝上 egui 標誌；泡泡 viewport 沿用根 viewport 的 icon
            .with_icon(app_icon()),
        wgpu_options,
        ..Default::default()
    }
}

/// 強制走 WARP 軟體 adapter 的 eframe 設定（硬體 adapter 建立裝置或 surface 失敗時的退路）。
pub fn warp_options() -> NativeOptions {
    let mut native = native_options();
    if let WgpuSetup::CreateNew(setup) = &mut native.wgpu_options.wgpu_setup {
        setup.native_adapter_selector = Some(Arc::new(|adapters, _surface| {
            adapters
                .iter()
                .find(|a| a.get_info().device_type == wgpu::DeviceType::Cpu)
                .cloned()
                .ok_or_else(|| "no WARP adapter".to_owned())
        }));
    }
    native
}

/// `visible` 為 false 時根視窗建立時就藏著（首次設定精靈：overlay 不會先閃一下）。
fn with_root_visibility(mut native: NativeOptions, visible: bool) -> NativeOptions {
    if !visible {
        native.viewport = native.viewport.with_visible(false);
    }
    native
}

/// 同 [`run`]，但 app 建立前就失敗（硬體 adapter 列得出來卻開不了）時改用 WARP 再試一次；`visible` 見 [`with_root_visibility`]。
pub fn run_with_warp_fallback<A: eframe::App + 'static>(
    visible: bool,
    build: impl FnOnce(&egui::Context) -> A + 'static,
) -> Result<(), eframe::Error> {
    let build = Rc::new(RefCell::new(Some(build)));
    let attempt = |native| {
        let build = Rc::clone(&build);
        run(with_root_visibility(native, visible), move |ctx| {
            let build = build.borrow_mut().take().expect("app is built once");
            build(ctx)
        })
    };
    match attempt(native_options()) {
        Err(e) if build.borrow().is_some() => {
            log(format!(
                "[ui] hardware renderer failed before the overlay opened: {e}; retrying with WARP"
            ));
            attempt(warp_options())
        }
        other => other,
    }
}

/// 開啟 UI 並跑到視窗關閉；`build` 在第一幀前以 egui Context 建立 app。
pub fn run<A: eframe::App + 'static>(
    native: NativeOptions,
    build: impl FnOnce(&egui::Context) -> A + 'static,
) -> Result<(), eframe::Error> {
    win32::disable_redirection_bitmap();
    eframe::run_native(
        &app_name(),
        native,
        Box::new(|cc| {
            if let Some(render) = &cc.wgpu_render_state {
                let info = render.adapter.get_info();
                log(format!(
                    "[ui] renderer: backend={:?} adapter={:?} type={:?}",
                    info.backend, info.name, info.device_type
                ));
            }
            fonts_egui::install(&cc.egui_ctx, &current_language());
            Ok(Box::new(build(&cc.egui_ctx)))
        }),
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::region::pipeline::RegionResult;
    use crate::ui::input_box::OutgoingRequest;
    use crate::ui::overlay::banners::Release;
    use crate::ui::overlay::bubble::POLL_INTERVAL;
    use crate::ui::region::selector::SelectorAction;
    use crate::ui::region_host::fakes;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use std::time::Duration;

    #[test]
    fn app_drains_events_each_frame() {
        let (poster, inbox) = channel();
        poster.post(UiEvent::SetStatus("listening"));
        poster.post(UiEvent::SetForegroundSlot(Some(2)));
        poster.post(UiEvent::SetError {
            key: "error.offline",
            args: vec![("detail", "x".into())],
        });
        let mut harness = Harness::new_eframe(|cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
        });
        harness.step();
        let app = harness.state();
        assert_eq!(app.status, Some("listening"));
        assert_eq!(app.model.client(), Some(2));
        assert_eq!(
            app.banners.error_text(),
            Some(crate::i18n::t("error.offline", &[]))
        );
    }

    #[test]
    fn app_routes_messages_into_the_overlay_model() {
        let (poster, inbox) = channel();
        poster.post(UiEvent::AddMessage {
            slot: 1,
            tabs: vec![0],
            original: "o".into(),
            translated: "...".into(),
            msg_id: Some(7),
            pending: true,
            color: None,
        });
        poster.post(UiEvent::UpdateMessage {
            msg_id: 7,
            translated: "t".into(),
            failed: false,
        });
        let mut harness = Harness::new_eframe(|cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
        });
        harness.step();
        let view = harness.state().model.current_view().unwrap();
        assert_eq!(view.messages[0].translated, "t");
        assert!(!view.messages[0].pending);
    }

    #[test]
    fn tab_bar_sits_between_the_title_bar_and_the_list() {
        let (poster, inbox) = channel();
        poster.post(UiEvent::SetTabs {
            slot: 1,
            state: Some(crate::reader::chat_tabs::TabState {
                tabs: vec![(0, "Main".into())],
                current: 0,
            }),
        });
        poster.post(UiEvent::AddMessage {
            slot: 1,
            tabs: vec![0],
            original: "o".into(),
            translated: "t".into(),
            msg_id: None,
            pending: false,
            color: None,
        });
        let mut harness = Harness::new_eframe(|cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
        });
        harness.step();
        let tab = harness.get_by_label("Main").rect();
        let row = harness.state().list.rows()[0].lines[0].0;
        assert!(tab.min.y >= BAR_HEIGHT, "{tab:?}");
        assert!(row.y >= tab.max.y, "{row:?} {tab:?}");
    }

    #[test]
    fn error_banner_not_squeezed_by_list() {
        let (poster, inbox) = channel();
        for _ in 0..50 {
            poster.post(UiEvent::AddMessage {
                slot: 1,
                tabs: vec![0],
                original: "original".into(),
                translated: "譯文".into(),
                msg_id: None,
                pending: false,
                color: None,
            });
        }
        poster.post(UiEvent::SetError {
            key: "notice.game_missing",
            args: Vec::new(),
        });
        let size = egui::vec2(200.0, 120.0);
        let mut harness = Harness::builder().with_size(size).build_eframe(|cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
        });
        harness.step();
        let banner = harness
            .get_by_label(&crate::i18n::t("notice.game_missing", &[]))
            .rect();
        assert!(
            banner.min.y >= BAR_HEIGHT && banner.max.y <= size.y,
            "{banner:?}"
        );
        let list = &harness.state().list;
        assert!(!list.rows().is_empty());
        assert!(
            list.viewport().max.y <= banner.min.y,
            "{:?}",
            list.viewport()
        );
    }

    #[test]
    fn tall_update_banner_never_covers_the_title_buttons() {
        let (_poster, inbox) = channel();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(200.0, 90.0))
            .build_eframe(|cc| {
                fonts_egui::install(&cc.egui_ctx, "en-US");
                App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
            });
        harness.state_mut().banners_mut().set_update(Release {
            version: format!("0.2.0-{}", "rc.".repeat(20)),
            url: "https://example.invalid/rel".into(),
        });
        harness.step();
        harness.get_by_label("─").click();
        harness.step();
        assert!(harness.state().minimized);
    }

    #[test]
    fn region_tooltip_shows_the_set_hotkey_after_half_a_second() {
        let (_poster, inbox) = channel();
        let mut harness = Harness::new_eframe(|cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            // 換掉 egui 的預設值：提示要等 0.5 秒是 App 自己設的
            cc.egui_ctx
                .all_styles_mut(|style| style.interaction.tooltip_delay = 10.0);
            App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
        });
        harness.state_mut().set_region_hotkey("ctrl+alt+r");
        harness.step();
        let text = crate::i18n::t("tooltip.region", &[("hotkey", "ctrl+alt+r")]);
        harness.get_by_label("⛶").hover();
        harness.step();
        assert!(
            harness.query_by_label(&text).is_none(),
            "shown before the delay"
        );
        for _ in 0..40 {
            harness.step();
        }
        harness.get_by_label(&text);
    }

    #[test]
    fn minimizing_clears_the_selection() {
        let (poster, inbox) = channel();
        poster.post(UiEvent::AddMessage {
            slot: 1,
            tabs: vec![0],
            original: "原文一".into(),
            translated: "譯文一".into(),
            msg_id: None,
            pending: false,
            color: None,
        });
        let mut harness = Harness::new_eframe(|cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
        });
        harness.step();
        let app = harness.state_mut();
        let messages = &app.model.current_view().unwrap().messages;
        let [(first, _), (second, _)] = &app.list.rows()[0].lines;
        assert!(
            app.selection
                .begin(&app.list, messages, *first + egui::vec2(20.0, 5.0))
        );
        app.selection
            .extend(&app.list, messages, *second + egui::vec2(1000.0, 5.0));
        assert!(app.selection.span().is_some());
        app.set_minimized(true);
        assert_eq!(app.selection.span(), None);
    }

    #[test]
    fn minimizing_closes_the_copy_menu() {
        let (poster, inbox) = channel();
        poster.post(UiEvent::AddMessage {
            slot: 1,
            tabs: vec![0],
            original: "原文一".into(),
            translated: "譯文一".into(),
            msg_id: None,
            pending: false,
            color: None,
        });
        let mut harness = Harness::new_eframe(|cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
        });
        harness.step();
        let app = harness.state_mut();
        let messages = &app.model.current_view().unwrap().messages;
        let [(first, _), (second, _)] = &app.list.rows()[0].lines;
        app.selection
            .begin(&app.list, messages, *first + egui::vec2(20.0, 5.0));
        app.selection
            .extend(&app.list, messages, *second + egui::vec2(1000.0, 5.0));
        let spot = egui::pos2(200.0, 150.0);
        harness.hover_at(spot);
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos: spot,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        harness.step();
        harness.step();
        let label = crate::i18n::t("menu.copy", &[]);
        let menu_shown = |h: &Harness<App>| {
            h.output()
                .shapes
                .iter()
                .any(|c| matches!(&c.shape, egui::Shape::Text(s) if s.galley.text() == label))
        };
        assert!(menu_shown(&harness));
        harness.state_mut().set_minimized(true);
        harness.step();
        assert!(!menu_shown(&harness));
    }

    fn message(original: &str) -> UiEvent {
        UiEvent::AddMessage {
            slot: 1,
            tabs: vec![0],
            original: original.into(),
            translated: "t".into(),
            msg_id: None,
            pending: false,
            color: None,
        }
    }

    #[test]
    fn root_viewport_carries_the_app_icon() {
        // 沒給 icon 時 eframe 會換上 egui 標誌；泡泡沿用根 viewport 的這一份
        for native in [native_options(), warp_options()] {
            let icon = native.viewport.icon.expect("root viewport has an icon");
            assert_eq!((icon.width, icon.height), (48, 48));
            assert_ne!(*icon, egui::IconData::default());
        }
    }

    #[test]
    fn root_viewport_can_start_hidden() {
        for native in [native_options(), warp_options()] {
            let hidden = with_root_visibility(native, false);
            assert_eq!(hidden.viewport.visible, Some(false));
        }
        assert_eq!(
            with_root_visibility(native_options(), true)
                .viewport
                .visible,
            None
        );
    }

    fn app_harness(inbox: Inbox) -> Harness<'static, App> {
        Harness::new_eframe(|cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
        })
    }

    #[test]
    fn minimize_counts_unread_and_expand_resets() {
        let (poster, inbox) = channel();
        poster.post(message("m0"));
        let mut harness = app_harness(inbox);
        harness.step();
        harness.get_by_label("─").click();
        harness.step();
        assert!(harness.state().bubble.is_some());
        poster.post(message("m1"));
        poster.post(message("m2"));
        harness.step();
        assert_eq!(harness.state().model.unread(), 2);
        harness.state_mut().set_minimized(false);
        harness.step();
        let app = harness.state();
        assert!(!app.minimized && app.bubble.is_none());
        assert_eq!(app.model.unread(), 0);
        assert_eq!(app.model.current_view().unwrap().messages.len(), 3);
    }

    #[test]
    fn foreground_poll_runs_only_while_minimized() {
        let (_poster, inbox) = channel();
        let mut harness = app_harness(inbox);
        let delay = |h: &Harness<App>| h.output().viewport_output[&ViewportId::ROOT].repaint_delay;
        harness.step();
        assert!(delay(&harness) > POLL_INTERVAL);
        // 切換顯示時會為了補 NOACTIVATE 立即重繪，等它停下來再看
        harness.state_mut().set_minimized(true);
        harness.run();
        assert!(delay(&harness) <= POLL_INTERVAL);
        harness.state_mut().set_minimized(false);
        harness.run();
        assert!(delay(&harness) > POLL_INTERVAL);
    }

    #[test]
    fn bubble_close_takes_the_root_close_path() {
        let (_poster, inbox) = channel();
        let mut harness = app_harness(inbox);
        harness.state_mut().set_minimized(true);
        harness.step();
        // kittest 把泡泡嵌在根 viewport 裡，關閉要求因此從根送進來
        harness
            .input_mut()
            .viewports
            .entry(ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        harness.step();
        let commands = &harness.output().viewport_output[&ViewportId::ROOT].commands;
        assert!(commands.contains(&ViewportCommand::Close), "{commands:?}");
    }

    #[test]
    fn close_request_runs_on_close_once_before_the_window_goes() {
        let (_poster, inbox) = channel();
        let closes = Rc::new(RefCell::new(0));
        let count = Rc::clone(&closes);
        let mut harness = Harness::new_eframe(|cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
                .with_on_close(move || *count.borrow_mut() += 1)
        });
        harness.step();
        assert_eq!(*closes.borrow(), 0);
        for _ in 0..2 {
            harness
                .input_mut()
                .viewports
                .entry(ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
            harness.step();
        }
        assert_eq!(*closes.borrow(), 1);
    }

    #[test]
    fn bubble_move_is_saved_and_reused_on_next_minimize() {
        let (_poster, inbox) = channel();
        let saved = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&saved);
        let mut harness = Harness::new_eframe(|cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
                .with_bubble_position((Some(-100_000), Some(10)), move |x, y| {
                    sink.borrow_mut().push((x, y))
                })
        });
        harness.state_mut().set_minimized(true);
        harness.step();
        let pending = harness.state().bubble.as_ref().unwrap().pending().unwrap();
        assert!(
            pending.0 > -100_000,
            "saved offscreen position pulled back: {pending:?}"
        );
        let ctx = harness.ctx.clone();
        let app = harness.state_mut();
        app.bubble_action(&ctx, Action::Moved(30, 40));
        assert_eq!(*saved.borrow(), [(30, 40)]);
        app.bubble_action(&ctx, Action::Expand);
        assert!(!app.minimized);
        app.set_minimized(true);
        harness.step();
        let bubble = harness.state().bubble.as_ref().unwrap();
        assert_eq!(bubble.pending(), Some((30, 40)));
    }

    #[test]
    fn default_bubble_position_sticks_after_the_overlay_moves() {
        let (_poster, inbox) = channel();
        let ctx = egui::Context::default();
        let saved = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&saved);
        let mut app = App::new(&ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
            .with_bubble_position((None, None), move |x, y| sink.borrow_mut().push((x, y)));
        let monitors = [(0, 0, 1920, 1080)];
        app.set_minimized(true);
        app.open_bubble((100, 200, 640, 420), 48, &monitors);
        let pending = |app: &App| app.bubble.as_ref().unwrap().pending();
        assert_eq!(pending(&app), Some((100 + 640 - 48, 200)));
        app.set_minimized(false);
        app.set_minimized(true);
        app.open_bubble((900, 700, 640, 420), 48, &monitors);
        assert_eq!(pending(&app), Some((692, 200)));
        assert!(
            saved.borrow().is_empty(),
            "the default is not written to config"
        );
    }

    #[test]
    fn limits_trim_messages_and_fading_wakes_at_the_next_expiry() {
        let (poster, inbox) = channel();
        let mut harness = app_harness(inbox);
        let delay = |h: &Harness<App>| h.output().viewport_output[&ViewportId::ROOT].repaint_delay;
        harness.state_mut().set_limits(10, 30);
        harness.run();
        assert!(
            delay(&harness) > Duration::from_secs(60),
            "empty view: no wake"
        );
        for i in 0..15 {
            poster.post(message(&format!("m{i}")));
        }
        harness.run();
        assert_eq!(
            harness.state().model.current_view().unwrap().messages.len(),
            10
        );
        let wait = delay(&harness);
        assert!(
            wait > Duration::from_secs(25) && wait <= Duration::from_secs(30),
            "{wait:?}"
        );
        harness.state_mut().set_limits(10, 0);
        harness.run();
        assert!(
            delay(&harness) > Duration::from_secs(60),
            "fading off: no wake"
        );
    }

    type Sent = Rc<RefCell<Vec<(String, isize)>>>;
    type Requests = Rc<RefCell<Vec<OutgoingRequest>>>;

    thread_local! {
        static RAISED: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
    }

    struct Wired {
        requests: Requests,
        sent: Sent,
        settings: Arc<RwLock<Config>>,
    }

    /// 接上輸入框的 App：前景與游標是假的，搶前景只記下對象、不碰真的視窗。
    fn input_harness(inbox: Inbox) -> (Harness<'static, App>, Wired) {
        let wired = Wired {
            requests: Requests::default(),
            sent: Sent::default(),
            settings: Arc::new(RwLock::new(
                serde_json::json!({"auto_show_input": false})
                    .as_object()
                    .unwrap()
                    .clone(),
            )),
        };
        let (r, s, settings) = (
            wired.requests.clone(),
            wired.sent.clone(),
            wired.settings.clone(),
        );
        let harness = Harness::new_eframe(move |cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            let input = InputBox::new(
                Box::new(move |req| r.borrow_mut().push(req)),
                Box::new(move |text, hwnd| s.borrow_mut().push((text, hwnd))),
                Arc::new(AtomicBool::new(false)),
                || 0xA,
                || (640, 500),
            );
            let events = GameInputEvents::new(Arc::default(), || 0xA);
            let mut app = App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
                .with_input(input, events, settings);
            app.set_input_desktop(Desktop {
                force_foreground: |hwnd| {
                    RAISED.with(|r| r.borrow_mut().push(hwnd));
                    true
                },
                work_area_at: |_, _| (0, 0, 1920, 1040),
                trigger_held: || false,
            });
            app
        });
        (harness, wired)
    }

    fn input_box<'a>(harness: &'a Harness<'_, App>) -> &'a InputBox {
        &harness.state().input.as_ref().unwrap().input
    }

    #[test]
    fn game_input_events_reach_the_input_box() {
        let (poster, inbox) = channel();
        let (mut harness, wired) = input_harness(inbox);
        let anchor = (300, 400, 500, 40);
        poster.post(UiEvent::GameInputOpened {
            hwnd: 0xA,
            anchor: Some(anchor),
        });
        poster.post(UiEvent::ShowInputBox {
            retarget: Some(0xA),
        });
        harness.step();
        let input = input_box(&harness);
        assert!(input.is_open());
        assert_eq!(input.placement_anchor().0, Some(anchor));
        assert_eq!(input.target_hwnd(), 0xA);
        // 自動顯示每次都從設定讀：關閉時已開啟，所以跟著收起
        wired
            .settings
            .write()
            .unwrap()
            .insert("auto_show_input".into(), true.into());
        poster.post(UiEvent::GameInputClosed { hwnd: 0xA });
        harness.step();
        assert!(!input_box(&harness).is_open());
    }

    #[test]
    fn outgoing_done_is_forwarded_with_its_session() {
        RAISED.with(|r| r.borrow_mut().clear());
        let (poster, inbox) = channel();
        let (mut harness, wired) = input_harness(inbox);
        poster.post(UiEvent::ShowInputBox { retarget: None });
        harness.run_steps(2);
        harness.event(egui::Event::Text("hello".into()));
        harness.step();
        harness.key_press(egui::Key::Enter);
        harness.step();
        let session = wired.requests.borrow()[0].session;
        poster.post(UiEvent::OutgoingDone {
            session: session + 1,
            result: Ok("stale".into()),
        });
        harness.step();
        assert!(wired.sent.borrow().is_empty());
        poster.post(UiEvent::OutgoingDone {
            session,
            result: Ok("hi".into()),
        });
        harness.step();
        assert_eq!(*wired.sent.borrow(), [("hi".to_owned(), 0xA)]);
        assert!(!input_box(&harness).is_open());
        // logic 裡 finish 關窗，同一幀的 ui 就把前景交還給打字對象
        assert_eq!(RAISED.with(|r| r.borrow().clone()), [0xA]);
    }

    #[test]
    fn enter_in_the_frame_the_box_closes_submits_nothing() {
        let (poster, inbox) = channel();
        let (mut harness, wired) = input_harness(inbox);
        poster.post(UiEvent::ShowInputBox { retarget: None });
        harness.run_steps(2);
        harness.event(egui::Event::Text("hello".into()));
        harness.step();
        // kittest 把輸入框嵌在根 viewport 裡，Alt+F4 因此從根送進來
        harness
            .input_mut()
            .viewports
            .entry(ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        harness.key_press(egui::Key::Enter);
        harness.step();
        assert!(!input_box(&harness).is_open());
        assert!(wired.requests.borrow().is_empty());
    }

    #[test]
    fn quit_closes_the_root_viewport() {
        let (poster, inbox) = channel();
        let mut harness = app_harness(inbox);
        harness.step();
        poster.post(UiEvent::Quit);
        harness.step();
        let commands = &harness.output().viewport_output[&ViewportId::ROOT].commands;
        assert!(commands.contains(&ViewportCommand::Close), "{commands:?}");
    }

    #[test]
    fn hotkey_opens_the_input_box_while_minimized() {
        let (poster, inbox) = channel();
        let (mut harness, _wired) = input_harness(inbox);
        harness.state_mut().set_minimized(true);
        harness.step();
        poster.post(UiEvent::ShowInputBox {
            retarget: Some(0xA),
        });
        harness.run_steps(2);
        let app = harness.state();
        assert!(app.input.as_ref().unwrap().input.is_open());
        assert!(app.minimized && app.bubble.is_some());
        harness.get_by_role(egui::accesskit::Role::TextInput);
    }

    /// 接上框選流程（替身桌面與 runner）的 App：搶前景不碰真的視窗。
    fn region_harness(inbox: Inbox) -> (Harness<'static, App>, fakes::Calls) {
        let (flow, calls) = fakes::flow();
        let harness = Harness::new_eframe(move |cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            let mut app = App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
                .with_region(flow);
            app.set_region_desktop(Desktop {
                force_foreground: |_| true,
                work_area_at: |_, _| (0, 0, 1920, 1040),
                trigger_held: || false,
            });
            app
        });
        (harness, calls)
    }

    fn selecting(harness: &mut Harness<'_, App>) -> bool {
        let region = harness.state_mut().region.as_mut().unwrap();
        region.flow.selector_mut().is_some()
    }

    #[test]
    fn region_hotkey_toggles_the_flow() {
        let (poster, inbox) = channel();
        let (mut harness, calls) = region_harness(inbox);
        poster.post(UiEvent::RegionHotkey { hwnd: fakes::GAME });
        harness.step();
        assert!(selecting(&mut harness));
        poster.post(UiEvent::RegionHotkey { hwnd: fakes::GAME });
        harness.step();
        assert!(!selecting(&mut harness));
        assert_eq!(*calls.borrow(), ["capture 0x1234", "foreground 0x1234"]);
    }

    #[test]
    fn region_button_starts_from_the_button() {
        let (_poster, inbox) = channel();
        let (mut harness, calls) = region_harness(inbox);
        harness.step();
        harness.get_by_label("⛶").click();
        harness.step();
        assert!(selecting(&mut harness));
        assert_eq!(*calls.borrow(), ["capture 0x9999"]);
    }

    #[test]
    fn region_done_is_forwarded_with_its_session() {
        let (poster, inbox) = channel();
        let (mut harness, calls) = region_harness(inbox);
        poster.post(UiEvent::RegionHotkey { hwnd: fakes::GAME });
        harness.step();
        let region = harness.state_mut().region.as_mut().unwrap();
        region
            .flow
            .on_selector(SelectorAction::Selected(fakes::RECT));
        assert_eq!(calls.borrow().last().unwrap(), "run 1");
        let card_text = |h: &mut Harness<'_, App>| {
            let region = h.state_mut().region.as_mut().unwrap();
            region.flow.card_mut().unwrap().all_text()
        };
        let pending = card_text(&mut harness);
        let done = |session| UiEvent::RegionDone {
            session,
            result: Ok(RegionResult {
                text: "譯文".into(),
                source: String::new(),
            }),
        };
        poster.post(done(2));
        harness.step();
        assert_eq!(card_text(&mut harness), pending, "stale session dropped");
        poster.post(done(1));
        harness.step();
        assert_eq!(card_text(&mut harness), "譯文");
    }

    #[test]
    fn overlay_alpha_reaches_the_region_card() {
        let (poster, inbox) = channel();
        let (mut harness, _calls) = region_harness(inbox);
        poster.post(UiEvent::RegionHotkey { hwnd: fakes::GAME });
        harness.step();
        let region = harness.state_mut().region.as_mut().unwrap();
        region
            .flow
            .on_selector(SelectorAction::Selected(fakes::RECT));
        harness.step();
        let region = harness.state_mut().region.as_mut().unwrap();
        assert_eq!(region.flow.card_mut().unwrap().alpha(), 0.8);
    }

    #[test]
    fn every_viewport_lookup_skips_the_other_viewports() {
        let (_poster, inbox) = channel();
        let ctx = egui::Context::default();
        let input = InputBox::new(
            Box::new(|_| {}),
            Box::new(|_, _| {}),
            Arc::new(AtomicBool::new(false)),
            || 0xA,
            || (640, 500),
        );
        let events = GameInputEvents::new(Arc::default(), || 0xA);
        let settings = Arc::new(RwLock::new(Config::new()));
        let mut app = App::new(&ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
            .with_input(input, events, settings)
            .with_region(fakes::flow().0);
        app.hwnd = 1;
        app.bubble = Some(Bubble::new((0, 0), Instant::now()));
        let set = |app: &mut App, role: usize, hwnd: isize| match role {
            0 => app.hwnd = hwnd,
            1 => app.bubble.as_mut().unwrap().attach(hwnd, 0),
            2 => app.input.as_mut().unwrap().hwnd = hwnd,
            3 => app.region.as_mut().unwrap().selector.hwnd = hwnd,
            4 => app.region.as_mut().unwrap().region_box.hwnd = hwnd,
            _ => app.region.as_mut().unwrap().card.hwnd = hwnd,
        };
        let titled: Vec<isize> = (1..=6).collect();
        for (role, &hwnd) in titled.iter().enumerate() {
            set(&mut app, role, hwnd);
        }
        assert_eq!(app.viewport_hwnds(), titled);
        // 每個視窗建立時（自己還是 0）認標題只會認到自己，不會認成其他同標題的視窗
        for (role, &hwnd) in titled.iter().enumerate() {
            set(&mut app, role, 0);
            let known = app.viewport_hwnds();
            let by_title = || {
                titled
                    .iter()
                    .copied()
                    .find(|h| !known.contains(h))
                    .unwrap_or(0)
            };
            assert_eq!(
                win32::pick_new_window(&titled, &titled, &known, by_title),
                hwnd,
                "role {role}"
            );
            set(&mut app, role, hwnd);
        }
    }

    #[test]
    fn posting_after_start_requests_repaint() {
        let (poster, inbox) = channel();
        let ctx = egui::Context::default();
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        ctx.set_request_repaint_callback(move |_| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        let _app = App::new(&ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}));
        poster.post(UiEvent::ClearError);
        assert!(hits.load(std::sync::atomic::Ordering::SeqCst) > 0);
    }
}
