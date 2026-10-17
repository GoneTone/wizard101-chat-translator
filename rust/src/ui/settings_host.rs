//! 在 App 裡托管設定視窗與首次設定精靈：有系統標題列、可縮放、有工作列按鈕的一般 immediate viewport。
//!
//! 建立那一幀藏著、找到 HWND 就套深色標題列，下一幀顯示並在同一幀搶前景；設定視窗建立時置頂、顯示那一幀就放掉。

use eframe::egui::{self, Ui, Vec2, ViewportBuilder, ViewportCommand, ViewportId, WindowLevel};

use crate::config::{Config, app_name};
use crate::i18n::t;
use crate::logs::log;
use crate::ui::app::app_icon;
use crate::ui::geometry::Rect;
use crate::ui::settings::env::{SettingsEnv, SettingsEvent};
use crate::ui::settings::theme;
use crate::ui::settings::window::{SettingsAction, SettingsWindow};
use crate::ui::settings::wizard::{Wizard, WizardAction};
use crate::ui::win32;

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
    /// 目前執行緒的頂層視窗（建立 viewport 前取一次）。
    pub thread_windows: fn() -> Vec<isize>,
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
            thread_windows: win32::thread_windows,
            find_new: |before, known, title| {
                win32::pick_new_window(before, &win32::thread_windows(), known, || {
                    win32::find_thread_window(title, known)
                })
            },
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
struct Spec {
    /// log 用的名稱。
    name: &'static str,
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
    /// 下一幀要送新標題（介面語言換了）。
    retitle: bool,
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
            retitle: false,
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

    /// 已開著的視窗再叫到前景；還沒顯示就等顯示那一幀。
    fn bring_to_front(&mut self) {
        self.raise = true;
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

    /// 每幀呼叫：畫出視窗（`content` 另收到這一幀有沒有關閉要求），找 HWND 時略過 `known`。
    fn show<T>(
        &mut self,
        ctx: &egui::Context,
        known: &[isize],
        mut content: impl FnMut(&mut Ui, bool) -> T,
    ) -> T {
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
        if std::mem::take(&mut self.retitle) {
            commands.push(ViewportCommand::Title(title.clone()));
        }
        let before = if self.hwnd == 0 {
            (self.ops.thread_windows)()
        } else {
            Vec::new()
        };
        let builder = self.builder(creating, &title);
        let (out, closing) = ctx.show_viewport_immediate(self.id(), builder, |ui, _| {
            let closing = ui.input(|i| i.viewport().close_requested());
            // 這一幀就要關掉的視窗不再顯示、也不再改它
            if !closing {
                for command in &commands {
                    ui.ctx().send_viewport_cmd(command.clone());
                }
            }
            // 根視窗的 clear_color 是透明的，內容區要自己鋪滿底色
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, theme::WINDOW_BG);
            theme::apply(ui.style_mut());
            (content(ui, closing), closing)
        });
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

/// 設定視窗與它的 viewport；所有方法都在 UI 執行緒呼叫。
pub struct SettingsHost {
    env: Box<dyn SettingsEnv>,
    window: Option<SettingsWindow>,
    win: HostedWindow,
}

impl SettingsHost {
    /// `env` 跑測試連線、抓模型清單、檢查更新與錄熱鍵。
    pub fn new(env: Box<dyn SettingsEnv>) -> Self {
        Self {
            env,
            window: None,
            win: HostedWindow::new(SETTINGS),
        }
    }

    /// 換掉視窗操作（測試與 probe 用：不搶前景、假的工作區）。
    pub fn set_window_ops(&mut self, ops: WindowOps) {
        self.win.ops = ops;
    }

    /// 以 `cfg` 開窗；已開著就叫到前景。
    pub fn open(&mut self, cfg: &Config) {
        if self.window.is_some() {
            log("[ui] settings window already open; bringing it to the front");
            self.win.bring_to_front();
            return;
        }
        log("[ui] settings window opened");
        self.window = Some(SettingsWindow::new(cfg));
    }

    pub fn is_open(&self) -> bool {
        self.window.is_some()
    }

    /// 設定視窗的 HWND；沒開或還沒找到是 0。
    pub fn hwnd(&self) -> isize {
        self.win.hwnd
    }

    /// 背景工作的結果；沒開窗時丟掉。
    pub fn apply(&mut self, event: SettingsEvent) {
        if let Some(window) = &mut self.window {
            window.apply(&event);
        }
    }

    /// 介面語言換了：下一幀換視窗標題。
    pub fn retitle(&mut self) {
        self.win.retitle = true;
    }

    /// 關窗（儲存或取消之後）。
    pub fn close(&mut self) {
        if self.window.take().is_some() {
            log("[ui] settings window closed");
            self.win.reset();
        }
    }

    /// 每幀呼叫：開著就畫出視窗，回傳這一幀要 App 做的事；✕、Alt+F4 當成取消。
    pub fn show(&mut self, ctx: &egui::Context, known: &[isize]) -> Vec<SettingsAction> {
        let Some(window) = &mut self.window else {
            return Vec::new();
        };
        let env = self.env.as_ref();
        self.win.show(ctx, known, |ui, closing| {
            let mut actions = window.ui(ui, env);
            let done = actions
                .iter()
                .any(|a| matches!(a, SettingsAction::Save(_) | SettingsAction::Cancel));
            if closing && !done {
                actions.extend(window.cancel(env));
            }
            actions
        })
    }
}

/// 首次設定精靈與它的 viewport；所有方法都在 UI 執行緒呼叫。
pub struct WizardHost {
    env: Box<dyn SettingsEnv>,
    wizard: Option<Wizard>,
    win: HostedWindow,
}

impl WizardHost {
    /// `env` 跑測試連線、抓模型清單與錄熱鍵。
    pub fn new(env: Box<dyn SettingsEnv>) -> Self {
        Self {
            env,
            wizard: None,
            win: HostedWindow::new(WIZARD),
        }
    }

    /// 換掉視窗操作（測試與 probe 用：不搶前景、假的工作區）。
    pub fn set_window_ops(&mut self, ops: WindowOps) {
        self.win.ops = ops;
    }

    /// 以 `cfg` 開精靈；已開著就叫到前景。
    pub fn open(&mut self, cfg: &Config) {
        if self.wizard.is_some() {
            log("[ui] wizard already open; bringing it to the front");
            self.win.bring_to_front();
            return;
        }
        log("[ui] wizard opened");
        self.wizard = Some(Wizard::new(cfg.clone()));
    }

    pub fn is_open(&self) -> bool {
        self.wizard.is_some()
    }

    /// 精靈的 HWND；沒開或還沒找到是 0。
    pub fn hwnd(&self) -> isize {
        self.win.hwnd
    }

    /// 背景工作的結果；沒開時丟掉。
    pub fn apply(&mut self, event: SettingsEvent) {
        if let Some(wizard) = &mut self.wizard {
            wizard.apply(&event);
        }
    }

    /// 介面語言換了：下一幀換視窗標題。
    pub fn retitle(&mut self) {
        self.win.retitle = true;
    }

    /// 關掉精靈（完成或取消之後）。
    pub fn close(&mut self) {
        if self.wizard.take().is_some() {
            log("[ui] wizard closed");
            self.win.reset();
        }
    }

    /// 每幀呼叫：開著就畫出精靈，回傳這一幀要 App 做的事；✕、Alt+F4 當成取消。
    pub fn show(&mut self, ctx: &egui::Context, known: &[isize]) -> Option<WizardAction> {
        let wizard = self.wizard.as_mut()?;
        let env = self.env.as_ref();
        self.win.show(ctx, known, |ui, closing| {
            let action = wizard.ui(ui, env);
            let done = matches!(action, Some(WizardAction::Finish(_) | WizardAction::Cancel));
            if closing && !done {
                Some(wizard.cancel(env))
            } else {
                action
            }
        })
    }
}
