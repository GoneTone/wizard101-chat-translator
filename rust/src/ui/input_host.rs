//! 在 App 裡托管翻譯輸入框：immediate viewport 的建立、找 HWND、擺放、顯示與前景交接，並分派輸入框相關的 UI 事件。

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, CursorIcon, FontId, Label, Layout, RichText, Sense, UiBuilder, Vec2,
    ViewportBuilder, ViewportCommand, ViewportId,
};

use crate::config::py::truthy;
use crate::config::{Config, app_name};
use crate::input::dispatch::{GameInputEvents, InputBoxControl};
use crate::logs::log;
use crate::ui::events::UiEvent;
use crate::ui::fonts_egui::tk_pt;
use crate::ui::geometry::Rect;
use crate::ui::input_box::{BG, InputBox, open_rect, refit};
use crate::ui::overlay::frame::BAR_HEIGHT;
use crate::ui::palette::{BAR, FG_BAR};
use crate::ui::win32;
use crate::util::read;

/// 等熱鍵觸發鍵放開的上限；漏收放開時不讓輸入框永遠不出現。
const RELEASE_WAIT: Duration = Duration::from_secs(1);
/// 等放開期間重繪的間隔。
pub(crate) const RELEASE_POLL: Duration = Duration::from_millis(15);

/// 建立那一幀的暫定高度（DIP）；量到內容高度後隨即貼合。
const INITIAL_HEIGHT: f32 = 84.0;

/// 輸入框視窗對桌面的操作；測試與 probe 換成不搶前景、擺在螢幕外的版本。
#[derive(Clone, Copy)]
pub struct Desktop {
    /// 把視窗切到前景。
    pub force_foreground: fn(isize) -> bool,
    /// 含 (x, y) 那顆螢幕的工作區。
    pub work_area_at: fn(i32, i32) -> Rect,
    /// 熱鍵吞掉的觸發鍵是否還按著（見 [`crate::input::hotkeys::swallowed_key_held`]）。
    pub trigger_held: fn() -> bool,
}

impl Desktop {
    /// 接 Win32 的真實操作。
    pub fn real() -> Self {
        Self {
            force_foreground: win32::force_foreground,
            work_area_at: win32::work_area_at,
            trigger_held: crate::input::hotkeys::swallowed_key_held,
        }
    }
}

/// 輸入框與它的視窗；所有方法都在 UI 執行緒呼叫。
pub struct InputHost {
    pub(crate) input: InputBox,
    events: GameInputEvents,
    settings: Arc<RwLock<Config>>,
    pub(crate) desktop: Desktop,
    /// 這次開窗的 HWND；還沒找到是 0。
    pub(crate) hwnd: isize,
    created: bool,
    shown: bool,
    raise: bool,
    above: bool,
    /// 上次擺放或貼合時的內容高度（DIP）。
    content_dip: f32,
    /// 開始等觸發鍵放開的時間；沒在等是 `None`。
    release_wait: Option<Instant>,
}

impl InputHost {
    /// `settings` 用來在每次遊戲聊天框事件時讀 `auto_show_input`。
    pub fn new(input: InputBox, events: GameInputEvents, settings: Arc<RwLock<Config>>) -> Self {
        Self {
            input,
            events,
            settings,
            desktop: Desktop::real(),
            hwnd: 0,
            created: false,
            shown: false,
            raise: false,
            above: false,
            content_dip: 0.0,
            release_wait: None,
        }
    }

    /// 輸入框視窗的 HWND；沒開或還沒找到是 0。
    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    /// 套用輸入框相關的 UI 事件；其他事件不理。
    pub fn apply(&mut self, event: UiEvent) {
        match event {
            UiEvent::ShowInputBox { retarget } => {
                if let Some(hwnd) = retarget {
                    self.input.retarget(hwnd);
                }
                self.input.show();
            }
            UiEvent::GameInputOpened { hwnd, anchor } => {
                let auto_show = self.auto_show();
                self.events.opened(hwnd, anchor, auto_show, &mut self.input);
            }
            UiEvent::GameInputClosed { hwnd } => {
                let auto_show = self.auto_show();
                self.events.closed(hwnd, auto_show, &mut self.input);
            }
            UiEvent::OutgoingDone { session, result } => self.input.finish(session, result),
            _ => {}
        }
    }

    fn auto_show(&self) -> bool {
        truthy(read(&self.settings).get("auto_show_input"))
    }

    /// 每幀呼叫：開著就畫出視窗，關了就把前景交還；找 HWND 時略過 `known`（其他 viewport，0＝沒有），輸入框由 overlay 擁有。
    ///
    /// 建立那一幀先藏著量內容高度、以實體像素擺好，下一幀才顯示並搶前景。
    pub fn show(&mut self, ctx: &egui::Context, overlay: isize, known: &[isize]) {
        if !self.input.is_open() {
            // shortcut: 同一輪 logic 先關後開時沿用原視窗與位置（Python 會重建、重新貼齊）；實機遇到再改成關閉就換新的 viewport id
            (self.hwnd, self.created, self.shown, self.raise) = (0, false, false, false);
            self.release_wait = None;
            if let Some(target) = self.input.take_restore_foreground() {
                (self.desktop.force_foreground)(target);
            }
            return;
        }
        self.raise |= self.input.take_focus_request();
        let pending = (self.created && !self.shown) || self.raise;
        let defer = defer_for_release(
            pending,
            (self.desktop.trigger_held)(),
            &mut self.release_wait,
            Instant::now(),
        );
        if defer {
            ctx.request_repaint_after(RELEASE_POLL);
        }
        let creating = !self.created;
        let reveal = self.created && !self.shown && !defer;
        let mut builder = ViewportBuilder::default()
            .with_title(app_name())
            .with_decorations(false)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false);
        if creating {
            // 概略的位置與寬度（寬度決定提示換行、也就決定內容高度），找到 HWND 後再以實體像素擺正
            let ((x, y, w, h), _, scale) = self.open_rect(INITIAL_HEIGHT);
            let dip = |v: i32| v as f32 / scale;
            builder = builder
                .with_visible(false)
                .with_position([dip(x), dip(y)])
                .with_inner_size([dip(w), dip(h)]);
        }
        let before = if self.hwnd == 0 {
            win32::thread_windows()
        } else {
            Vec::new()
        };
        let input = &mut self.input;
        let (height, ppp) =
            ctx.show_viewport_immediate(ViewportId::from_hash_of("input_box"), builder, |ui, _| {
                if reveal {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Visible(true));
                }
                (content(ui, input), ui.ctx().pixels_per_point())
            });
        self.created = true;
        self.shown |= reveal;
        if !self.input.is_open() {
            // 前景下一幀才交還
            ctx.request_repaint();
            return;
        }
        if self.hwnd == 0 {
            self.hwnd = win32::pick_new_window(&before, &win32::thread_windows(), known, || {
                win32::find_thread_window(&app_name(), known)
            });
            if self.hwnd != 0 {
                self.place(height, overlay);
            } else if creating {
                log("[ui] input box hwnd lookup failed: no new window on the UI thread");
            }
        } else if height != self.content_dip {
            let content_h = (height * ppp).round() as i32;
            let rect = refit(win32::window_rect(self.hwnd), content_h, self.above);
            win32::set_window_rect(self.hwnd, rect);
            self.content_dip = height;
        }
        if creating {
            ctx.request_repaint();
        }
        if self.raise && self.shown && self.hwnd != 0 && !defer {
            if !(self.desktop.force_foreground)(self.hwnd) {
                log(format!(
                    "[input] box could not take the foreground (hwnd={:#x})",
                    self.hwnd
                ));
            }
            self.raise = false;
        }
    }

    /// 拿掉系統外框與工作列按鈕，依內容高度（DIP）貼齊錨點或游標。
    fn place(&mut self, content_dip: f32, overlay: isize) {
        win32::remove_system_frame(self.hwnd);
        win32::hide_from_taskbar(self.hwnd, overlay);
        let (rect, above, _) = self.open_rect(content_dip);
        win32::set_window_rect(self.hwnd, rect);
        (self.above, self.content_dip) = (above, content_dip);
        log(format!(
            "[ui] input box placed: hwnd={:#x} rect={rect:?} above_anchor={above}",
            self.hwnd
        ));
    }

    /// 開窗外框（實體像素）、是否在錨點上方與換算用的縮放：以錨點或游標所在螢幕為準，不是視窗剛好落在哪顆螢幕。
    fn open_rect(&self, content_dip: f32) -> (Rect, bool, f32) {
        let (anchor, cursor) = self.input.placement_anchor();
        let (x, y) = anchor.map_or(cursor, |(x, y, _, _)| (x, y));
        let scale = win32::scale_at(x, y);
        let content_h = (content_dip * scale).round() as i32;
        let area = (self.desktop.work_area_at)(x, y);
        let (rect, above) = open_rect(anchor, cursor, content_h, scale, area);
        (rect, above, scale)
    }
}

/// 視窗內容：底色、拖曳條與輸入框本體；回傳內容總高（DIP）。
fn content(ui: &mut egui::Ui, input: &mut InputBox) -> f32 {
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, BG);
    let bar = egui::Rect::from_min_size(full.min, Vec2::new(full.width(), BAR_HEIGHT));
    if drag_strip(ui, bar) || ui.input(|i| i.viewport().close_requested()) {
        input.close();
        return BAR_HEIGHT;
    }
    let mut body = ui.new_child(UiBuilder::new().max_rect(full.with_min_y(bar.max.y)));
    input.content(&mut body);
    BAR_HEIGHT + body.min_rect().height()
}

/// 頂端的拖曳條（名稱與 ✕）：按住交給系統移動視窗；回傳 ✕ 是否被按下。
fn drag_strip(ui: &mut egui::Ui, bar: egui::Rect) -> bool {
    ui.painter().rect_filled(bar, 0.0, BAR);
    let response = ui.interact(bar, ui.scope_id().with("input_box_drag"), Sense::drag());
    if response.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    let layout = Layout::right_to_left(Align::Center);
    ui.scope_builder(UiBuilder::new().max_rect(bar).layout(layout), |ui| {
        ui.add_space(6.0);
        let close = RichText::new("✕")
            .font(FontId::proportional(tk_pt(9.0)))
            .color(FG_BAR);
        let close = ui
            .add(Label::new(close).selectable(false).sense(Sense::click()))
            .on_hover_cursor(CursorIcon::PointingHand)
            .clicked();
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.add_space(6.0);
            let title = RichText::new(app_name())
                .font(FontId::proportional(tk_pt(8.0)))
                .color(FG_BAR);
            ui.add(Label::new(title).selectable(false).truncate());
        });
        close
    })
    .inner
}

/// 要顯示或搶前景（`pending`）時，熱鍵吞掉的觸發鍵還按著就先等，最多等 [`RELEASE_WAIT`]；回 true 表示這幀先別動。
pub(crate) fn defer_for_release(
    pending: bool,
    trigger_held: bool,
    since: &mut Option<Instant>,
    now: Instant,
) -> bool {
    if !pending || !trigger_held {
        *since = None;
        return false;
    }
    now.duration_since(*since.get_or_insert(now)) < RELEASE_WAIT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn showing_waits_for_the_trigger_key_release_up_to_a_second() {
        let t0 = Instant::now();
        let mut since = None;
        assert!(defer_for_release(true, true, &mut since, t0));
        assert!(defer_for_release(
            true,
            true,
            &mut since,
            t0 + Duration::from_millis(900)
        ));
        assert!(
            !defer_for_release(true, true, &mut since, t0 + RELEASE_WAIT),
            "a missed release never hides the box for good"
        );
        assert!(!defer_for_release(
            true,
            false,
            &mut since,
            t0 + RELEASE_WAIT
        ));
        assert_eq!(since, None, "released: the next wait starts afresh");
        assert!(
            !defer_for_release(false, true, &mut since, t0),
            "nothing to show"
        );
    }
}
