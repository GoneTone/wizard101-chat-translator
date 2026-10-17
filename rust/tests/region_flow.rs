//! 框選流程的 log：過期結果、撤銷請求、取消與擷取失敗；裁切走預設的 `crop_frame`，桌面與 runner 用替身。

#[allow(dead_code)]
mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use common::log_capture::{capture_logs, logged};
use image::{Rgb, RgbImage};
use tokio_util::sync::CancellationToken;
use wizard101_chat_translator::region::capture::{CaptureError, Frame};
use wizard101_chat_translator::region::pipeline::RegionResult;
use wizard101_chat_translator::ui::geometry::Rect;
use wizard101_chat_translator::ui::region::flow::{RegionDesktop, RegionFlow, RegionRunner};
use wizard101_chat_translator::ui::region::selector::SelectorAction;

/// client 區在 (0, 0)、8×8，每個像素顏色不同，裁切才會成功。
fn frame() -> Arc<Frame> {
    Arc::new(Frame {
        image: RgbImage::from_fn(8, 8, |x, y| Rgb([x as u8 * 30, y as u8 * 30, 0])),
        client_origin: (0, 0),
        client_size: (8, 8),
    })
}

struct Desktop {
    fail_capture: bool,
}

impl RegionDesktop for Desktop {
    fn capture_window(&mut self, _hwnd: isize) -> Result<Arc<Frame>, CaptureError> {
        if self.fail_capture {
            return Err(CaptureError::Failed("no frame within 2.0s".into()));
        }
        Ok(frame())
    }

    fn capture_screen(&mut self, _monitor: Rect) -> RgbImage {
        RgbImage::new(1, 1)
    }

    fn monitor_at(&self, _x: i32, _y: i32) -> Rect {
        (0, 0, 1920, 1080)
    }

    fn foreground(&mut self, _hwnd: isize) {}

    fn find_game(&self) -> Option<isize> {
        None
    }

    fn cursor(&self) -> (i32, i32) {
        (0, 0)
    }
}

type Runs = Rc<RefCell<Vec<(Vec<u8>, u64)>>>;

struct Runner(Runs);

impl RegionRunner for Runner {
    fn run(&self, png: Vec<u8>, _rect: Rect, session: u64, _cancel: CancellationToken) {
        self.0.borrow_mut().push((png, session));
    }
}

fn flow(fail_capture: bool) -> (RegionFlow, Runs) {
    capture_logs();
    let runs = Runs::default();
    let flow = RegionFlow::new(
        Box::new(Desktop { fail_capture }),
        Box::new(Runner(Rc::clone(&runs))),
        Arc::new(AtomicBool::new(false)),
    );
    (flow, runs)
}

fn select(flow: &mut RegionFlow, hwnd: isize, rect: Rect) {
    flow.toggle(hwnd);
    flow.on_selector(SelectorAction::Selected(rect));
}

#[test]
fn stale_result_is_dropped_and_the_replaced_request_cancelled_with_logs() {
    let (mut flow, runs) = flow(false);
    select(&mut flow, 0x5050, (2, 2, 4, 4));
    select(&mut flow, 0x5050, (1, 1, 3, 3));
    assert_eq!(
        runs.borrow().iter().map(|r| r.1).collect::<Vec<_>>(),
        [1, 2]
    );
    flow.finish(1, Ok(RegionResult::default()));
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
    let (mut flow, runs) = flow(false);
    select(&mut flow, 0x5151, (2, 3, 4, 5));
    let png = runs.borrow()[0].0.clone();
    let cropped = image::load_from_memory(&png).unwrap().to_rgb8();
    assert_eq!(cropped.dimensions(), (4, 5));
    assert_eq!(cropped.get_pixel(0, 0), &Rgb([60, 90, 0]));
}

#[test]
fn a_selection_outside_the_game_logs_the_capture_failure() {
    let (mut flow, runs) = flow(false);
    select(&mut flow, 0x5252, (20, 20, 5, 5));
    assert!(runs.borrow().is_empty());
    assert!(logged(
        "[region] capture failed (hwnd=0x5252, rect=(20, 20, 5, 5)): \
         selection outside game window (rect=(20, 20, 5, 5), client=(0, 0, 8, 8))"
    ));
}

#[test]
fn frame_capture_failure_is_logged() {
    let (mut flow, _) = flow(true);
    flow.toggle(0x5353);
    assert!(logged(
        "[region] frame capture failed (hwnd=0x5353): no frame within 2.0s"
    ));
}

#[test]
fn hotkey_cancel_logs_both_lines_and_a_closed_toggle_is_ignored() {
    let (mut flow, _) = flow(false);
    flow.toggle(0x5454);
    flow.toggle(0x5454);
    flow.toggle(0);
    assert!(logged("[region] selection cancelled by hotkey"));
    assert!(logged("[region] selection cancelled"));
    assert!(logged(
        "[region] toggle ignored: no game window (selector already closed)"
    ));
}

#[test]
fn adjusting_the_box_logs_the_recapture() {
    let (mut flow, runs) = flow(false);
    select(&mut flow, 0x5555, (2, 2, 4, 4));
    flow.on_box_changed((1, 1, 2, 2));
    assert_eq!(runs.borrow().len(), 2);
    assert!(logged(
        "[region] box adjusted, recapturing (game_hwnd=0x5555, rect=(1, 1, 2, 2))"
    ));
}

#[test]
fn the_button_logs_when_no_game_window_is_found() {
    let (mut flow, _) = flow(false);
    flow.start_from_button();
    assert!(flow.card_mut().is_some());
    assert!(logged(
        "[region] no game window found for the overlay button"
    ));
}
