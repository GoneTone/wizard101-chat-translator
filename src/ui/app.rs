//! egui 應用程式外殼：overlay 是根 viewport（縮小時根視窗本身縮成泡泡），翻譯輸入框、框選與設定的視窗是 immediate viewport，每幀先取完 UI 事件再繪製。

use std::sync::mpsc::Receiver;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Instant;

use eframe::egui::{self, Color32, UiBuilder, ViewportCommand, ViewportId, WindowLevel};

use serde_json::Value;

use crate::config::py::py_bool;
use crate::config::{Config, Settings, app_name};
use crate::i18n::current_language;
use crate::input::dispatch::GameInputEvents;
use crate::logs::log;
use crate::ui::events::{UiEvent, UiPoster};
use crate::ui::geometry::Rect;
use crate::ui::input_box::InputBox;
use crate::ui::input_host::{Desktop, InputHost};
use crate::ui::overlay::banners::Banners;
use crate::ui::overlay::bubble::{Action, BUBBLE_SIZE, Bubble, bubble_alpha, initial_position};
use crate::ui::overlay::frame::{BAR_HEIGHT, Clicked, Frame, Shell, Win32Shell, describe};
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

/// 設定視窗按儲存：收到視窗管的那些鍵（不是完整設定），回傳實際生效的框選熱鍵（沒註冊為 `None`）。
pub type OnSave = Box<dyn FnMut(Config) -> Option<String>>;
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
    /// 根視窗的位置大小操作；測試換成假的。
    shell: Box<dyn Shell>,
    frame: Frame,
    minimized: bool,
    /// 根視窗目前擺成泡泡的位置大小（與 `minimized` 不同＝切換中）。
    as_bubble: bool,
    /// 上一幀什麼都沒畫：呈現的是全透明畫面，這時改視窗大小才不會閃出舊畫面。
    blank: bool,
    /// 切換期間 DWM 不顯示根視窗：改大小後 swapchain 裡殘留的舊畫面才不會被看到。
    cloaked: bool,
    /// 隱形中已畫過一幀新內容；下一幀等它合成上螢幕後才解除隱形。
    drawn_while_cloaked: bool,
    /// 縮成泡泡前的 overlay 矩形（實體像素）；擺回去時取走。
    overlay_rect: Option<Rect>,
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
    /// overlay 根視窗與泡泡目前的疊放層級；設定視窗開著時降為一般層級。
    overlay_level: WindowLevel,
}

impl App {
    /// 綁上 Context 讓投遞端能喚醒重繪；`frame` 是 overlay 的外框（含底板不透明度與起始位置）。
    pub fn new(ctx: &egui::Context, inbox: Inbox, frame: Frame) -> Self {
        let _ = inbox.ctx.set(ctx.clone());
        ctx.all_styles_mut(|style| style.interaction.tooltip_delay = 0.5);
        Self {
            rx: inbox.rx,
            hwnd: 0,
            shell: Box::new(Win32Shell(0)),
            frame,
            minimized: false,
            as_bubble: false,
            blank: false,
            cloaked: false,
            drawn_while_cloaked: false,
            overlay_rect: None,
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
            overlay_level: WindowLevel::AlwaysOnTop,
        }
    }

    /// 接上翻譯輸入框與遊戲聊天框事件；`settings` 用來讀 `auto_show_input`。
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
    pub fn set_on_close(&mut self, on_close: impl FnOnce() + 'static) {
        self.on_close = Some(Box::new(on_close));
    }

    /// 接上設定視窗：⚙ 以 `settings` 當下的內容開窗，按儲存時呼叫 `on_save` 再套用 overlay 端的設定。
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

    /// 目前畫面套用的介面語言碼。
    pub fn ui_language(&self) -> &str {
        &self.ui_language
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

    /// overlay 根視窗與泡泡目前的疊放層級。
    pub fn overlay_level(&self) -> WindowLevel {
        self.overlay_level
    }

    /// 設定視窗開著時 overlay 與泡泡降為一般層級，免得置頂的 overlay 蓋住它；關窗（儲存、取消、✕）後恢復置頂。
    fn sync_overlay_level(&mut self, ctx: &egui::Context) {
        let open = self.settings.as_ref().is_some_and(|w| w.host.is_open());
        let level = if open {
            WindowLevel::Normal
        } else {
            WindowLevel::AlwaysOnTop
        };
        if level == self.overlay_level {
            return;
        }
        self.overlay_level = level;
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::WindowLevel(level));
        log(format!(
            "[ui] overlay window level set to {level:?} (settings window open={open})"
        ));
    }

    /// overlay 的 HWND；第一幀之前是 0。
    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    /// 縮成泡泡（根視窗經 [`Self::transition`] 縮成泡泡大小、改畫泡泡，訊息照常累積）或展開回 overlay；已在該狀態時不動。
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
        let follow = py_bool(self.list.follow());
        log(format!("[ui] {what} (messages={messages} follow={follow})"));
    }

    /// 根視窗該縮成的泡泡矩形（實體像素）；`ppp` 是 overlay 目前的縮放。
    ///
    /// 起點記成這次執行期間的泡泡位置（不寫回設定），之後縮小都沿用，overlay 移動也不跟著跑。
    fn bubble_rect(&mut self, overlay: Rect, ppp: f32) -> Rect {
        let px = |scale: f32| (BUBBLE_SIZE as f32 * scale).round() as i32;
        let monitors = self.shell.monitors();
        let (x, y) = initial_position(self.bubble_position, overlay, px(ppp), &monitors);
        self.bubble_position = (Some(x), Some(y));
        // 混合 DPI 下泡泡所在的螢幕可能與 overlay 不同，邊長以那顆的縮放為準
        let size = px(self.shell.scale_at(x, y));
        (x, y, size, size)
    }

    /// 根視窗在 overlay 與泡泡之間切換的一幀：一律不畫，回傳這一幀結束時要擺的矩形。
    ///
    /// 改大小時 DWM 會把上一次呈現的畫面拉伸或裁切著顯示到下一次呈現，所以先呈現一幀全透明才擺；
    /// 擺的那一幀 surface 還是舊大小（eframe 收到 Resized 才換），也只能是全透明，下一幀才畫新內容。
    fn transition(&mut self, ctx: &egui::Context) -> Option<Rect> {
        ctx.request_repaint();
        self.drawn_while_cloaked = false;
        if !std::mem::replace(&mut self.cloaked, true) {
            self.shell.set_cloaked(true);
        }
        if !std::mem::replace(&mut self.blank, true) {
            return None;
        }
        self.as_bubble = self.minimized;
        if !self.minimized {
            let overlay = self.overlay_rect.take()?;
            log(format!(
                "[ui] overlay window restored from the bubble: {}",
                describe(overlay)
            ));
            return Some(overlay);
        }
        let overlay = self.shell.rect();
        self.overlay_rect = Some(overlay);
        let bubble = self.bubble_rect(overlay, ctx.pixels_per_point());
        log(format!(
            "[ui] overlay window shrinking to the bubble: overlay={} bubble={}",
            describe(overlay),
            describe(bubble)
        ));
        Some(bubble)
    }

    /// 隱形中先畫一幀新內容，下一幀等它合成上螢幕後才恢復顯示。
    fn reveal_after_redraw(&mut self, ctx: &egui::Context) {
        if !self.cloaked {
            return;
        }
        if std::mem::replace(&mut self.drawn_while_cloaked, true) {
            self.shell.wait_for_composition();
            self.shell.set_cloaked(false);
            self.cloaked = false;
            self.drawn_while_cloaked = false;
        } else {
            ctx.request_repaint();
        }
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

    /// 所有已知 viewport 的 HWND（overlay、輸入框、選取畫面、框、卡片、設定視窗、精靈；0 不列）：找新視窗時一律略過。
    fn viewport_hwnds(&self) -> Vec<isize> {
        let input = self.input.as_ref().map_or(0, InputHost::hwnd);
        let region = self.region.as_ref().map_or([0; 3], RegionHost::hwnds);
        let settings = self.settings.as_ref().map_or(0, |s| s.host.hwnd());
        let wizard = self.first_run.as_ref().map_or(0, |w| w.host.hwnd());
        [self.hwnd, input]
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

    /// 縮小期間每幀在根視窗畫泡泡、處理它的點擊／拖曳，並每 300ms 檢查一次是否該自動展開。
    fn show_bubble(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let (unread, alpha) = (self.model.unread(), bubble_alpha(self.frame.alpha()));
        let hwnd = self.hwnd;
        let bubble = self
            .bubble
            .get_or_insert_with(|| Bubble::new(hwnd, win32::foreground_window(), Instant::now()));
        // 以視窗實際的大小為準：跨 DPI 拖過去時視窗維持實體大小
        let (_, _, w, h) = self.shell.rect();
        let area =
            egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(w as f32, h as f32) / ppp);
        let shell = self.shell.as_mut();
        let action = ui
            .scope_builder(UiBuilder::new().max_rect(area), |ui| {
                bubble.show(ui, shell, unread, alpha)
            })
            .inner;
        let now = Instant::now();
        let expand = bubble.watch(now, win32::foreground_window(), self.shell.as_ref());
        ctx.request_repaint_after(bubble.next_poll().saturating_duration_since(now));
        match action {
            Some(action) => self.bubble_action(action),
            None if expand => self.set_minimized(false),
            None => {}
        }
        if !self.minimized {
            ctx.request_repaint();
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
                let region_hotkey = (wiring.on_save)(cfg.clone());
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

    fn bubble_action(&mut self, action: Action) {
        match action {
            Action::Expand => self.set_minimized(false),
            Action::Moved(x, y) => {
                log(format!("[ui] bubble moved: +{x}+{y}"));
                self.bubble_position = (Some(x), Some(y));
                (self.on_bubble_move)(x, y);
            }
        }
    }

    /// overlay 的外框（拖曳、縮放、標題列按鈕）與內容。
    fn show_overlay(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        match self.frame.show(
            ui,
            self.shell.as_mut(),
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
            Some(Clicked::Minimize) => {
                self.set_minimized(true);
                ctx.request_repaint();
            }
            // 與 Alt+F4、工作列關閉同一條路：關掉根 viewport
            Some(Clicked::Close) => ctx.send_viewport_cmd(ViewportCommand::Close),
            None => {}
        }
        self.content(ui);
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
                self.shell = Box::new(Win32Shell(self.hwnd));
            }
        }
        let visible = !self.in_first_run();
        if self.shown != visible {
            self.shown = visible;
            ctx.send_viewport_cmd(ViewportCommand::Visible(self.shown));
            // winit 處理 Visible 時會重寫 extended style 洗掉 NOACTIVATE，要保證下一幀補回
            ctx.request_repaint();
        }
        if self.hwnd != 0 {
            win32::make_non_activating(self.hwnd);
        }
        let place = if self.minimized != self.as_bubble {
            self.transition(&ctx)
        } else {
            self.blank = false;
            self.reveal_after_redraw(&ctx);
            if self.minimized {
                self.show_bubble(ui);
            } else {
                self.show_overlay(ui);
            }
            None
        };
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
        self.sync_overlay_level(&ctx);
        if let Some(rect) = place {
            // 上一幀的全透明畫面要先真的上螢幕才能搬，否則 DWM 會把更早的內容帶到新位置
            self.shell.wait_for_composition();
            if !self.shell.set_rect(rect) {
                log(format!(
                    "[ui] overlay window could not be placed: hwnd={:#x} rect={}",
                    self.hwnd,
                    describe(rect)
                ));
            }
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        Color32::TRANSPARENT.to_normalized_gamma_f32()
    }
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
    use crate::ui::overlay::frame::tests::FakeShell;
    use crate::ui::overlay::message_list::ROW_PAD;
    use crate::ui::overlay::outlined::line_height;
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
        let mut harness = app_harness(inbox);
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
    fn empty_queue_is_a_noop() {
        let (_poster, inbox) = channel();
        let mut harness = app_harness(inbox);
        harness.step();
        let app = harness.state();
        assert_eq!(app.status, None);
        assert_eq!(app.model.client(), None);
        assert_eq!(app.banners.error_text(), None);
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
        let mut harness = app_harness(inbox);
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
        let mut harness = app_harness(inbox);
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
        let mut harness = app_harness(inbox);
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
        let mut harness = app_harness(inbox);
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
        harness.run_steps(4);
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

    fn long_message(i: usize) -> UiEvent {
        UiEvent::AddMessage {
            slot: 1,
            tabs: vec![0],
            original: format!("message {i}: a chat line long enough to wrap in a narrow window"),
            translated: format!("第 {i} 則譯文，長度足以換行，窄視窗下一定會換成兩行以上"),
            msg_id: None,
            pending: false,
            color: None,
        }
    }

    /// 最新一則完整落在清單視口底緣。
    fn newest_is_in_view(app: &App) -> bool {
        let Some(last) = app.list.rows().last() else {
            return false;
        };
        let newest = app
            .model
            .current_view()
            .unwrap()
            .messages
            .last()
            .unwrap()
            .seq;
        let (origin, galley) = &last.lines[1];
        let bottom = origin.y + line_height(galley) + ROW_PAD;
        last.seq == newest && (bottom - app.list.viewport().max.y).abs() < 0.01
    }

    #[test]
    fn expand_reanchors_view_to_bottom() {
        let (poster, inbox) = channel();
        for i in 0..40 {
            poster.post(long_message(i));
        }
        let mut harness = app_harness(inbox);
        harness.step();
        assert!(newest_is_in_view(harness.state()));
        harness.state_mut().set_minimized(true);
        harness.step();
        for i in 100..106 {
            poster.post(long_message(i));
            harness.step();
        }
        harness.state_mut().set_minimized(false);
        harness.run_steps(3);
        assert!(harness.state().list.follow());
        assert!(newest_is_in_view(harness.state()));
    }

    #[test]
    fn messages_received_as_a_bubble_are_visible_after_expand() {
        let (poster, inbox) = channel();
        for i in 0..40 {
            poster.post(long_message(i));
        }
        let mut harness = app_harness(inbox);
        harness.step();
        harness.get_by_label("─").click();
        harness.run_steps(4);
        assert!(harness.state().bubble.is_some());
        for i in 200..204 {
            poster.post(long_message(i));
            harness.step();
        }
        harness.state_mut().set_minimized(false);
        harness.run_steps(3);
        assert!(newest_is_in_view(harness.state()));
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
    fn closing_the_bubble_runs_on_close() {
        let (_poster, inbox) = channel();
        let closes = Rc::new(RefCell::new(0));
        let count = Rc::clone(&closes);
        let mut harness = app_harness(inbox);
        harness
            .state_mut()
            .set_on_close(move || *count.borrow_mut() += 1);
        harness.state_mut().set_minimized(true);
        harness.step();
        // 泡泡就是根視窗：Alt+F4、工作列關閉與 ✕ 同一條路
        harness
            .input_mut()
            .viewports
            .entry(ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        harness.step();
        assert_eq!(*closes.borrow(), 1);
    }

    #[test]
    fn close_request_runs_on_close_once_before_the_window_goes() {
        let (_poster, inbox) = channel();
        let closes = Rc::new(RefCell::new(0));
        let count = Rc::clone(&closes);
        let mut harness = app_harness(inbox);
        harness
            .state_mut()
            .set_on_close(move || *count.borrow_mut() += 1);
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

    const OVERLAY: Rect = (100, 200, 640, 420);

    /// 根視窗的替身：App 與測試共用同一份矩形、游標與隱形狀態；`1` 是泡泡所在螢幕的縮放。
    struct SharedShell(Rc<RefCell<FakeShell>>, f32, Rc<std::cell::Cell<bool>>);

    impl Shell for SharedShell {
        fn cursor(&self) -> (i32, i32) {
            self.0.borrow().cursor
        }
        fn rect(&self) -> Rect {
            self.0.borrow().rect
        }
        fn set_rect(&mut self, rect: Rect) -> bool {
            self.0.borrow_mut().set_rect(rect)
        }
        fn monitors(&self) -> Vec<Rect> {
            self.0.borrow().monitors()
        }
        fn scale_at(&self, _x: i32, _y: i32) -> f32 {
            self.1
        }
        fn set_cloaked(&mut self, cloaked: bool) {
            self.2.set(cloaked);
        }
    }

    type Saved<T> = Rc<RefCell<Vec<T>>>;

    /// 根視窗換成替身的 App 留給測試檢查的東西。
    struct Root {
        shell: Rc<RefCell<FakeShell>>,
        /// overlay 拖曳／縮放放開時存下的幾何。
        geometry: Saved<Rect>,
        /// 泡泡拖曳放開時存下的位置。
        moves: Saved<(i32, i32)>,
        /// DWM 目前是否不顯示根視窗。
        cloaked: Rc<std::cell::Cell<bool>>,
    }

    /// 根視窗擺在 [`OVERLAY`] 的 App；`saved` 是設定裡的泡泡位置，`scale` 是泡泡所在螢幕的縮放。
    fn root_harness(
        inbox: Inbox,
        saved: (Option<i32>, Option<i32>),
        scale: f32,
    ) -> (Harness<'static, App>, Root) {
        let fake = FakeShell {
            cursor: (0, 0),
            rect: OVERLAY,
            ready: true,
        };
        let root = Root {
            shell: Rc::new(RefCell::new(fake)),
            geometry: Saved::default(),
            moves: Saved::default(),
            cloaked: Rc::default(),
        };
        let shell = SharedShell(Rc::clone(&root.shell), scale, Rc::clone(&root.cloaked));
        let (geometry, moves) = (Rc::clone(&root.geometry), Rc::clone(&root.moves));
        let mut harness = Harness::new_eframe(move |cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            let frame = Frame::new(0.8, None, move |x, y, w, h| {
                geometry.borrow_mut().push((x, y, w, h))
            });
            let mut app = App::new(&cc.egui_ctx, inbox, frame);
            app.set_bubble_position(saved, move |x, y| moves.borrow_mut().push((x, y)));
            app.shell = Box::new(shell);
            app
        });
        harness.step();
        (harness, root)
    }

    /// 在泡泡中央按下、游標（螢幕實體像素）移動 `by` 後放開。
    fn press_bubble(harness: &mut Harness<App>, root: &Root, by: (i32, i32)) {
        let at = egui::pos2(24.0, 24.0);
        let (x, y, _, _) = root.shell.borrow().rect;
        root.shell.borrow_mut().cursor = (x + 24, y + 24);
        harness.hover_at(at);
        harness.drag_at(at);
        harness.step();
        root.shell.borrow_mut().cursor = (x + 24 + by.0, y + 24 + by.1);
        harness.step();
        harness.drop_at(at);
        harness.step();
    }

    fn painted_text(harness: &Harness<App>, text: &str) -> bool {
        harness
            .output()
            .shapes
            .iter()
            .any(|c| matches!(&c.shape, egui::Shape::Text(s) if s.galley.text() == text))
    }

    /// 這一幀根視窗什麼都沒畫（呈現全透明）。
    fn drew_nothing(harness: &Harness<App>) -> bool {
        harness.output().shapes.is_empty()
    }

    const TOP_RIGHT: Rect = (100 + 640 - 48, 200, 48, 48);

    #[test]
    fn minimize_and_expand_blank_a_frame_before_and_while_the_window_moves() {
        let (_poster, inbox) = channel();
        let (mut harness, root) = root_harness(inbox, (None, None), 1.0);
        harness.get_by_label("─").click();
        harness.step();
        assert!(harness.state().minimized);
        // 1. 隱形，原位置呈現一幀全透明
        harness.step();
        assert!(drew_nothing(&harness));
        assert!(root.cloaked.get());
        assert_eq!(root.shell.borrow().rect, OVERLAY);
        // 2. 縮成泡泡，這一幀也不畫
        harness.step();
        assert!(drew_nothing(&harness));
        assert_eq!(root.shell.borrow().rect, TOP_RIGHT);
        // 3. 新大小畫泡泡，仍隱形
        harness.step();
        assert!(!drew_nothing(&harness));
        assert!(root.cloaked.get());
        assert!(harness.query_by_label("─").is_none(), "title bar is gone");
        // 4. 泡泡已上螢幕才恢復顯示
        harness.step();
        assert!(!root.cloaked.get());

        harness.state_mut().set_minimized(false);
        harness.step();
        assert!(drew_nothing(&harness));
        assert!(root.cloaked.get());
        assert_eq!(root.shell.borrow().rect, TOP_RIGHT);
        harness.step();
        assert!(drew_nothing(&harness));
        assert_eq!(root.shell.borrow().rect, OVERLAY);
        harness.step();
        harness.get_by_label("─");
        assert!(root.cloaked.get());
        harness.step();
        assert!(!root.cloaked.get());
        assert!(root.geometry.borrow().is_empty());
    }

    #[test]
    fn reversing_mid_transition_settles_in_the_requested_state() {
        let (_poster, inbox) = channel();
        let (mut harness, root) = root_harness(inbox, (None, None), 1.0);
        // 透明幀之後、擺之前就展開：視窗沒動過，直接畫回 overlay
        harness.state_mut().set_minimized(true);
        harness.step();
        harness.state_mut().set_minimized(false);
        harness.step();
        assert_eq!(root.shell.borrow().rect, OVERLAY);
        harness.get_by_label("─");

        // 擺成泡泡之後、畫泡泡之前就展開：上一幀已是透明，下一幀直接擺回
        harness.state_mut().set_minimized(true);
        harness.run_steps(2);
        assert_eq!(root.shell.borrow().rect, TOP_RIGHT);
        harness.state_mut().set_minimized(false);
        harness.step();
        assert!(drew_nothing(&harness));
        assert_eq!(root.shell.borrow().rect, OVERLAY);
        harness.step();
        harness.get_by_label("─");

        // 展開的透明幀之後又縮小：視窗仍是泡泡，直接畫泡泡
        harness.state_mut().set_minimized(true);
        harness.run_steps(3);
        harness.state_mut().set_minimized(false);
        harness.step();
        harness.state_mut().set_minimized(true);
        harness.step();
        assert!(!drew_nothing(&harness));
        assert_eq!(root.shell.borrow().rect, TOP_RIGHT);
        assert!(harness.state().bubble.is_some());
        harness.state_mut().set_minimized(false);
        harness.run_steps(3);
        assert_eq!(root.shell.borrow().rect, OVERLAY);
        harness.get_by_label("─");
        assert!(root.geometry.borrow().is_empty());
        harness.step();
        assert!(!root.cloaked.get(), "settled windows are shown again");
    }

    #[test]
    fn the_bubble_shows_unread_and_a_click_expands_it() {
        let (poster, inbox) = channel();
        let (mut harness, root) = root_harness(inbox, (None, None), 1.0);
        harness.state_mut().set_minimized(true);
        harness.run_steps(3);
        poster.post(message("m1"));
        poster.post(message("m2"));
        harness.step();
        assert!(painted_text(&harness, "2"), "unread badge");

        press_bubble(&mut harness, &root, (0, 0));
        assert!(!harness.state().minimized);
        harness.run_steps(3);
        assert_eq!(root.shell.borrow().rect, OVERLAY);
        harness.get_by_label("─");
        assert!(root.geometry.borrow().is_empty());
        assert!(root.moves.borrow().is_empty());
    }

    #[test]
    fn dragging_the_bubble_moves_the_root_without_saving_overlay_geometry() {
        let (_poster, inbox) = channel();
        let (mut harness, root) = root_harness(inbox, (None, None), 1.0);
        harness.state_mut().set_minimized(true);
        harness.run_steps(3);
        press_bubble(&mut harness, &root, (-300, 150));
        assert_eq!(root.shell.borrow().rect, (392, 350, 48, 48));
        assert_eq!(*root.moves.borrow(), [(392, 350)]);
        assert!(harness.state().minimized);
        harness.state_mut().set_minimized(false);
        harness.run_steps(2);
        assert_eq!(root.shell.borrow().rect, OVERLAY);
        assert!(root.geometry.borrow().is_empty(), "{:?}", root.geometry);
    }

    #[test]
    fn the_bubble_takes_the_scale_of_its_own_monitor() {
        let (_poster, inbox) = channel();
        let (mut harness, root) = root_harness(inbox, (Some(1000), Some(30)), 1.5);
        harness.state_mut().set_minimized(true);
        harness.run_steps(2);
        assert_eq!(root.shell.borrow().rect, (1000, 30, 72, 72));
        harness.state_mut().set_minimized(false);
        harness.run_steps(2);
        assert_eq!(root.shell.borrow().rect, OVERLAY);
    }

    #[test]
    fn bubble_move_is_saved_and_reused_on_next_minimize() {
        let (_poster, inbox) = channel();
        let (mut harness, root) = root_harness(inbox, (Some(-100_000), Some(10)), 1.0);
        harness.state_mut().set_minimized(true);
        harness.run_steps(2);
        let placed = root.shell.borrow().rect;
        assert!(
            placed.0 > -100_000,
            "saved offscreen position pulled back: {placed:?}"
        );
        let app = harness.state_mut();
        app.bubble_action(Action::Moved(30, 40));
        assert_eq!(*root.moves.borrow(), [(30, 40)]);
        app.bubble_action(Action::Expand);
        assert!(!app.minimized);
        harness.run_steps(3);
        assert_eq!(root.shell.borrow().rect, OVERLAY);
        harness.state_mut().set_minimized(true);
        harness.run_steps(2);
        assert_eq!(root.shell.borrow().rect, (30, 40, 48, 48));
    }

    #[test]
    fn default_bubble_position_sticks_after_the_overlay_moves() {
        let (_poster, inbox) = channel();
        let ctx = egui::Context::default();
        let saved = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&saved);
        let mut app = App::new(&ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}));
        app.set_bubble_position((None, None), move |x, y| sink.borrow_mut().push((x, y)));
        app.shell = Box::new(FakeShell {
            cursor: (0, 0),
            rect: OVERLAY,
            ready: true,
        });
        app.set_minimized(true);
        let first = app.bubble_rect(OVERLAY, 1.0);
        assert_eq!(first, (100 + 640 - 48, 200, 48, 48));
        app.set_minimized(false);
        app.set_minimized(true);
        assert_eq!(app.bubble_rect((900, 700, 640, 420), 1.0), first);
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
            let mut app = App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}));
            app.set_input(input, events, settings);
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
            let mut app = App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}));
            app.set_region(flow);
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
        let mut app = App::new(&ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}));
        app.set_input(input, events, settings);
        app.set_region(fakes::flow().0);
        app.hwnd = 1;
        let set = |app: &mut App, role: usize, hwnd: isize| match role {
            0 => app.hwnd = hwnd,
            1 => app.input.as_mut().unwrap().hwnd = hwnd,
            2 => app.region.as_mut().unwrap().selector.hwnd = hwnd,
            3 => app.region.as_mut().unwrap().region_box.hwnd = hwnd,
            _ => app.region.as_mut().unwrap().card.hwnd = hwnd,
        };
        let titled: Vec<isize> = (1..=5).collect();
        for (role, &hwnd) in titled.iter().enumerate() {
            set(&mut app, role, hwnd);
        }
        assert_eq!(app.viewport_hwnds(), titled);
        // 每個視窗建立時（自己還是 0），已知清單要列出其餘四個，認標題才不會認錯
        for (role, &hwnd) in titled.iter().enumerate() {
            set(&mut app, role, 0);
            let others: Vec<isize> = titled.iter().copied().filter(|h| *h != hwnd).collect();
            assert_eq!(app.viewport_hwnds(), others, "role {role}");
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
