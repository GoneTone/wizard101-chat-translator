//! 框選流程的 log：過期結果、撤銷請求、取消與擷取失敗；裁切走預設的 `crop_frame`，桌面、擷取與 runner 用替身。

use crate::common;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use common::log_capture::{capture_logs, logged};
use image::{Rgb, RgbImage};
use tokio_util::sync::CancellationToken;
use wizard101_chat_translator::region::capture::{CaptureError, Frame};
use wizard101_chat_translator::region::pipeline::RegionResult;
use wizard101_chat_translator::ui::geometry::Rect;
use wizard101_chat_translator::ui::region::flow::{
    CaptureKind, RegionCapturer, RegionDesktop, RegionFlow, RegionRunner, Shot,
};
use wizard101_chat_translator::ui::region::selector::{self, SelectorAction};

/// client 區在 (0, 0)、8×8，每個像素顏色不同，裁切才會成功。
fn frame() -> Arc<Frame> {
    Arc::new(Frame {
        image: RgbImage::from_fn(8, 8, |x, y| Rgb([x as u8 * 30, y as u8 * 30, 0])),
        client_origin: (0, 0),
        client_size: (8, 8),
    })
}

struct Desktop;

impl RegionDesktop for Desktop {
    fn foreground(&mut self, _hwnd: isize) {}

    fn find_game(&self) -> Option<isize> {
        None
    }

    fn cursor(&self) -> (i32, i32) {
        (0, 0)
    }
}

type Done = Rc<RefCell<VecDeque<(u64, Result<Shot, CaptureError>)>>>;

/// 當場算好結果存著，由測試以 `deliver` 交回流程。
struct Capturer {
    fail: bool,
    done: Done,
}

impl RegionCapturer for Capturer {
    fn capture(&self, id: u64, _hwnd: isize, kind: CaptureKind) {
        let result = if self.fail {
            Err(CaptureError::Failed("no frame within 2.0s".into()))
        } else {
            let frame = frame();
            let selector = (kind == CaptureKind::Select).then(|| {
                (
                    (0, 0, 1920, 1080),
                    selector::images(&frame, &RgbImage::new(1, 1)),
                )
            });
            Ok(Shot { frame, selector })
        };
        self.done.borrow_mut().push_back((id, result));
    }
}

type Runs = Rc<RefCell<Vec<(Vec<u8>, u64, CancellationToken)>>>;

struct Runner(Runs);

impl RegionRunner for Runner {
    fn run(&self, png: Vec<u8>, _rect: Rect, session: u64, cancel: CancellationToken) {
        self.0.borrow_mut().push((png, session, cancel));
    }
}

struct H {
    flow: RegionFlow,
    runs: Runs,
    done: Done,
}

impl H {
    fn new(fail: bool) -> Self {
        capture_logs();
        let (runs, done) = (Runs::default(), Done::default());
        let flow = RegionFlow::new(
            Box::new(Desktop),
            Box::new(Capturer {
                fail,
                done: Rc::clone(&done),
            }),
            Box::new(Runner(Rc::clone(&runs))),
            Arc::new(AtomicBool::new(false)),
        );
        Self { flow, runs, done }
    }

    /// 把最早一筆擷取結果交回流程。
    fn deliver(&mut self) {
        let (id, result) = self.done.borrow_mut().pop_front().expect("a capture");
        self.flow.on_captured(id, result);
    }

    fn select(&mut self, hwnd: isize, rect: Rect) {
        self.flow.toggle(hwnd);
        self.deliver();
        self.flow.on_selector(SelectorAction::Selected(rect));
    }
}

#[test]
fn stale_result_is_dropped_and_the_replaced_request_cancelled_with_logs() {
    let mut h = H::new(false);
    h.select(0x5050, (2, 2, 4, 4));
    h.select(0x5050, (1, 1, 3, 3));
    assert_eq!(
        h.runs.borrow().iter().map(|r| r.1).collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(h.runs.borrow()[0].2.is_cancelled() && !h.runs.borrow()[1].2.is_cancelled());
    h.flow.finish(1, Ok(RegionResult::default()));
    assert!(logged("[region] selection started (game_hwnd=0x5050)"));
    assert!(logged(
        "[region] cancelling the previous request (session=1)"
    ));
    assert!(logged(
        "[region] stale result dropped (session=1, current=2)"
    ));
}

#[test]
fn the_default_crop_sends_the_selected_part_of_the_frame() {
    let mut h = H::new(false);
    h.select(0x5151, (2, 3, 4, 5));
    let png = h.runs.borrow()[0].0.clone();
    let cropped = image::load_from_memory(&png).unwrap().to_rgb8();
    assert_eq!(cropped.dimensions(), (4, 5));
    assert_eq!(cropped.get_pixel(0, 0), &Rgb([60, 90, 0]));
}

#[test]
fn a_selection_outside_the_game_logs_the_capture_failure() {
    let mut h = H::new(false);
    h.select(0x5252, (20, 20, 5, 5));
    assert!(h.runs.borrow().is_empty());
    assert!(logged(
        "[region] capture failed (hwnd=0x5252, rect=(20, 20, 5, 5)): \
         selection outside game window (rect=(20, 20, 5, 5), client=(0, 0, 8, 8))"
    ));
}

#[test]
fn frame_capture_failure_is_logged() {
    let mut h = H::new(true);
    h.flow.toggle(0x5353);
    h.deliver();
    assert!(logged(
        "[region] frame capture failed (hwnd=0x5353): no frame within 2.0s"
    ));
}

#[test]
fn hotkey_cancel_logs_both_lines_and_a_closed_toggle_is_ignored() {
    let mut h = H::new(false);
    h.flow.toggle(0x5454);
    h.deliver();
    h.flow.toggle(0x5454);
    h.flow.toggle(0);
    assert!(logged("[region] selection cancelled by hotkey"));
    assert!(logged("[region] selection cancelled"));
    assert!(logged(
        "[region] toggle ignored: no game window (selector already closed)"
    ));
}

#[test]
fn adjusting_the_box_logs_the_recapture() {
    let mut h = H::new(false);
    h.select(0x5555, (2, 2, 4, 4));
    h.flow.on_box_changed((1, 1, 2, 2));
    h.deliver();
    assert_eq!(h.runs.borrow().len(), 2);
    assert!(logged(
        "[region] box adjusted, recapturing (game_hwnd=0x5555, rect=(1, 1, 2, 2))"
    ));
}

#[test]
fn the_button_logs_when_no_game_window_is_found() {
    let mut h = H::new(false);
    h.flow.start_from_button();
    assert!(h.flow.card_mut().is_some());
    assert!(logged(
        "[region] no game window found for the overlay button"
    ));
}

#[test]
fn capture_start_stale_drop_and_cancel_while_capturing_are_logged() {
    let mut h = H::new(false);
    h.flow.toggle(0x5656);
    assert!(logged(
        "[region] capture started (id=1, hwnd=0x5656, purpose=select)"
    ));
    h.flow.toggle(0x5656);
    assert!(logged("[region] selection cancelled while capturing"));
    h.deliver();
    assert!(logged(
        "[region] stale capture dropped (id=1, current=none)"
    ));
}
