//! 在 App 裡托管設定視窗與首次設定精靈：有系統標題列、可縮放、有工作列按鈕的一般 immediate viewport。
//!
//! 建立那一幀藏著、找到 HWND 就套深色標題列，下一幀顯示並在同一幀搶前景；設定視窗建立時置頂、顯示那一幀就放掉。

use eframe::egui::{self, Ui, Vec2, ViewportBuilder, ViewportCommand, ViewportId, WindowLevel};

use crate::config::{Config, app_name};
use crate::i18n::t;
use crate::logs::log;
use crate::ui::geometry::Rect;
use crate::ui::icon::app_icon;
use crate::ui::settings::env::{SettingsEnv, SettingsEvent};
use crate::ui::settings::theme;
use crate::ui::settings::window::{SettingsAction, SettingsWindow};
use crate::ui::settings::wizard::{Wizard, WizardAction};
use crate::ui::{viewport, win32};

/// 視窗與工作區每邊留的邊距（point）。
const SCREEN_MARGIN: f32 = 24.0;

/// 開窗大小：`want` 夾在工作區（point）內、每邊各留 24 的邊距。
pub fn initial_size(want: (f32, f32), work_area_points: (f32, f32)) -> (f32, f32) {
    let fit = |want: f32, area: f32| want.min(area - 2.0 * SCREEN_MARGIN);
    (
        fit(want.0, work_area_points.0),
        fit(want.1, work_area_points.1),
    )
}

/// 托管視窗對桌面的操作；測試與 probe 換成不碰真視窗的版本。
#[derive(Clone, Copy)]
pub struct WindowOps {
    /// 建立 viewport 前的執行緒頂層視窗；傳入目前的 HWND，已知時不必列舉。
    pub windows_before: fn(isize) -> Vec<isize>,
    /// 認出新視窗：建立前的清單、要略過的已知視窗與視窗標題；找不到回 0。
    pub find_new: fn(&[isize], &[isize], &str) -> isize,
    /// 系統標題列改深色。
    pub dark_caption: fn(isize),
    /// 把視窗切到前景。
    pub force_foreground: fn(isize) -> bool,
    /// 游標所在螢幕的工作區（實體像素）與縮放。
    pub cursor_work_area: fn() -> (Rect, f32),
}

impl WindowOps {
    /// 接 Win32 的真實操作。
    pub fn real() -> Self {
        Self {
            windows_before: viewport::windows_before,
            find_new: viewport::find_new_window,
            dark_caption: win32::dark_caption,
            force_foreground: win32::force_foreground,
            cursor_work_area: || {
                let (x, y) = win32::cursor_pos();
                (win32::work_area_at(x, y), win32::scale_at(x, y))
            },
        }
    }
}

/// 一種托管視窗的固定設定。
#[derive(Clone, Copy)]
pub struct Spec {
    /// log 用的名稱。
    pub name: &'static str,
    title_key: &'static str,
    size: Vec2,
    min_size: Vec2,
    /// 建立時置頂、顯示那一幀放掉（從置頂的 overlay 叫出來才不會被它蓋住）。
    topmost_once: bool,
}

const SETTINGS: Spec = Spec {
    name: "settings window",
    title_key: "settings.title",
    size: theme::SETTINGS_SIZE,
    min_size: theme::SETTINGS_MIN_SIZE,
    topmost_once: true,
};

const WIZARD: Spec = Spec {
    name: "wizard",
    title_key: "wizard.title",
    size: theme::WIZARD_SIZE,
    min_size: theme::WIZARD_MIN_SIZE,
    topmost_once: false,
};

/// 一個托管視窗的 HWND 與顯示狀態；關窗後換下一個 viewport id，重開就是新視窗、重新置中。
struct HostedWindow {
    spec: Spec,
    ops: WindowOps,
    serial: u64,
    hwnd: isize,
    /// 建立那一幀的編號；還沒建立是 `None`。
    created: Option<u64>,
    shown: bool,
    /// 找到 HWND 且顯示後要搶前景。
    raise: bool,
    /// 下一幀要送還原：最小化的視窗光搶前景叫不回來。
    restore: bool,
}

impl HostedWindow {
    fn new(spec: Spec) -> Self {
        Self {
            spec,
            ops: WindowOps::real(),
            serial: 0,
            hwnd: 0,
            created: None,
            shown: false,
            raise: false,
            restore: false,
        }
    }

    fn id(&self) -> ViewportId {
        ViewportId::from_hash_of((self.spec.title_key, self.serial))
    }

    fn title(&self) -> String {
        t(self.spec.title_key, &[("app", &app_name())])
    }

    /// 視窗關了：狀態歸零，下一次開啟用新的 viewport。
    fn reset(&mut self) {
        *self = Self {
            ops: self.ops,
            serial: self.serial + 1,
            ..Self::new(self.spec)
        };
    }

    /// 已開著的視窗再叫到前景（最小化了就還原）；還沒顯示就等顯示那一幀。
    fn bring_to_front(&mut self) {
        self.raise = true;
        self.restore = true;
        if self.shown && self.hwnd != 0 {
            self.foreground();
        }
    }

    fn foreground(&mut self) {
        self.raise = false;
        if !(self.ops.force_foreground)(self.hwnd) {
            log(format!(
                "[ui] {} could not take the foreground (hwnd={:#x})",
                self.spec.name, self.hwnd
            ));
        }
    }

    /// 建立那一幀才給藏著、大小位置（游標所在螢幕的工作區置中）與置頂；之後只維持標題、最小大小等不變的部分。
    fn builder(&self, creating: bool, title: &str) -> ViewportBuilder {
        let builder = ViewportBuilder::default()
            .with_title(title)
            .with_decorations(true)
            .with_resizable(true)
            .with_taskbar(true)
            .with_min_inner_size(self.spec.min_size)
            .with_icon(app_icon());
        if !creating {
            return builder;
        }
        let ((x, y, w, h), scale) = (self.ops.cursor_work_area)();
        let area = (w as f32 / scale, h as f32 / scale);
        let (width, height) = initial_size((self.spec.size.x, self.spec.size.y), area);
        let left = x as f32 / scale + (area.0 - width) / 2.0;
        let top = y as f32 / scale + (area.1 - height) / 2.0;
        let builder = builder
            .with_visible(false)
            .with_inner_size([width, height])
            .with_position([left, top]);
        if self.spec.topmost_once {
            builder.with_always_on_top()
        } else {
            builder
        }
    }

    /// 每幀呼叫：畫出視窗（`content` 另收到這一幀有沒有關閉要求），找 HWND 時略過 `known`；回傳這一幀各 pass 的結果。
    ///
    /// 版面重排（`request_discard`）時 `content` 在同一幀跑不只一次，只有第一次收得到輸入，
    /// 所以每次回傳的動作都要收下，不能只留最後一次的。
    fn show<A>(
        &mut self,
        ctx: &egui::Context,
        known: &[isize],
        mut content: impl FnMut(&mut Ui, bool) -> Vec<A>,
    ) -> Vec<A> {
        // 以幀而不是 pass 計：同一幀內 discard 重跑的 pass 不算「下一幀」
        let frame = ctx.cumulative_frame_nr();
        let creating = self.created.is_none();
        let reveal = !self.shown && self.created.is_some_and(|at| frame > at);
        let title = self.title();
        let mut commands = Vec::new();
        if reveal {
            commands.push(ViewportCommand::Visible(true));
            if self.spec.topmost_once {
                commands.push(ViewportCommand::WindowLevel(WindowLevel::Normal));
            }
        }
        if std::mem::take(&mut self.restore) && self.shown {
            commands.push(ViewportCommand::Minimized(false));
        }
        let mut before = (self.ops.windows_before)(self.hwnd);
        let builder = self.builder(creating, &title);
        let mut out = Vec::new();
        let mut closing = false;
        let (id, name) = (self.id(), self.spec.name);
        let shown = viewport::show_immediate(ctx, id, name, builder, &mut before, |ui, _| {
            let close_requested = ui.input(|i| i.viewport().close_requested());
            closing |= close_requested;
            // 這一幀就要關掉的視窗不再顯示、也不再改它
            if !closing {
                for command in std::mem::take(&mut commands) {
                    ui.ctx().send_viewport_cmd(command);
                }
            }
            // 根視窗的 clear_color 是透明的，內容區要自己鋪滿底色
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, theme::WINDOW_BG);
            theme::apply(ui.style_mut());
            out.extend(content(ui, close_requested));
        });
        if shown.is_none() {
            return out;
        }
        self.created.get_or_insert(frame);
        if closing {
            return out;
        }
        self.shown |= reveal;
        if self.hwnd == 0 {
            self.hwnd = (self.ops.find_new)(&before, known, &title);
            if self.hwnd != 0 {
                (self.ops.dark_caption)(self.hwnd);
                log(format!(
                    "[ui] {} found: hwnd={:#x}",
                    self.spec.name, self.hwnd
                ));
                // 顯示之後才認出來（建立那一幀沒找到）：現在補搶前景
                self.raise |= self.shown;
            } else if creating {
                log(format!(
                    "[ui] {} hwnd lookup failed: no new window on the UI thread",
                    self.spec.name
                ));
            }
        }
        // 顯示與搶前景在同一幀：先顯示晚點才給焦點的話，這段空檔打的字會進別的視窗
        self.raise |= reveal;
        if self.raise && self.shown && self.hwnd != 0 {
            self.foreground();
        }
        if creating {
            ctx.request_repaint();
        }
        out
    }
}

/// 托管視窗裡的內容（設定視窗或首次設定精靈）。
pub trait HostedContent {
    /// 每一幀回給 App 的動作。
    type Action;
    /// 視窗固定的名稱、標題與大小。
    const SPEC: Spec;
    /// 以目前設定建立內容。
    fn create(cfg: &Config) -> Self;
    /// 背景工作的結果。
    fn apply(&mut self, event: &SettingsEvent);
    /// 畫一幀；`closing` 是這一幀有沒有關閉要求（✕、Alt+F4 當成取消）。
    fn run(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, closing: bool) -> Vec<Self::Action>;
}

impl HostedContent for SettingsWindow {
    type Action = SettingsAction;
    const SPEC: Spec = SETTINGS;

    fn create(cfg: &Config) -> Self {
        SettingsWindow::new(cfg)
    }

    fn apply(&mut self, event: &SettingsEvent) {
        SettingsWindow::apply(self, event);
    }

    fn run(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, closing: bool) -> Vec<SettingsAction> {
        let mut actions = self.ui(ui, env);
        let done = actions
            .iter()
            .any(|a| matches!(a, SettingsAction::Save(_) | SettingsAction::Cancel));
        if closing && !done {
            actions.extend(self.cancel(env));
        }
        actions
    }
}

impl HostedContent for Wizard {
    type Action = WizardAction;
    const SPEC: Spec = WIZARD;

    fn create(cfg: &Config) -> Self {
        Wizard::new(cfg.clone())
    }

    fn apply(&mut self, event: &SettingsEvent) {
        Wizard::apply(self, event);
    }

    fn run(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, closing: bool) -> Vec<WizardAction> {
        let action = self.ui(ui, env);
        let done = matches!(action, Some(WizardAction::Finish(_) | WizardAction::Cancel));
        if closing && !done {
            vec![self.cancel(env)]
        } else {
            action.into_iter().collect()
        }
    }
}

/// 內容與它的 viewport；所有方法都在 UI 執行緒呼叫。
pub struct Host<C: HostedContent> {
    env: Box<dyn SettingsEnv>,
    content: Option<C>,
    win: HostedWindow,
}

/// 設定視窗的托管。
pub type SettingsHost = Host<SettingsWindow>;
/// 首次設定精靈的托管。
pub type WizardHost = Host<Wizard>;

impl<C: HostedContent> Host<C> {
    /// `env` 跑測試連線、抓模型清單、檢查更新與錄熱鍵。
    pub fn new(env: Box<dyn SettingsEnv>) -> Self {
        Self {
            env,
            content: None,
            win: HostedWindow::new(C::SPEC),
        }
    }

    /// 換掉視窗操作（測試與 probe 用：不搶前景、假的工作區）。
    pub fn set_window_ops(&mut self, ops: WindowOps) {
        self.win.ops = ops;
    }

    /// 以 `cfg` 開窗；已開著就叫到前景。
    pub fn open(&mut self, cfg: &Config) {
        let name = C::SPEC.name;
        if self.content.is_some() {
            log(format!(
                "[ui] {name} already open; bringing it to the front"
            ));
            self.win.bring_to_front();
            return;
        }
        log(format!("[ui] {name} opened"));
        self.content = Some(C::create(cfg));
    }

    pub fn is_open(&self) -> bool {
        self.content.is_some()
    }

    /// 視窗的 HWND；沒開或還沒找到是 0。
    pub fn hwnd(&self) -> isize {
        self.win.hwnd
    }

    /// 背景工作的結果；沒開窗時丟掉。
    pub fn apply(&mut self, event: SettingsEvent) {
        if let Some(content) = &mut self.content {
            content.apply(&event);
        }
    }

    /// 關窗（完成或取消之後）。
    pub fn close(&mut self) {
        if self.content.take().is_some() {
            log(format!("[ui] {} closed", C::SPEC.name));
            self.win.reset();
        }
    }

    /// 每幀呼叫：開著就畫出視窗，回傳這一幀要 App 做的事。
    pub fn show(&mut self, ctx: &egui::Context, known: &[isize]) -> Vec<C::Action> {
        let Some(content) = &mut self.content else {
            return Vec::new();
        };
        let env = self.env.as_ref();
        self.win
            .show(ctx, known, |ui, closing| content.run(ui, env, closing))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use eframe::egui::{Context, RawInput, ViewportEvent, ViewportInfo};

    use super::*;

    thread_local! {
        static CLOSE_NEXT: Cell<bool> = const { Cell::new(false) };
    }

    fn offscreen_ops() -> WindowOps {
        WindowOps {
            windows_before: |_| Vec::new(),
            find_new: |_, _, _| 0,
            dark_caption: |_| {},
            force_foreground: |_| true,
            cursor_work_area: || ((0, 0, 1920, 1080), 1.0),
        }
    }

    /// 同 eframe：immediate viewport 在自己的 `run_ui` 裡跑，`request_discard` 只重跑這個 viewport。
    fn native_like_context() -> Context {
        let ctx = Context::default();
        ctx.set_embed_viewports(false);
        Context::set_immediate_viewport_renderer(|ctx, mut viewport| {
            let mut info = ViewportInfo::default();
            if CLOSE_NEXT.take() {
                info.events.push(ViewportEvent::Close);
            }
            let mut input = RawInput {
                viewport_id: viewport.ids.this,
                ..Default::default()
            };
            input.viewports.insert(viewport.ids.this, info);
            ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui))
                .textures_delta
                .clear();
        });
        ctx
    }

    /// 視窗建好之後跑一幀；`content` 第一個 pass 要求重排，回傳（pass 數，收到的動作，各 pass 收到的關閉要求）。
    fn relaid_frame(close: bool) -> (usize, Vec<&'static str>, Vec<bool>) {
        let ctx = native_like_context();
        let mut win = HostedWindow::new(SETTINGS);
        win.ops = offscreen_ops();
        let mut registered = ctx.run_ui(RawInput::default(), |ui| {
            assert!(win.show(ui.ctx(), &[], |_, _| vec![""]).is_empty());
        });
        assert!(registered.viewport_output.contains_key(&win.id()));
        registered.textures_delta.clear();
        let mut input = RawInput::default();
        input
            .viewports
            .insert(ViewportId::ROOT, ViewportInfo::default());
        let window = ViewportInfo {
            native_pixels_per_point: Some(1.0),
            ..Default::default()
        };
        input.viewports.insert(win.id(), window);
        CLOSE_NEXT.set(close);
        let mut closing = Vec::new();
        let mut got = Vec::new();
        let mut output = ctx.run_ui(input, |ui| {
            got = win.show(ui.ctx(), &[], |ui, close| {
                closing.push(close);
                if closing.len() == 1 {
                    // 換語言後版面變了：這一 pass 會被丟掉重跑，但它收到的點擊已經發生
                    ui.ctx().request_discard("test relayout");
                    vec!["picked in the first pass"]
                } else {
                    Vec::new()
                }
            });
        });
        output.textures_delta.clear();
        (closing.len(), got, closing)
    }

    #[test]
    fn actions_from_every_pass_of_a_discarded_frame_are_kept() {
        let (passes, got, closing) = relaid_frame(false);
        assert_eq!(passes, 2, "the frame was laid out twice");
        assert_eq!(got, ["picked in the first pass"]);
        assert_eq!(closing, [false, false]);
    }

    #[test]
    fn a_close_request_reaches_only_the_pass_that_received_it() {
        let (_, _, closing) = relaid_frame(true);
        assert_eq!(closing, [true, false]);
    }
}
