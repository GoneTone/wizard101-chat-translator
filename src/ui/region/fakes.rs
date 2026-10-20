//! 框選流程的替身：桌面操作、擷取與 runner 只記下呼叫，不碰真的視窗、截圖或網路。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use image::RgbImage;
use tokio_util::sync::CancellationToken;

use crate::region::capture::{CaptureError, Frame, crop_frame};
use crate::ui::events::UiEvent;
use crate::ui::geometry::Rect;
use crate::ui::region::flow::{
    CaptureKind, RegionCapturer, RegionDesktop, RegionFlow, RegionRunner, Shot,
};
use crate::ui::region::selector::images;

/// 落在凍結畫面內的框選矩形。
pub(crate) const RECT: Rect = (100, 100, 300, 120);
pub(crate) const MONITOR: Rect = (0, 0, 1920, 1080);
/// 熱鍵帶來的遊戲視窗。
pub(crate) const GAME: isize = 0x1234;
/// ⛶ 列舉視窗找到的遊戲視窗。
pub(crate) const FOUND: isize = 0x9999;
pub(crate) const CURSOR: (i32, i32) = (40, 50);

/// 凍結畫面的影像內容與流程無關；流程只讀 client 區的位置與大小，所以 4x4 影像與 `client_size` 不必一致。
pub(crate) fn frame() -> Arc<Frame> {
    Arc::new(Frame {
        image: RgbImage::new(4, 4),
        client_origin: (100, 200),
        client_size: (800, 600),
    })
}

pub(crate) fn boom() -> CaptureError {
    CaptureError::Failed("no frame within 2.0s".into())
}

#[derive(Default)]
pub(crate) struct Calls {
    /// 前景與送出請求的先後。
    pub(crate) order: Vec<String>,
    pub(crate) captured: Vec<isize>,
    /// 擷取完成、還沒交給流程的結果（依發出先後）。
    pub(crate) pending: VecDeque<(u64, Result<Shot, CaptureError>)>,
    pub(crate) foreground: Vec<isize>,
    /// 裁切時收到的畫面（以位址辨識）與矩形。
    pub(crate) crops: Vec<(usize, Rect)>,
    pub(crate) runs: Vec<(Rect, u64, CancellationToken)>,
}

pub(crate) type Shared = Rc<RefCell<Calls>>;

pub(crate) enum Crop {
    Png,
    Real,
    Fail(CaptureError),
    /// 只有裁這個矩形時失敗。
    FailAt(Rect, CaptureError),
}

pub(crate) struct FakeDesktop {
    pub(crate) calls: Shared,
    pub(crate) crop: Crop,
    pub(crate) game: Option<isize>,
}

impl RegionDesktop for FakeDesktop {
    fn foreground(&mut self, hwnd: isize) {
        let mut calls = self.calls.borrow_mut();
        calls.foreground.push(hwnd);
        calls.order.push(format!("foreground {hwnd:#x}"));
    }

    fn find_game(&self) -> Option<isize> {
        self.game
    }

    fn cursor(&self) -> (i32, i32) {
        CURSOR
    }

    fn crop(&self, frame: &Frame, rect: Rect) -> Result<Vec<u8>, CaptureError> {
        self.calls
            .borrow_mut()
            .crops
            .push((std::ptr::from_ref(frame) as usize, rect));
        match &self.crop {
            Crop::Png => Ok(b"png".to_vec()),
            Crop::Real => crop_frame(frame, rect),
            Crop::Fail(e) => Err(e.clone()),
            Crop::FailAt(at, e) if *at == rect => Err(e.clone()),
            Crop::FailAt(..) => Ok(b"png".to_vec()),
        }
    }
}

pub(crate) struct FakeRunner(pub(crate) Shared);

impl RegionRunner for FakeRunner {
    fn run(&self, png: Vec<u8>, rect: Rect, session: u64, cancel: CancellationToken) {
        assert!(!png.is_empty());
        let mut calls = self.0.borrow_mut();
        calls.order.push(format!("run {session}"));
        calls.runs.push((rect, session, cancel));
    }
}

pub(crate) fn desktop() -> FakeDesktop {
    FakeDesktop {
        calls: Shared::default(),
        crop: Crop::Png,
        game: Some(GAME),
    }
}

/// 當場算好結果存進 `calls.pending`，由測試決定何時以 `deliver` 交給流程。
pub(crate) struct FakeCapturer {
    pub(crate) calls: Shared,
    /// 依序回傳；用完就一直回 `fallback`。
    pub(crate) frames: RefCell<VecDeque<Result<Arc<Frame>, CaptureError>>>,
    pub(crate) fallback: Arc<Frame>,
    pub(crate) backdrop: (u32, u32),
}

impl RegionCapturer for FakeCapturer {
    fn capture(&self, id: u64, hwnd: isize, kind: CaptureKind) {
        let next = self.frames.borrow_mut().pop_front();
        let result = next.unwrap_or_else(|| Ok(Arc::clone(&self.fallback)));
        let result = result.map(|frame| {
            let selector = (kind == CaptureKind::Select).then(|| {
                let backdrop = RgbImage::new(self.backdrop.0, self.backdrop.1);
                (MONITOR, images(&frame, &backdrop))
            });
            Shot { frame, selector }
        });
        let mut calls = self.calls.borrow_mut();
        calls.captured.push(hwnd);
        calls.pending.push_back((id, result));
    }
}

pub(crate) fn capturer() -> FakeCapturer {
    FakeCapturer {
        calls: Shared::default(),
        frames: RefCell::default(),
        fallback: frame(),
        backdrop: (1, 1),
    }
}

/// 最早一筆擷取結果，包成背景執行緒會投遞的事件。
pub(crate) fn delivery(calls: &Shared) -> UiEvent {
    let (id, result) = calls
        .borrow_mut()
        .pending
        .pop_front()
        .expect("a capture is pending");
    UiEvent::RegionCaptured { id, result }
}

/// 把最早一筆擷取結果交給流程。
pub(crate) fn deliver(flow: &mut RegionFlow, calls: &Shared) {
    let UiEvent::RegionCaptured { id, result } = delivery(calls) else {
        unreachable!()
    };
    flow.on_captured(id, result);
}

/// 接上替身的流程與它的呼叫紀錄。
pub(crate) fn flow() -> (RegionFlow, Shared) {
    let desktop = FakeDesktop {
        game: Some(FOUND),
        ..desktop()
    };
    let calls = Rc::clone(&desktop.calls);
    let capturer = FakeCapturer {
        calls: Rc::clone(&calls),
        ..capturer()
    };
    let flow = RegionFlow::new(
        Box::new(desktop),
        Box::new(capturer),
        Box::new(FakeRunner(Rc::clone(&calls))),
        Arc::new(AtomicBool::new(false)),
    );
    (flow, calls)
}
