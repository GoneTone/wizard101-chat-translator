//! 框選翻譯的主流程：熱鍵 → 擷取一張凍結畫面 → 選取畫面 → 從凍結畫面裁切 → 背景辨識翻譯 → 結果卡。
//!
//! 一律在 UI 執行緒呼叫；辨識翻譯交給 `RegionRunner`，結果經 `UiEvent::RegionDone` 回到 `finish`。
//! 每次送出或改顯示錯誤時 session +1、過期結果丟掉；被取代或沒人要的請求經 `CancellationToken` 撤銷，伺服器才不會照樣生成、計費。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use image::RgbImage;
use tokio_util::sync::CancellationToken;

use crate::i18n::t;
use crate::logs::log;
use crate::region::capture::{CaptureError, Frame, crop_frame};
use crate::region::pipeline::RegionResult;
use crate::ui::geometry::Rect;
use crate::ui::region::card::{Card, CardAction, CardContent};
use crate::ui::region::region_box::RegionBox;
use crate::ui::region::selector::{Selector, SelectorAction};

/// 流程用到的桌面操作；正式版接 WGC、螢幕與前景 API，測試注入替身。
pub trait RegionDesktop {
    /// 把遊戲視窗的 client 區拍成一張凍結畫面。
    fn capture_window(&mut self, hwnd: isize) -> Result<Arc<Frame>, CaptureError>;
    /// 整顆螢幕的截圖，給選取畫面當暗化背景；失敗回全黑，不擋框選。
    fn capture_screen(&mut self, monitor: Rect) -> RgbImage;
    /// 點 (x, y) 所在螢幕的矩形。
    fn monitor_at(&self, x: i32, y: i32) -> Rect;
    /// 把前景交給 `hwnd`。
    fn foreground(&mut self, hwnd: isize);
    /// 列舉視窗找遊戲；找不到回 `None`。
    fn find_game(&self) -> Option<isize>;
    /// 游標的螢幕座標。
    fn cursor(&self) -> (i32, i32);
    /// 從凍結畫面裁出框選矩形（螢幕座標）成 PNG；預設就是 `crop_frame`。
    fn crop(&self, frame: &Frame, rect: Rect) -> Result<Vec<u8>, CaptureError> {
        crop_frame(frame, rect)
    }
}

/// 在背景辨識翻譯一張截圖：完成後投遞 `UiEvent::RegionDone`，被撤銷則什麼都不投遞。
pub trait RegionRunner {
    fn run(&self, png: Vec<u8>, rect: Rect, session: u64, cancel: CancellationToken);
}

/// 一次一個框選；`toggle(hwnd)` 是熱鍵的入口（開選取畫面／取消）。
///
/// 選取畫面、框與卡片只存狀態，視窗由宿主依 `*_mut` 存取器托管，使用者操作經 `on_*` 回報。
/// 卡片每次回到「辨識中」都是新的一張（同 Python 重開視窗），宿主要重設剪貼簿並重新擺放。
pub struct RegionFlow {
    desktop: Box<dyn RegionDesktop>,
    runner: Box<dyn RegionRunner>,
    selecting: Arc<AtomicBool>,
    alpha: f32,
    selector: Option<Selector>,
    region_box: Option<RegionBox>,
    card: Option<Card>,
    session: u64,
    /// 進行中的請求，換新的或沒人要時撤銷。
    request: Option<CancellationToken>,
    game_hwnd: isize,
}

impl RegionFlow {
    /// `selecting` 與熱鍵執行緒共用，永遠等於「選取畫面開著」；卡片透明度預設 1.0，由宿主 `set_alpha`。
    pub fn new(
        desktop: Box<dyn RegionDesktop>,
        runner: Box<dyn RegionRunner>,
        selecting: Arc<AtomicBool>,
    ) -> Self {
        selecting.store(false, Ordering::SeqCst);
        Self {
            desktop,
            runner,
            selecting,
            alpha: 1.0,
            selector: None,
            region_box: None,
            card: None,
            session: 0,
            request: None,
            game_hwnd: 0,
        }
    }

    /// 卡片背景不透明度跟著 overlay 一起變（之後開的卡片也套用）。
    pub fn set_alpha(&mut self, alpha: f32) {
        self.alpha = alpha;
        if let Some(card) = &mut self.card {
            card.set_alpha(alpha);
        }
    }

    /// 熱鍵：選取畫面開著就取消；否則收掉舊的框與卡片、拍一張凍結畫面，在遊戲所在的螢幕開選取畫面。
    ///
    /// `hwnd == 0` 代表選取畫面已經關了（熱鍵取消與放開滑鼠兩條路徑競速），不開一個綁著假 hwnd 的選取畫面。
    pub fn toggle(&mut self, hwnd: isize) {
        if self.selector.is_some() {
            log("[region] selection cancelled by hotkey");
            self.on_selector(SelectorAction::Cancelled);
            return;
        }
        if hwnd == 0 {
            log("[region] toggle ignored: no game window (selector already closed)");
            return;
        }
        self.card = None;
        self.region_box = None;
        self.cancel_request();
        self.game_hwnd = hwnd;
        let frame = match self.desktop.capture_window(hwnd) {
            Ok(frame) => frame,
            Err(e) => {
                // 選取畫面還沒開，遊戲仍是前景，不必還前景
                log(format!(
                    "[region] frame capture failed (hwnd={hwnd:#x}): {e}"
                ));
                let (x, y) = self.desktop.cursor();
                self.show_error((x, y, 0, 0), capture_failed(&e));
                return;
            }
        };
        let monitor = self.desktop.monitor_at(
            frame.client_origin.0 + frame.client_size.0 / 2,
            frame.client_origin.1 + frame.client_size.1 / 2,
        );
        let backdrop = self.desktop.capture_screen(monitor);
        log(format!("[region] selection started (game_hwnd={hwnd:#x})"));
        self.selector = Some(Selector::new(monitor, frame, backdrop));
        self.selecting.store(true, Ordering::SeqCst);
    }

    /// 標題列按鈕的入口：按下當下遊戲多半不是前景，`hwnd` 靠列舉視窗找，找不到就在游標處顯示錯誤卡片。
    pub fn start_from_button(&mut self) {
        if self.selector.is_some() {
            self.toggle(0);
            return;
        }
        let Some(hwnd) = self.desktop.find_game() else {
            log("[region] no game window found for the overlay button");
            let (x, y) = self.desktop.cursor();
            self.show_error((x, y, 0, 0), t("region.no_game", &[]));
            return;
        };
        self.toggle(hwnd);
    }

    /// 辨識翻譯結果（`UiEvent::RegionDone`）：只有目前的 session 才填進卡片，過期的丟掉。
    pub fn finish(&mut self, session: u64, result: Result<RegionResult, String>) {
        if session != self.session {
            if result.is_ok() {
                log(format!(
                    "[region] stale result dropped (session={session}, current={})",
                    self.session
                ));
            }
            return;
        }
        self.set_card(match result {
            Ok(RegionResult { text, source }) => CardContent::Text { text, source },
            Err(message) => CardContent::Error(message),
        });
    }

    /// 開著的選取畫面。
    pub fn selector_mut(&mut self) -> Option<&mut Selector> {
        self.selector.as_mut()
    }

    /// 開著的框選框。
    pub fn region_box_mut(&mut self) -> Option<&mut RegionBox> {
        self.region_box.as_mut()
    }

    /// 開著的結果卡。
    pub fn card_mut(&mut self) -> Option<&mut Card> {
        self.card.as_mut()
    }

    /// 選取畫面回報：框好就從同一張凍結畫面裁切送出；取消（Esc、右鍵、點一下、熱鍵）就把前景還給遊戲。
    pub fn on_selector(&mut self, action: SelectorAction) {
        let Some(selector) = self.selector.take() else {
            return;
        };
        self.selecting.store(false, Ordering::SeqCst);
        let hwnd = self.game_hwnd;
        match action {
            SelectorAction::Selected(rect) => {
                self.desktop.foreground(hwnd);
                self.region_box = Some(RegionBox::new(rect));
                self.translate(rect, hwnd, selector.frame());
            }
            SelectorAction::Cancelled => {
                // 選取畫面開著時搶走了前景；不還的話下一次熱鍵會被當成「前景不是遊戲」擋掉
                log("[region] selection cancelled");
                self.desktop.foreground(hwnd);
            }
        }
    }

    /// 框選框調整完放開：凍結畫面早已過期，對遊戲當下的畫面重拍一張再送出。
    pub fn on_box_changed(&mut self, rect: Rect) {
        let hwnd = self.game_hwnd;
        log(format!(
            "[region] box adjusted, recapturing (game_hwnd={hwnd:#x}, rect={rect:?})"
        ));
        match self.desktop.capture_window(hwnd) {
            Ok(frame) => self.translate(rect, hwnd, &frame),
            Err(e) => {
                log(format!(
                    "[region] frame capture failed (hwnd={hwnd:#x}): {e}"
                ));
                self.show_error(rect, capture_failed(&e));
            }
        }
    }

    /// 卡片回報：使用者關了卡片就連框一起收、撤銷還在跑的請求；複製與借焦點由宿主處理。
    pub fn on_card(&mut self, action: CardAction) {
        if action == CardAction::Close {
            self.card = None;
            self.region_box = None;
            self.cancel_request();
        }
    }

    fn cancel_request(&mut self) {
        if let Some(request) = &self.request
            && !request.is_cancelled()
        {
            log(format!(
                "[region] cancelling the previous request (session={})",
                self.session
            ));
            request.cancel();
        }
    }

    /// 從 `frame` 裁出 `rect`、開辨識中卡片、交給 runner（首次框選與事後調整共用）。
    fn translate(&mut self, rect: Rect, hwnd: isize, frame: &Frame) {
        let png = match self.desktop.crop(frame, rect) {
            Ok(png) => png,
            Err(e) => {
                log(format!(
                    "[region] capture failed (hwnd={hwnd:#x}, rect={rect:?}): {e}"
                ));
                self.show_error(
                    rect,
                    match e {
                        CaptureError::OutsideGame(_) => t("region.outside_game", &[]),
                        CaptureError::Failed(_) => capture_failed(&e),
                    },
                );
                return;
            }
        };
        self.card = Some(Card::new(rect, self.alpha));
        self.cancel_request();
        self.session += 1;
        let cancel = CancellationToken::new();
        self.request = Some(cancel.clone());
        self.runner.run(png, rect, self.session, cancel);
    }

    /// 換成錯誤卡片；進行中的請求一併撤銷並換 session，晚到的結果才不會蓋掉錯誤（Python 版會蓋掉）。
    fn show_error(&mut self, rect: Rect, message: String) {
        self.cancel_request();
        self.session += 1;
        self.card = Some(Card::new(rect, self.alpha));
        self.set_card(CardContent::Error(message));
    }

    fn set_card(&mut self, content: CardContent) {
        if let Some(card) = &mut self.card {
            card.set_content(content);
        }
    }
}

fn capture_failed(e: &CaptureError) -> String {
    t("region.capture_failed", &[("error", &e.to_string())])
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::rc::Rc;

    use super::*;
    use crate::region::ocr::OcrUnavailable;
    use crate::region::pipeline::{RegionError, describe_error};
    use crate::ui::region::fakes::{
        CURSOR, Crop, FOUND, FakeDesktop, FakeRunner, GAME, MONITOR, RECT, Shared, boom, desktop,
        frame,
    };

    struct H {
        flow: RegionFlow,
        calls: Shared,
        selecting: Arc<AtomicBool>,
        frame: Arc<Frame>,
    }

    impl H {
        fn new(desktop: FakeDesktop) -> Self {
            let calls = Rc::clone(&desktop.calls);
            let frame = Arc::clone(&desktop.fallback);
            let selecting = Arc::new(AtomicBool::new(false));
            let flow = RegionFlow::new(
                Box::new(desktop),
                Box::new(FakeRunner(Rc::clone(&calls))),
                Arc::clone(&selecting),
            );
            Self {
                flow,
                calls,
                selecting,
                frame,
            }
        }

        fn selecting(&mut self) -> bool {
            let open = self.flow.selector_mut().is_some();
            assert_eq!(open, self.selecting.load(Ordering::SeqCst));
            open
        }

        fn pick(&mut self, rect: Rect) {
            self.flow.on_selector(SelectorAction::Selected(rect));
        }

        fn sessions(&self) -> Vec<u64> {
            self.calls.borrow().runs.iter().map(|r| r.1).collect()
        }

        fn token(&self, index: usize) -> CancellationToken {
            self.calls.borrow().runs[index].2.clone()
        }

        fn finish_last(&mut self, result: Result<RegionResult, String>) {
            let session = *self.sessions().last().expect("a request was sent");
            self.flow.finish(session, result);
        }

        /// 卡片的矩形與顯示文字；沒有卡片回 `None`。
        fn card(&mut self) -> Option<(Rect, String)> {
            self.flow.card_mut().map(|c| (c.rect(), c.all_text()))
        }
    }

    fn shown(content: CardContent) -> String {
        let mut card = Card::new((0, 0, 0, 0), 1.0);
        card.set_content(content);
        card.all_text()
    }

    fn pending() -> String {
        shown(CardContent::Pending)
    }

    fn text(text: &str, source: &str) -> CardContent {
        CardContent::Text {
            text: text.into(),
            source: source.into(),
        }
    }

    fn result(text: &str, source: &str) -> Result<RegionResult, String> {
        Ok(RegionResult {
            text: text.into(),
            source: source.into(),
        })
    }

    fn error(message: &str) -> String {
        shown(CardContent::Error(message.into()))
    }

    #[test]
    fn toggle_opens_the_selector_and_a_second_toggle_cancels_it() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        assert!(h.selecting());
        h.flow.toggle(GAME);
        assert!(!h.selecting());
    }

    #[test]
    fn toggle_passes_the_captured_frame_to_the_selector() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        let selector = h.flow.selector_mut().unwrap();
        assert!(Arc::ptr_eq(selector.frame(), &h.frame));
    }

    #[test]
    fn toggle_passes_the_captured_backdrop_to_the_selector() {
        let mut h = H::new(FakeDesktop {
            backdrop: (3, 2),
            ..desktop()
        });
        h.flow.toggle(GAME);
        assert_eq!(h.calls.borrow().screens, [MONITOR]);
        let selector = h.flow.selector_mut().unwrap();
        assert_eq!(selector.backdrop_size(), Some([3, 2]));
    }

    #[test]
    fn selection_captures_and_fills_the_card_with_the_translation() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        assert_eq!(h.card(), Some((RECT, pending())));
        h.finish_last(result("譯文", ""));
        assert_eq!(h.card(), Some((RECT, shown(text("譯文", "")))));
    }

    #[test]
    fn selection_passes_the_source_text_to_the_card() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        h.finish_last(result("譯文", "Talk to Merle"));
        assert_eq!(h.card(), Some((RECT, shown(text("譯文", "Talk to Merle")))));
    }

    #[test]
    fn selection_crops_the_same_frame_that_was_captured() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        let frame = Arc::as_ptr(&h.frame) as usize;
        assert_eq!(h.calls.borrow().crops, [(frame, RECT)]);
    }

    #[test]
    fn frame_capture_failure_is_shown_at_the_cursor_without_opening_the_selector() {
        let mut h = H::new(FakeDesktop {
            frames: VecDeque::from([Err(boom())]),
            ..desktop()
        });
        h.flow.toggle(GAME);
        let at_cursor = (CURSOR.0, CURSOR.1, 0, 0);
        assert_eq!(h.card(), Some((at_cursor, error(&capture_failed(&boom())))));
        assert!(!h.selecting());
        assert!(h.sessions().is_empty());
    }

    #[test]
    fn the_selector_covers_the_monitor_under_the_middle_of_the_game_client_area() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        assert_eq!(h.calls.borrow().monitor_at, [(100 + 400, 200 + 300)]);
    }

    #[test]
    fn capture_failure_is_shown_on_the_card_without_a_worker() {
        let mut h = H::new(FakeDesktop {
            crop: Crop::Real,
            ..desktop()
        });
        h.flow.toggle(GAME);
        let outside = (0, 0, 50, 50);
        h.pick(outside);
        let message = t("region.outside_game", &[]);
        assert_eq!(h.card(), Some((outside, error(&message))));
        assert!(h.sessions().is_empty());
    }

    #[test]
    fn plain_capture_error_is_shown_as_capture_failed() {
        let mut h = H::new(FakeDesktop {
            crop: Crop::Fail(boom()),
            ..desktop()
        });
        h.flow.toggle(GAME);
        h.pick(RECT);
        assert_eq!(h.card(), Some((RECT, error(&capture_failed(&boom())))));
        assert!(h.sessions().is_empty());
    }

    #[test]
    fn pipeline_errors_are_described_on_the_card() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        let message = describe_error(&RegionError::Ocr(OcrUnavailable("no pack".into())));
        h.finish_last(Err(message));
        let expected = error(&t("region.ocr_unavailable", &[]));
        assert_eq!(h.card(), Some((RECT, expected)));
    }

    #[test]
    fn stale_result_is_dropped_after_a_new_selection() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        h.flow.toggle(GAME);
        h.pick((0, 0, 50, 50));
        assert_eq!(h.sessions(), [1, 2]);
        h.flow.finish(1, result("譯文300", ""));
        assert_eq!(h.card(), Some(((0, 0, 50, 50), pending())));
        h.flow.finish(2, result("譯文50", ""));
        assert_eq!(h.card(), Some(((0, 0, 50, 50), shown(text("譯文50", "")))));
    }

    #[test]
    fn stale_error_is_dropped_after_a_new_selection() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        h.flow.toggle(GAME);
        h.pick((0, 0, 50, 50));
        let message = t("region.ocr_unavailable", &[]);
        h.flow.finish(1, Err(message.clone()));
        assert_eq!(h.card(), Some(((0, 0, 50, 50), pending())));
        h.flow.finish(2, Err(message.clone()));
        assert_eq!(h.card(), Some(((0, 0, 50, 50), error(&message))));
    }

    #[test]
    fn selection_moves_foreground_before_showing_the_pending_card() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        assert_eq!(h.calls.borrow().order, ["foreground 0x1234", "run 1"]);
        assert_eq!(h.card(), Some((RECT, pending())));
    }

    #[test]
    fn toggle_hides_a_previous_card() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        h.finish_last(result("譯文", ""));
        h.flow.toggle(GAME);
        assert_eq!(h.card(), None);
    }

    #[test]
    fn esc_style_cancel_restores_the_games_foreground() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.flow.on_selector(SelectorAction::Cancelled);
        assert_eq!(h.calls.borrow().foreground, [GAME]);
        assert!(!h.selecting());
    }

    #[test]
    fn hotkey_cancel_restores_the_games_foreground() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.flow.toggle(GAME);
        assert_eq!(h.calls.borrow().foreground, [GAME]);
    }

    #[test]
    fn toggle_with_hwnd_zero_is_ignored_once_the_selector_is_already_closed() {
        let mut h = H::new(desktop());
        h.flow.toggle(0);
        assert!(!h.selecting());
        assert!(h.calls.borrow().captured.is_empty());
    }

    #[test]
    fn set_alpha_forwards_to_the_card() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        h.flow.set_alpha(0.5);
        assert_eq!(h.flow.card_mut().unwrap().alpha(), 0.5);
        h.flow.toggle(GAME);
        h.pick(RECT);
        assert_eq!(h.flow.card_mut().unwrap().alpha(), 0.5);
    }

    #[test]
    fn start_from_button_opens_the_selector_with_the_found_window() {
        let mut h = H::new(FakeDesktop {
            game: Some(FOUND),
            ..desktop()
        });
        h.flow.start_from_button();
        assert!(h.selecting());
        assert_eq!(h.calls.borrow().captured, [0x9999]);
    }

    #[test]
    fn start_from_button_shows_an_error_card_when_no_game_window_is_found() {
        let mut h = H::new(FakeDesktop {
            game: None,
            ..desktop()
        });
        h.flow.start_from_button();
        assert!(!h.selecting());
        let at_cursor = (CURSOR.0, CURSOR.1, 0, 0);
        assert_eq!(
            h.card(),
            Some((at_cursor, error(&t("region.no_game", &[]))))
        );
    }

    #[test]
    fn start_from_button_cancels_an_already_open_selector() {
        let mut h = H::new(desktop());
        h.flow.start_from_button();
        assert!(h.selecting());
        h.flow.start_from_button();
        assert!(!h.selecting());
        assert_eq!(h.calls.borrow().foreground, [GAME]);
    }

    #[test]
    fn selection_leaves_the_box_on_the_selected_rect() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        h.finish_last(result("譯文", ""));
        assert_eq!(h.flow.region_box_mut().map(|b| b.rect()), Some(RECT));
    }

    #[test]
    fn adjusting_the_box_recaptures_the_live_game_and_retranslates() {
        let (first, live) = (frame(), frame());
        let mut h = H::new(FakeDesktop {
            frames: VecDeque::from([Ok(Arc::clone(&first)), Ok(Arc::clone(&live))]),
            ..desktop()
        });
        h.flow.toggle(GAME);
        h.pick(RECT);
        h.finish_last(result("譯文300", ""));
        let new_rect = (120, 110, 340, 130);
        h.flow.on_box_changed(new_rect);
        assert_eq!(h.calls.borrow().captured, [GAME, GAME]);
        let ptr = |f: &Arc<Frame>| Arc::as_ptr(f) as usize;
        assert_eq!(
            h.calls.borrow().crops,
            [(ptr(&first), RECT), (ptr(&live), new_rect)]
        );
        assert_eq!(h.card(), Some((new_rect, pending())));
        h.finish_last(result("譯文340", ""));
        assert_eq!(h.card(), Some((new_rect, shown(text("譯文340", "")))));
        assert!(h.flow.region_box_mut().is_some());
    }

    #[test]
    fn capture_failure_while_adjusting_is_shown_and_keeps_the_box() {
        let mut h = H::new(FakeDesktop {
            frames: VecDeque::from([Ok(frame()), Err(boom())]),
            ..desktop()
        });
        h.flow.toggle(GAME);
        h.pick(RECT);
        h.finish_last(result("譯文", ""));
        h.flow.on_box_changed((0, 0, 50, 50));
        let expected = error(&capture_failed(&boom()));
        assert_eq!(h.card(), Some(((0, 0, 50, 50), expected)));
        assert!(h.flow.region_box_mut().is_some());
    }

    #[test]
    fn closing_the_card_hides_the_box() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        h.finish_last(result("譯文", ""));
        h.flow.on_card(CardAction::Close);
        assert_eq!(h.card(), None);
        assert!(h.flow.region_box_mut().is_none());
    }

    #[test]
    fn a_new_selection_hides_the_previous_box() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        h.finish_last(result("譯文", ""));
        h.flow.toggle(GAME);
        assert!(h.flow.region_box_mut().is_none());
        assert!(h.selecting());
    }

    #[test]
    fn each_request_gets_its_own_cancel_handle() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        h.finish_last(result("譯文", ""));
        assert_eq!(h.sessions(), [1]);
        assert!(!h.token(0).is_cancelled());
    }

    #[test]
    fn adjusting_the_box_cancels_the_previous_request() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        h.finish_last(result("譯文", ""));
        h.flow.on_box_changed((0, 0, 50, 50));
        assert_eq!(h.sessions(), [1, 2]);
        assert!(h.token(0).is_cancelled() && !h.token(1).is_cancelled());
    }

    #[test]
    fn a_new_selection_cancels_the_previous_request() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        h.finish_last(result("譯文", ""));
        h.flow.toggle(GAME);
        assert!(h.token(0).is_cancelled());
    }

    #[test]
    fn closing_the_card_cancels_the_request() {
        let mut h = H::new(desktop());
        h.flow.toggle(GAME);
        h.pick(RECT);
        h.flow.on_card(CardAction::Close);
        assert!(h.token(0).is_cancelled());
    }

    /// 先送出一個請求，再讓 `fail` 把卡片換成錯誤：那個請求被撤銷，晚到的結果與錯誤都蓋不掉錯誤卡片。
    fn late_results_leave_the_error_card(desktop: FakeDesktop, fail: impl FnOnce(&mut H)) {
        let mut h = H::new(desktop);
        h.flow.toggle(GAME);
        h.pick(RECT);
        assert_eq!(h.sessions(), [1]);
        fail(&mut h);
        let card = h.card().expect("an error card");
        assert_ne!(card.1, pending());
        assert!(h.token(0).is_cancelled());
        h.flow.finish(1, result("late", ""));
        h.flow.finish(1, Err("late error".into()));
        assert_eq!(h.card(), Some(card));
    }

    #[test]
    fn a_recapture_failure_cancels_the_request_and_keeps_its_error() {
        late_results_leave_the_error_card(
            FakeDesktop {
                frames: VecDeque::from([Ok(frame()), Err(boom())]),
                ..desktop()
            },
            |h| h.flow.on_box_changed((0, 0, 50, 50)),
        );
    }

    #[test]
    fn a_crop_failure_cancels_the_request_and_keeps_its_error() {
        late_results_leave_the_error_card(
            FakeDesktop {
                crop: Crop::FailAt((0, 0, 50, 50), boom()),
                ..desktop()
            },
            |h| h.flow.on_box_changed((0, 0, 50, 50)),
        );
    }

    #[test]
    fn a_frame_capture_failure_on_toggle_keeps_its_error() {
        late_results_leave_the_error_card(
            FakeDesktop {
                frames: VecDeque::from([Ok(frame()), Err(boom())]),
                ..desktop()
            },
            |h| h.flow.toggle(GAME),
        );
    }

    #[test]
    fn no_game_for_the_button_keeps_its_error() {
        late_results_leave_the_error_card(
            FakeDesktop {
                game: None,
                ..desktop()
            },
            |h| h.flow.start_from_button(),
        );
    }
}
