//! 在 App 裡托管框選翻譯的三個視窗：選取畫面、框選框與結果卡的 immediate viewport，使用者操作回報給 `RegionFlow`。
//!
//! 三個視窗都在建立那一幀藏著、找到 HWND 後以實體像素擺好，下一幀才顯示；選取畫面另外等熱鍵觸發鍵放開、HWND 找到才顯示並搶前景。

use std::time::Instant;

use eframe::egui::{
    self, Event, PointerButton, Pos2, ViewportBuilder, ViewportCommand, ViewportId,
};

use crate::config::app_name;
use crate::logs::log;
use crate::region::pipeline::RegionResult;
use crate::ui::geometry::Rect;
use crate::ui::input_host::{Desktop, RELEASE_POLL, defer_for_release};
use crate::ui::region::card::{CardAction, MIN_HEIGHT, MIN_WIDTH};
use crate::ui::region::flow::RegionFlow;
use crate::ui::region::region_box::{self, RegionBox, hit_at};
use crate::ui::viewport::{self, show_immediate};
use crate::ui::win32;

/// 一個框選視窗的 HWND 與顯示狀態；宿主看到元件關掉就換下一個 viewport id，同一幀先關後開也不沿用舊視窗。
///
/// 流程在同一次呼叫裡換掉的卡片（宿主看不到中間的關閉）沿用原視窗：每幀重新擺放、重設剪貼簿。
#[derive(Default)]
pub(crate) struct HostedWindow {
    serial: u64,
    /// 這個視窗的 HWND；還沒找到是 0。
    pub(crate) hwnd: isize,
    created: bool,
    pub(crate) shown: bool,
    /// 上次以 `set_window_rect` 擺的外框（實體像素）。
    placed: Option<Rect>,
}

impl HostedWindow {
    fn id(&self, name: &str) -> ViewportId {
        ViewportId::from_hash_of((name, self.serial))
    }

    /// 元件關掉了：狀態歸零，下一次開啟用新的 viewport。
    fn close(&mut self) {
        if self.created {
            *self = Self {
                serial: self.serial + 1,
                ..Self::default()
            };
        }
    }

    /// 擺到 `rect`（實體像素）；與上次相同時不動，回傳是否搬了。
    fn place(&mut self, rect: Rect) -> bool {
        if self.hwnd == 0 || self.placed == Some(rect) {
            return false;
        }
        win32::set_window_rect(self.hwnd, rect);
        self.placed = Some(rect);
        true
    }
}

/// 框選流程與它的三個視窗；所有方法都在 UI 執行緒呼叫。
pub struct RegionHost {
    pub(crate) flow: RegionFlow,
    pub(crate) desktop: Desktop,
    pub(crate) selector: HostedWindow,
    pub(crate) region_box: HostedWindow,
    pub(crate) card: HostedWindow,
    /// 選取畫面開始等觸發鍵放開或 HWND 的時間；沒在等是 `None`。
    release_wait: Option<Instant>,
    alpha: Option<f32>,
}

impl RegionHost {
    pub fn new(flow: RegionFlow) -> Self {
        Self {
            flow,
            desktop: Desktop::real(),
            selector: HostedWindow::default(),
            region_box: HostedWindow::default(),
            card: HostedWindow::default(),
            release_wait: None,
            alpha: None,
        }
    }

    /// 框選熱鍵（`UiEvent::RegionHotkey`）。
    pub fn toggle(&mut self, hwnd: isize) {
        self.flow.toggle(hwnd);
        self.sync_closed();
    }

    /// 標題列 ⛶。
    pub fn start_from_button(&mut self) {
        self.flow.start_from_button();
        self.sync_closed();
    }

    /// 辨識翻譯結果（`UiEvent::RegionDone`）。
    pub fn finish(&mut self, session: u64, result: Result<RegionResult, String>) {
        self.flow.finish(session, result);
    }

    /// 卡片背景不透明度跟著 overlay；沒變時不動。
    pub fn set_alpha(&mut self, alpha: f32) {
        if self.alpha != Some(alpha) {
            self.alpha = Some(alpha);
            self.flow.set_alpha(alpha);
        }
    }

    /// 選取畫面、框與卡片的 HWND（0＝沒開或還沒找到）。
    pub fn hwnds(&self) -> [isize; 3] {
        [self.selector.hwnd, self.region_box.hwnd, self.card.hwnd]
    }

    /// 找新視窗的 HWND 時要略過的已知視窗：`others`（overlay 等）加上框選自己的視窗。
    pub(crate) fn known(&self, others: &[isize]) -> Vec<isize> {
        others
            .iter()
            .copied()
            .chain(self.hwnds())
            .filter(|&h| h != 0)
            .collect()
    }

    /// 流程已收掉的元件，視窗狀態跟著歸零。
    fn sync_closed(&mut self) {
        if self.flow.selector_mut().is_none() {
            self.selector.close();
            self.release_wait = None;
        }
        if self.flow.region_box_mut().is_none() {
            self.region_box.close();
        }
        if self.flow.card_mut().is_none() {
            self.card.close();
        }
    }

    /// 每幀呼叫：畫出開著的視窗；`overlay` 擁有它們，`others` 是其他 viewport 的 HWND。
    pub fn show(&mut self, ctx: &egui::Context, overlay: isize, others: &[isize]) {
        self.sync_closed();
        self.show_selector(ctx, overlay, others);
        self.show_box(ctx, overlay, others);
        self.show_card(ctx, overlay, others);
        self.sync_closed();
    }

    fn show_selector(&mut self, ctx: &egui::Context, overlay: isize, others: &[isize]) {
        let known = self.known(others);
        let Some(selector) = self.flow.selector_mut() else {
            return;
        };
        let monitor = selector.monitor();
        let win = &mut self.selector;
        let pending = win.created && !win.shown;
        // 沒有 HWND 就搶不了前景：與等觸發鍵放開共用 1 秒上限，逾時照樣顯示
        let defer = defer_for_release(
            pending,
            (self.desktop.trigger_held)() || win.hwnd == 0,
            &mut self.release_wait,
            Instant::now(),
        );
        if defer {
            ctx.request_repaint_after(RELEASE_POLL);
        }
        let creating = !win.created;
        let reveal = pending && !defer;
        let builder = viewport_builder(creating, monitor);
        let mut before = viewport::windows_before(win.hwnd);
        let id = win.id("region_selector");
        let Some(action) = show_immediate(ctx, id, "selector", builder, &mut before, |ui, _| {
            if reveal {
                ui.ctx().send_viewport_cmd(ViewportCommand::Visible(true));
            }
            selector.content(ui)
        }) else {
            return;
        };
        win.created = true;
        win.shown |= reveal;
        if win.hwnd == 0 {
            win.hwnd = find(&before, &known, creating, "selector");
            if win.hwnd != 0 {
                viewport::frameless_setup(win.hwnd, overlay);
                win.place(monitor);
                log(format!("[region] selector placed: monitor={monitor:?}"));
            }
        }
        if creating {
            ctx.request_repaint();
        }
        // 顯示與搶前景在同一幀：先顯示晚點才給焦點的話，這段空檔打的字會進遊戲
        if reveal && win.hwnd == 0 {
            log("[region] selector window not found; shown without focus");
        } else if reveal && !(self.desktop.force_foreground)(win.hwnd) {
            log(format!(
                "[region] selector focus failed: SetForegroundWindow refused (hwnd={:#x})",
                win.hwnd
            ));
        }
        if let Some(action) = action {
            self.flow.on_selector(action);
            // 不等下一次輸入：全螢幕置頂的選取畫面要在下一幀就收掉
            ctx.request_repaint();
        }
    }

    fn show_box(&mut self, ctx: &egui::Context, overlay: isize, others: &[isize]) {
        let known = self.known(others);
        let Some(region_box) = self.flow.region_box_mut() else {
            return;
        };
        let win = &mut self.region_box;
        let rect = region_box::window_rect(region_box.rect());
        let creating = !win.created;
        let reveal = win.created && !win.shown;
        let builder = viewport_builder(creating, rect)
            .with_transparent(true)
            .with_active(false);
        let mut before = viewport::windows_before(win.hwnd);
        let origin = win.placed.unwrap_or(rect);
        let id = win.id("region_box");
        let Some(changed) = show_immediate(ctx, id, "box", builder, &mut before, |ui, _| {
            if reveal {
                ui.ctx().send_viewport_cmd(ViewportCommand::Visible(true));
            }
            box_content(ui, region_box, (origin.0, origin.1))
        }) else {
            return;
        };
        win.created = true;
        win.shown |= reveal;
        if win.hwnd == 0 {
            win.hwnd = find(&before, &known, creating, "box");
            if win.hwnd != 0 {
                viewport::frameless_setup(win.hwnd, overlay);
            }
        }
        if win.hwnd != 0 {
            // winit 每次顯示都重寫 extended style 洗掉 NOACTIVATE
            win32::make_non_activating(win.hwnd);
        }
        // 拖曳中、誤觸還原與重新框選都一樣：視窗跟著框的矩形走
        win.place(region_box::window_rect(region_box.rect()));
        if creating || reveal {
            ctx.request_repaint();
        }
        if let Some(rect) = changed {
            self.flow.on_box_changed(rect);
        }
    }

    fn show_card(&mut self, ctx: &egui::Context, overlay: isize, others: &[isize]) {
        let known = self.known(others);
        let Some(card) = self.flow.card_mut() else {
            return;
        };
        let win = &mut self.card;
        let rect = card.rect();
        let area = (self.desktop.work_area_at)(rect.0, rect.1);
        let creating = !win.created;
        let reveal = win.created && !win.shown;
        let rough = (rect.0, rect.1 + rect.3, MIN_WIDTH, MIN_HEIGHT);
        let builder = viewport_builder(creating, rough)
            .with_transparent(true)
            .with_active(false);
        let mut before = viewport::windows_before(win.hwnd);
        let id = win.id("region_card");
        let Some((placed, action)) =
            show_immediate(ctx, id, "card", builder, &mut before, |ui, _| {
                if reveal {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Visible(true));
                }
                (card.layout(ui.ctx(), area), card.content(ui))
            })
        else {
            return;
        };
        win.created = true;
        win.shown |= reveal;
        if win.hwnd == 0 {
            win.hwnd = find(&before, &known, creating, "card");
            if win.hwnd != 0 {
                viewport::frameless_setup(win.hwnd, overlay);
            }
        }
        if win.hwnd != 0 {
            win32::make_non_activating(win.hwnd);
        }
        let moved = win.place((placed.x, placed.y, placed.w, placed.h));
        if creating || reveal || moved {
            ctx.request_repaint();
        }
        match action {
            Some(CardAction::FocusForCopy) => self.focus_card(),
            Some(action) => {
                self.flow.on_card(action);
                if action == CardAction::Close {
                    ctx.request_repaint();
                }
            }
            None => {}
        }
    }

    /// 拖曳選字結束：卡片不奪焦點，這裡是唯一例外，Ctrl+C 才收得到。
    fn focus_card(&mut self) {
        if self.card.hwnd == 0 {
            log("[region] selection focus skipped: card window not found");
            return;
        }
        log("[region] forcing keyboard focus");
        if (self.desktop.force_foreground)(self.card.hwnd) {
            log("[region] selection took keyboard focus");
        } else {
            log("[region] selection focus failed: SetForegroundWindow refused");
        }
    }
}

/// 建立那一幀藏著、概略擺在 `rough`（實體像素）的無邊框 viewport 設定。
fn viewport_builder(creating: bool, rough: Rect) -> ViewportBuilder {
    viewport::borderless_builder(creating.then(|| (rough, win32::scale_at(rough.0, rough.1))))
}

/// 認出新 viewport 的 HWND；找到時、或建立那一幀就找不到時記 log。
fn find(before: &[isize], known: &[isize], creating: bool, what: &str) -> isize {
    let hwnd = viewport::find_new_window(before, known, &app_name());
    if hwnd != 0 {
        log(format!("[region] {what} window found: hwnd={hwnd:#x}"));
    } else if creating {
        log(format!(
            "[region] {what} hwnd lookup failed: no new window on the UI thread"
        ));
    }
    hwnd
}

/// 框選框的內容：依序把按下、移動、放開餵給框（螢幕 px），畫框線；回傳放開後的新矩形。
///
/// `origin` 是視窗左上角的螢幕座標：事件位置是相對視窗的 DIP，換成 px 再加上它。
fn box_content(ui: &mut egui::Ui, region_box: &mut RegionBox, origin: (i32, i32)) -> Option<Rect> {
    let ppp = ui.ctx().pixels_per_point();
    let top_left = ui.max_rect().min;
    let local = |p: Pos2| {
        let v = (p - top_left) * ppp;
        (v.x.round() as i32, v.y.round() as i32)
    };
    let screen = |p: Pos2| {
        let (x, y) = local(p);
        (origin.0 + x, origin.1 + y)
    };
    // 依事件順序處理：eframe 會把兩次重繪間的按下、移動、放開併成一幀
    let (events, hover) = ui.input(|i| (i.events.clone(), i.pointer.latest_pos()));
    let mut changed = None;
    for event in events {
        match event {
            Event::PointerMoved(p) => {
                region_box.drag(screen(p));
            }
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                ..
            } => {
                if pressed {
                    region_box.press(screen(pos), local(pos));
                } else if let Some(rect) = region_box.release(screen(pos)) {
                    changed = Some(rect);
                }
            }
            _ => {}
        }
    }
    let (_, _, w, h) = region_box.rect();
    let hit = region_box
        .dragging()
        .or_else(|| hover.map(local).and_then(|(x, y)| hit_at(x, y, w, h)));
    if let Some(hit) = hit {
        ui.ctx().set_cursor_icon(region_box::cursor(hit));
    }
    region_box.paint(ui);
    changed
}

#[cfg(test)]
pub(crate) mod fakes {
    //! 框選流程的替身：桌面操作與 runner 只記下呼叫，不碰真的視窗、截圖或網路。

    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use image::RgbImage;
    use tokio_util::sync::CancellationToken;

    use crate::region::capture::{CaptureError, Frame};
    use crate::ui::geometry::Rect;
    use crate::ui::region::flow::{RegionDesktop, RegionFlow, RegionRunner};

    /// 熱鍵帶來的遊戲視窗。
    pub(crate) const GAME: isize = 0x1234;
    /// ⛶ 列舉視窗找到的遊戲視窗。
    pub(crate) const FOUND: isize = 0x9999;
    /// 落在凍結畫面內的框選矩形。
    pub(crate) const RECT: Rect = (100, 100, 300, 120);

    /// 依序記下的呼叫：`capture 0x..`、`foreground 0x..`、`run N`。
    pub(crate) type Calls = Rc<RefCell<Vec<String>>>;

    struct FakeDesktop(Calls);

    impl RegionDesktop for FakeDesktop {
        fn capture_window(&mut self, hwnd: isize) -> Result<Arc<Frame>, CaptureError> {
            self.0.borrow_mut().push(format!("capture {hwnd:#x}"));
            Ok(Arc::new(Frame {
                image: RgbImage::new(800, 600),
                client_origin: (0, 0),
                client_size: (800, 600),
            }))
        }

        fn capture_screen(&mut self, _monitor: Rect) -> RgbImage {
            RgbImage::new(1, 1)
        }

        fn monitor_at(&self, _x: i32, _y: i32) -> Rect {
            (0, 0, 800, 600)
        }

        fn foreground(&mut self, hwnd: isize) {
            self.0.borrow_mut().push(format!("foreground {hwnd:#x}"));
        }

        fn find_game(&self) -> Option<isize> {
            Some(FOUND)
        }

        fn cursor(&self) -> (i32, i32) {
            (40, 50)
        }
    }

    struct FakeRunner(Calls);

    impl RegionRunner for FakeRunner {
        fn run(&self, _png: Vec<u8>, _rect: Rect, session: u64, _cancel: CancellationToken) {
            self.0.borrow_mut().push(format!("run {session}"));
        }
    }

    /// 接上替身的流程與它的呼叫紀錄。
    pub(crate) fn flow() -> (RegionFlow, Calls) {
        let calls = Calls::default();
        let flow = RegionFlow::new(
            Box::new(FakeDesktop(Rc::clone(&calls))),
            Box::new(FakeRunner(Rc::clone(&calls))),
            Arc::new(AtomicBool::new(false)),
        );
        (flow, calls)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::time::Duration;

    use egui_kittest::Harness;

    use super::fakes::{GAME, RECT, flow};
    use super::*;
    use crate::ui::fonts_egui;
    use crate::ui::region::selector::SelectorAction;

    thread_local! {
        static HELD: Cell<bool> = const { Cell::new(false) };
        static RAISED: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
        /// 每一幀開始時記下的上一幀重繪原因（呼叫端檔名）。
        static CAUSES: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    }

    /// 只托管框選的 harness：搶前景只記下次數，觸發鍵狀態由 `HELD` 決定。
    fn harness() -> Harness<'static, RegionHost> {
        HELD.with(|h| h.set(false));
        RAISED.with(|r| r.borrow_mut().clear());
        let mut host = RegionHost::new(flow().0);
        host.desktop = Desktop {
            force_foreground: |hwnd| {
                RAISED.with(|r| r.borrow_mut().push(hwnd));
                true
            },
            work_area_at: |_, _| (0, 0, 1920, 1040),
            trigger_held: || HELD.with(Cell::get),
        };
        Harness::builder()
            .with_context_setup(|ctx| fonts_egui::install(ctx, "en-US"))
            .build_ui_state(
                |ui, host: &mut RegionHost| {
                    let causes = ui.ctx().repaint_causes();
                    CAUSES.with(|c| c.borrow_mut().extend(causes.iter().map(|c| c.file)));
                    host.show(&ui.ctx().clone(), 0, &[]);
                },
                host,
            )
    }

    /// 只交給注入的 `force_foreground` 的假 HWND。
    const FAKE_HWND: isize = 0x5E1EC7;

    fn raised() -> usize {
        RAISED.with(|r| r.borrow().len())
    }

    #[test]
    fn selector_takes_foreground_only_after_the_trigger_is_released() {
        let mut h = harness();
        HELD.with(|held| held.set(true));
        h.state_mut().toggle(GAME);
        h.step();
        // kittest 找不到真的 HWND；換成假的，搶前景只交給注入的替身
        h.state_mut().selector.hwnd = FAKE_HWND;
        for _ in 0..2 {
            h.step();
        }
        assert!(h.state().selector.created && !h.state().selector.shown);
        assert_eq!(raised(), 0, "no foreground while the trigger key is held");
        HELD.with(|held| held.set(false));
        h.step();
        assert!(h.state().selector.shown);
        assert_eq!(raised(), 1, "shown and focused in the same frame");
        h.step();
        assert_eq!(raised(), 1);
    }

    #[test]
    fn selector_waits_for_its_window_before_showing() {
        let mut h = harness();
        h.state_mut().toggle(GAME);
        h.run_steps(3);
        assert!(h.state().selector.created && !h.state().selector.shown);
        assert_eq!(raised(), 0);
        h.state_mut().selector.hwnd = FAKE_HWND;
        h.step();
        assert!(h.state().selector.shown);
        assert_eq!(RAISED.with(|r| r.borrow().clone()), [FAKE_HWND]);
    }

    #[test]
    fn selector_without_a_window_is_shown_unfocused_after_the_wait() {
        let mut h = harness();
        h.state_mut().toggle(GAME);
        h.run_steps(2);
        assert!(!h.state().selector.shown);
        h.state_mut().release_wait = Some(Instant::now() - Duration::from_secs(2));
        let text = crate::logs::capture(|| h.step());
        assert!(h.state().selector.shown);
        assert_eq!(raised(), 0);
        assert!(
            text.contains("[region] selector window not found; shown without focus"),
            "{text}"
        );
    }

    #[test]
    fn selector_closing_starts_a_fresh_window_for_the_next_selection() {
        let mut h = harness();
        h.state_mut().toggle(GAME);
        h.run_steps(2);
        let first = h.state().selector.id("region_selector");
        h.state_mut().toggle(GAME);
        h.state_mut().toggle(GAME);
        h.run_steps(2);
        assert_ne!(h.state().selector.id("region_selector"), first);
    }

    #[test]
    fn selection_opens_the_box_and_the_card_without_taking_focus() {
        let mut h = harness();
        h.state_mut().toggle(GAME);
        h.run_steps(2);
        let raised_before = raised();
        h.state_mut()
            .flow
            .on_selector(SelectorAction::Selected(RECT));
        h.run_steps(2);
        let host = h.state();
        assert!(host.region_box.shown && host.card.shown);
        assert!(h.state_mut().flow.selector_mut().is_none());
        assert_eq!(raised(), raised_before);
    }

    /// 這一幀之後是否立刻要求下一幀。
    fn repaints_now(h: &Harness<'_, RegionHost>) -> bool {
        h.output().viewport_output[&ViewportId::ROOT].repaint_delay == Duration::ZERO
    }

    /// 清掉之後是否有宿主自己要求的重繪：egui 收到輸入本來就會再畫一幀，要分開來看。
    fn host_requested_repaint() -> bool {
        CAUSES.with(|c| c.borrow().iter().any(|f| f.ends_with("region_host.rs")))
    }

    fn clear_causes() {
        CAUSES.with(|c| c.borrow_mut().clear());
    }

    #[test]
    fn dismissing_the_selector_tears_its_window_down_on_the_next_frame() {
        let mut h = harness();
        h.state_mut().toggle(GAME);
        h.step();
        h.state_mut().selector.hwnd = FAKE_HWND;
        h.run();
        assert!(h.state().selector.shown && !repaints_now(&h));
        clear_causes();
        h.key_press(egui::Key::Escape);
        h.step();
        assert!(repaints_now(&h));
        h.step();
        assert!(host_requested_repaint(), "not left to the input repaint");
        assert!(!h.state().selector.created);
        let textured = h.output().shapes.iter().any(|c| {
            matches!(&c.shape, egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default())
        });
        assert!(!textured, "the frozen frame is no longer drawn");
    }

    #[test]
    fn closing_the_card_tears_its_window_down_on_the_next_frame() {
        let mut h = harness();
        h.state_mut().toggle(GAME);
        h.run_steps(2);
        h.state_mut()
            .flow
            .on_selector(SelectorAction::Selected(RECT));
        h.run();
        assert!(h.state().card.shown && !repaints_now(&h));
        let close = h
            .output()
            .shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == "✕" => {
                    Some(egui::Rect::from_min_size(t.pos, t.galley.size()).center())
                }
                _ => None,
            })
            .expect("close button drawn");
        clear_causes();
        h.event(Event::PointerButton {
            pos: close,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        assert!(repaints_now(&h));
        h.step();
        assert!(host_requested_repaint(), "not left to the input repaint");
        let host = h.state();
        assert!(!host.card.created && !host.region_box.created);
    }

    #[test]
    fn known_windows_are_excluded_when_finding_a_new_viewport() {
        let mut host = RegionHost::new(flow().0);
        let (overlay, bubble, input) = (1, 2, 3);
        host.selector.hwnd = 4;
        host.region_box.hwnd = 5;
        let known = host.known(&[overlay, bubble, input, 0]);
        assert_eq!(known, [1, 2, 3, 4, 5]);
        assert_eq!(
            win32::pick_new_window(&[1, 2, 3, 4], &[1, 2, 3, 4, 5, 6], &known, || 0),
            6,
            "the box created earlier in the same frame is not taken for the card"
        );
    }

    #[test]
    fn box_follows_its_rect_on_screen_coordinates() {
        let mut b = RegionBox::new(RECT);
        let window = region_box::window_rect(RECT);
        let mut h = Harness::builder()
            .with_pixels_per_point(2.0)
            .build_ui_state(
                move |ui, (changed, top_left): &mut (Vec<Rect>, Pos2)| {
                    *top_left = ui.max_rect().min;
                    changed.extend(box_content(ui, &mut b, (window.0, window.1)));
                },
                (Vec::new(), Pos2::ZERO),
            );
        h.step();
        // 框內偏左上一點（視窗 px (50, 50)＝DIP (25, 25)），拖 (20, 10) DIP＝(40, 20) px
        let at = h.state().1 + egui::vec2(25.0, 25.0);
        let to = at + egui::vec2(20.0, 10.0);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        h.input_mut()
            .events
            .extend([button(at, true), Event::PointerMoved(to), button(to, false)]);
        h.step();
        assert_eq!(h.state().0, [(140, 120, 300, 120)]);
    }
}
