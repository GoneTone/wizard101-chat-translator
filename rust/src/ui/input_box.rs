//! 翻譯輸入框：打字（任何語言）→ Enter 翻成遊戲語言、Esc 關閉。
//!
//! 這裡只有狀態、內容與擺放幾何；視窗由 App 托管。遊戲聊天輸入框開著就貼在它正下方、同寬，
//! 沒開（熱鍵呼出）就貼在游標處。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Color32, Event, FontId, ImeEvent, Key, Margin, TextEdit, Ui};
use tokio_util::sync::CancellationToken;

use crate::i18n::t;
use crate::input::dispatch::InputBoxControl;
use crate::logs::log;
use crate::ui::fonts_egui::tk_pt;
use crate::ui::geometry::{Rect, anchored_position};
use crate::ui::palette::{FG_ERROR, FG_UPDATE};
use crate::ui::richtext;

/// 底色。
pub const BG: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x24);
/// 輸入文字。
pub const FG: Color32 = Color32::from_rgb(0xf2, 0xf2, 0xf7);
/// 提示與「翻譯中」文字。
pub const HINT_FG: Color32 = Color32::from_rgb(0x9a, 0x9a, 0xa8);
/// 輸入欄底色。
pub const ENTRY_BG: Color32 = Color32::from_rgb(0x26, 0x26, 0x36);
/// 遊戲聊天輸入框的長度上限（實測）。
pub const GAME_INPUT_MAX_CHARS: usize = 80;
/// 沒有錨點時的寬度（DIP）；有錨點就跟遊戲輸入框同寬。
pub const DEFAULT_WIDTH: f32 = 460.0;
/// 遊戲輸入框再窄也不跟（DIP）：提示文字會擠成一長條，且輸入欄放不下一句話。
pub const MIN_WIDTH: f32 = 320.0;
/// 與遊戲輸入框／游標的垂直間距（DIP）。
pub const ANCHOR_GAP: f32 = 4.0;

/// 交給翻譯端的一次發話請求；結果以 `session` 經 `InputBox::finish` 送回。
pub struct OutgoingRequest {
    /// 壓縮過空白的原文。
    pub text: String,
    /// 譯文要打回的遊戲視窗。
    pub target: isize,
    /// 送出當下的 session；對不上的結果會被丟掉。
    pub session: u64,
    /// 輸入框關閉時被 cancel，翻譯端應撤銷請求。
    pub cancel: CancellationToken,
}

/// `content` 這一幀觸發的動作（動作本身已在輸入框內執行）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxAction {
    /// Enter 送出翻譯。
    Submit,
    /// Esc 或空白 Enter 關閉。
    Close,
}

/// 翻譯輸入框的狀態機。
///
/// 每次開關 session +1，背景翻譯的結果對不上號就丟掉；關閉時進行中的請求一併 cancel。
/// `close()` 是使用者主動關或已送出，未送出的文字一併丟掉；`hide()` 是遊戲關了聊天框
/// 被動收起，文字留到下次 `show()`。
pub struct InputBox {
    translate: Box<dyn Fn(OutgoingRequest)>,
    send: Box<dyn Fn(String, isize)>,
    open_flag: Arc<AtomicBool>,
    foreground: fn() -> isize,
    cursor: fn() -> (i32, i32),
    anchor: Option<Rect>,
    open: bool,
    target: isize,
    session: u64,
    cancel: Option<CancellationToken>,
    text: String,
    translating: bool,
    error: Option<String>,
    composing: bool,
    focus_request: bool,
    restore_foreground: Option<isize>,
    placement: (Option<Rect>, (i32, i32)),
}

impl InputBox {
    /// `translate` 收發話請求（不可阻塞）；`send` 把譯文打進 hwnd 指定的遊戲視窗。
    pub fn new(
        translate: Box<dyn Fn(OutgoingRequest)>,
        send: Box<dyn Fn(String, isize)>,
        open_flag: Arc<AtomicBool>,
        foreground: fn() -> isize,
        cursor: fn() -> (i32, i32),
    ) -> Self {
        Self {
            translate,
            send,
            open_flag,
            foreground,
            cursor,
            anchor: None,
            open: false,
            target: 0,
            session: 0,
            cancel: None,
            text: String::new(),
            translating: false,
            error: None,
            composing: false,
            focus_request: false,
            restore_foreground: None,
            placement: (None, (0, 0)),
        }
    }

    /// 輸入框是否開著。
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 呼出當下記住的前景遊戲視窗（譯文要打回去的那個）；未呼出過為 0。
    pub fn target_hwnd(&self) -> isize {
        self.target
    }

    /// 使用者主動關閉或已送出：丟掉未送出的文字。
    pub fn close(&mut self) {
        self.text.clear();
        self.dismiss();
    }

    /// 翻譯結果回到 UI 執行緒：成功就關閉並打進遊戲，失敗或超長就顯示錯誤、留著讓使用者改。
    pub fn finish(&mut self, session: u64, result: Result<String, String>) {
        if session != self.session || !self.open {
            return;
        }
        self.translating = false;
        let translated = match result {
            Ok(translated) => translated,
            Err(message) => {
                self.error = Some(message);
                return;
            }
        };
        let chars = translated.chars().count();
        if chars > GAME_INPUT_MAX_CHARS {
            log(format!(
                "[input] translation too long for the game (chars={chars}, limit={GAME_INPUT_MAX_CHARS})"
            ));
            let (count, limit) = (chars.to_string(), GAME_INPUT_MAX_CHARS.to_string());
            self.error = Some(t("input.too_long", &[("count", &count), ("limit", &limit)]));
            return;
        }
        let target = self.target;
        self.close();
        (self.send)(translated, target);
    }

    /// 畫輸入欄與狀態列，處理 Enter／Esc；回傳這一幀觸發的動作。
    pub fn content(&mut self, ui: &mut Ui) -> Option<BoxAction> {
        // 組字中的 Enter 是選字、Esc 是取消組字，不可送出或關窗；依事件順序追蹤組字狀態
        let composing = &mut self.composing;
        ui.input_mut(|i| {
            i.events.retain(|event| match event {
                Event::Ime(ImeEvent::Preedit { text, .. }) => {
                    *composing = !text.is_empty();
                    true
                }
                Event::Ime(ImeEvent::Commit(_)) => {
                    *composing = false;
                    true
                }
                Event::Key {
                    key: Key::Enter | Key::Escape,
                    ..
                } => !*composing,
                _ => true,
            });
        });
        let mut action = None;
        egui::Frame::new()
            .fill(BG)
            .inner_margin(Margin {
                left: 8,
                right: 8,
                top: 10,
                bottom: 8,
            })
            .show(ui, |ui| {
                let entry = TextEdit::singleline(&mut self.text)
                    .font(FontId::proportional(tk_pt(12.0)))
                    .text_color(FG)
                    .background_color(ENTRY_BG)
                    .desired_width(f32::INFINITY);
                let response = ui.add_enabled(!self.translating, entry);
                ui.ctx().accesskit_node_builder(response.id, |node| {
                    node.set_label(t("input.hint", &[]));
                });
                ui.add_space(4.0);
                let (status, color) = self.status();
                ui.style_mut().override_font_id = Some(FontId::proportional(tk_pt(9.0)));
                ui.visuals_mut().hyperlink_color = FG_UPDATE;
                richtext::show(ui, &status, color);
                let (enter, escape) =
                    ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                if escape {
                    self.close();
                    action = Some(BoxAction::Close);
                } else if enter && !self.translating {
                    action = Some(self.submit());
                }
                // TextEdit 按 Enter 會交出焦點；框裡只有這一個欄位，一律拉回來
                if self.open && !self.translating && !response.has_focus() {
                    response.request_focus();
                }
            });
        action
    }

    /// `show()` 之後回一次 true：宿主要把視窗搶到前景。
    pub fn take_focus_request(&mut self) -> bool {
        std::mem::take(&mut self.focus_request)
    }

    /// 關閉後回一次 target：宿主要把前景還給它。
    pub fn take_restore_foreground(&mut self) -> Option<isize> {
        self.restore_foreground.take()
    }

    /// `show()` 當下的錨點與游標位置，給 `open_rect` 用。
    pub fn placement_anchor(&self) -> (Option<Rect>, (i32, i32)) {
        self.placement
    }

    fn submit(&mut self) -> BoxAction {
        // 壓縮所有空白（含貼上夾帶的換行）：輸入端也守住單行保證
        let text = self.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            self.close();
            return BoxAction::Close;
        }
        self.translating = true;
        self.error = None;
        let cancel = CancellationToken::new();
        self.cancel = Some(cancel.clone());
        (self.translate)(OutgoingRequest {
            text,
            target: self.target,
            session: self.session,
            cancel,
        });
        BoxAction::Submit
    }

    fn status(&self) -> (String, Color32) {
        match (&self.error, self.translating) {
            (Some(error), _) => (error.clone(), FG_ERROR),
            (None, true) => (t("input.translating", &[]), HINT_FG),
            (None, false) => (t("input.hint", &[]), HINT_FG),
        }
    }

    /// `close()` 與 `hide()` 共同的收尾：作廢進行中的翻譯，前景交還呼出時的視窗。
    fn dismiss(&mut self) {
        if !self.open {
            return;
        }
        self.open = false;
        self.open_flag.store(false, Ordering::SeqCst);
        self.session += 1;
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
        self.restore_foreground = Some(self.target);
    }
}

impl InputBoxControl for InputBox {
    fn set_anchor(&mut self, r: Rect) {
        self.anchor = Some(r);
    }

    fn clear_anchor(&mut self) {
        self.anchor = None;
    }

    fn retarget(&mut self, hwnd: isize) {
        if hwnd != 0 && hwnd != self.target {
            log(format!("[input] retarget {:#x} -> {hwnd:#x}", self.target));
            self.target = hwnd;
        }
    }

    fn show(&mut self) {
        self.focus_request = true;
        if self.open {
            return;
        }
        self.open = true;
        self.open_flag.store(true, Ordering::SeqCst);
        self.session += 1;
        self.target = (self.foreground)();
        let anchor = self
            .anchor
            .map_or_else(|| "None".to_owned(), |r| format!("{r:?}"));
        log(format!(
            "[input] box opened (target_hwnd={:#x}, anchor={anchor})",
            self.target
        ));
        self.placement = (self.anchor, (self.cursor)());
        self.translating = false;
        self.error = None;
        self.composing = false;
        self.restore_foreground = None;
        if !self.text.is_empty() {
            log(format!(
                "[input] draft restored (chars={})",
                self.text.chars().count()
            ));
        }
    }

    fn hide(&mut self) {
        if self.open && !self.text.is_empty() {
            log(format!(
                "[input] box hidden, draft kept (chars={})",
                self.text.chars().count()
            ));
        }
        self.dismiss();
    }
}

/// 開窗時的外框（實體像素）與「是否在錨點上方」。
///
/// 有錨點時與它同寬（不窄於 `MIN_WIDTH`），沒有就以游標為 0×0 錨點、寬 `DEFAULT_WIDTH`；
/// DIP 常數乘上 `scale`。
pub fn open_rect(
    anchor: Option<Rect>,
    cursor: (i32, i32),
    content_h: i32,
    scale: f32,
    area: Rect,
) -> (Rect, bool) {
    let px = |dip: f32| (dip * scale).round() as i32;
    let width = anchor.map_or(px(DEFAULT_WIDTH), |a| px(MIN_WIDTH).max(a.2));
    let anchor = anchor.unwrap_or((cursor.0, cursor.1, 0, 0));
    let (x, y) = anchored_position(anchor, width, content_h, area, px(ANCHOR_GAP));
    ((x, y, width, content_h), y < anchor.1)
}

/// 內容高度變了的新外框：在錨點上方時固定底邊往上長，否則固定頂邊。
pub fn refit(current: Rect, content_h: i32, above: bool) -> Rect {
    let (x, y, w, h) = current;
    let y = if above { y + h - content_h } else { y };
    (x, y, w, content_h)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::*;
    use crate::ui::fonts_egui;

    const AREA: Rect = (0, 0, 1920, 1040);
    const CURSOR: (i32, i32) = (640, 500);

    type Sent = Rc<RefCell<Vec<(String, isize)>>>;
    type Requests = Rc<RefCell<Vec<OutgoingRequest>>>;

    struct Fixture {
        input: InputBox,
        requests: Requests,
        sent: Sent,
        open_flag: Arc<AtomicBool>,
    }

    fn fixture() -> Fixture {
        let requests = Requests::default();
        let sent = Sent::default();
        let open_flag = Arc::new(AtomicBool::new(false));
        let (r, s) = (requests.clone(), sent.clone());
        let input = InputBox::new(
            Box::new(move |req| r.borrow_mut().push(req)),
            Box::new(move |text, hwnd| s.borrow_mut().push((text, hwnd))),
            open_flag.clone(),
            || 0xA,
            || CURSOR,
        );
        Fixture {
            input,
            requests,
            sent,
            open_flag,
        }
    }

    /// 打字後按 Enter：回傳送出的那次請求的 session。
    fn submit(f: &mut Fixture, text: &str) -> u64 {
        f.input.text = text.into();
        assert_eq!(f.input.submit(), BoxAction::Submit);
        f.requests.borrow().last().unwrap().session
    }

    #[test]
    fn stale_session_discarded_on_cancel() {
        let mut f = fixture();
        f.input.show();
        let session = submit(&mut f, "hi");
        f.input.close();
        f.input.finish(session, Ok("english text".into()));
        assert!(f.sent.borrow().is_empty());
    }

    #[test]
    fn finish_over_limit_keeps_window_and_blocks_send() {
        let mut f = fixture();
        f.input.show();
        let session = submit(&mut f, "hi");
        let long = "é".repeat(GAME_INPUT_MAX_CHARS + 1);
        f.input.finish(session, Ok(long));
        assert!(f.input.is_open());
        assert!(f.sent.borrow().is_empty());
        let count = (GAME_INPUT_MAX_CHARS + 1).to_string();
        let limit = GAME_INPUT_MAX_CHARS.to_string();
        let expected = t("input.too_long", &[("count", &count), ("limit", &limit)]);
        assert_eq!(f.input.status(), (expected, FG_ERROR));
        assert!(!f.input.translating);
    }

    #[test]
    fn finish_within_limit_sends_and_closes() {
        let mut f = fixture();
        f.input.show();
        let session = submit(&mut f, "hi");
        let text = "é".repeat(GAME_INPUT_MAX_CHARS);
        f.input.finish(session, Ok(text.clone()));
        assert!(!f.input.is_open());
        assert_eq!(*f.sent.borrow(), [(text, 0xA)]);
    }

    #[test]
    fn enter_on_empty_input_closes_window() {
        let mut f = fixture();
        f.input.show();
        f.input.text = " \t ".into();
        assert_eq!(f.input.submit(), BoxAction::Close);
        assert!(!f.input.is_open());
        assert!(f.requests.borrow().is_empty());
    }

    #[test]
    fn non_cancelled_translation_succeeds() {
        let mut f = fixture();
        f.input.show();
        let session = submit(&mut f, "hi");
        f.input.finish(session, Ok("english text".into()));
        assert_eq!(*f.sent.borrow(), [("english text".to_owned(), 0xA)]);
    }

    #[test]
    fn stale_error_discarded_on_cancel() {
        let mut f = fixture();
        f.input.show();
        let session = submit(&mut f, "hi");
        f.input.hide();
        f.input.show();
        f.input.finish(session, Err("Error message".into()));
        assert_eq!(f.input.status(), (t("input.hint", &[]), HINT_FG));
    }

    #[test]
    fn current_error_shown_on_error() {
        let mut f = fixture();
        f.input.show();
        let session = submit(&mut f, "hi");
        assert!(f.input.translating);
        f.input.finish(session, Err("boom".into()));
        assert_eq!(f.input.status(), ("boom".to_owned(), FG_ERROR));
        assert!(!f.input.translating);
        assert!(f.input.is_open());
    }

    #[test]
    fn is_open_tracks_window_lifecycle() {
        let mut f = fixture();
        assert!(!f.input.is_open());
        f.input.show();
        assert!(f.input.is_open() && f.open_flag.load(Ordering::SeqCst));
        f.input.hide();
        assert!(!f.input.is_open() && !f.open_flag.load(Ordering::SeqCst));
        f.input.show();
        f.input.close();
        assert!(!f.input.is_open() && !f.open_flag.load(Ordering::SeqCst));
    }

    #[test]
    fn retarget_changes_target_hwnd_while_open() {
        let mut f = fixture();
        f.input.show();
        assert_eq!(f.input.target_hwnd(), 0xA);
        f.input.retarget(0xB);
        assert_eq!(f.input.target_hwnd(), 0xB);
        let session = submit(&mut f, "hi");
        f.input.finish(session, Ok("x".into()));
        assert_eq!(*f.sent.borrow(), [("x".to_owned(), 0xB)]);
    }

    #[test]
    fn retarget_is_a_no_op_for_none_or_same_hwnd() {
        let mut f = fixture();
        f.input.show();
        f.input.retarget(0);
        f.input.retarget(0xA);
        assert_eq!(f.input.target_hwnd(), 0xA);
    }

    #[test]
    fn hide_keeps_the_unsent_draft_for_the_next_show() {
        let mut f = fixture();
        f.input.show();
        f.input.text = "half typed".into();
        f.input.hide();
        f.input.show();
        assert_eq!(f.input.text, "half typed");
    }

    #[test]
    fn manual_close_discards_the_draft() {
        let mut f = fixture();
        f.input.show();
        f.input.text = "half typed".into();
        f.input.close();
        f.input.show();
        assert_eq!(f.input.text, "");
    }

    #[test]
    fn sent_text_is_not_restored() {
        let mut f = fixture();
        f.input.show();
        let session = submit(&mut f, "hello");
        f.input.finish(session, Ok("hello".into()));
        f.input.show();
        assert_eq!(f.input.text, "");
    }

    #[test]
    fn hide_while_translating_restores_an_editable_draft() {
        let mut f = fixture();
        f.input.show();
        submit(&mut f, "still typing");
        f.input.hide();
        f.input.show();
        assert_eq!(f.input.text, "still typing");
        assert!(!f.input.translating);
        assert_eq!(f.input.status(), (t("input.hint", &[]), HINT_FG));
    }

    #[test]
    fn closing_the_box_while_translating_cancels_the_request() {
        let mut f = fixture();
        f.input.show();
        let session = submit(&mut f, "哈囉");
        assert!(!f.requests.borrow()[0].cancel.is_cancelled());
        f.input.close();
        assert!(f.requests.borrow()[0].cancel.is_cancelled());
        f.input.finish(session, Ok("hello".into()));
        assert!(f.sent.borrow().is_empty());
    }

    #[test]
    fn submit_collapses_whitespace() {
        let mut f = fixture();
        f.input.show();
        submit(&mut f, "  a \n b ");
        let requests = f.requests.borrow();
        assert_eq!(requests[0].text, "a b");
        assert_eq!(requests[0].target, 0xA);
        assert!(f.input.translating);
        assert_eq!(f.input.status(), (t("input.translating", &[]), HINT_FG));
    }

    #[test]
    fn show_and_close_hand_focus_requests_to_the_host() {
        let mut f = fixture();
        f.input.show();
        assert!(f.input.take_focus_request());
        assert!(!f.input.take_focus_request());
        assert_eq!(f.input.take_restore_foreground(), None);
        f.input.close();
        assert_eq!(f.input.take_restore_foreground(), Some(0xA));
        assert_eq!(f.input.take_restore_foreground(), None);
    }

    // 幾何

    #[test]
    fn anchored_position_sits_below_the_anchor() {
        assert_eq!(
            anchored_position((749, 893, 732, 44), 460, 84, AREA, 4),
            (749, 941)
        );
    }

    #[test]
    fn anchored_position_flips_above_when_no_room_below() {
        assert_eq!(
            anchored_position((749, 1000, 732, 44), 460, 84, AREA, 4),
            (749, 912)
        );
    }

    #[test]
    fn anchored_position_clamps_into_the_work_area() {
        assert_eq!(
            anchored_position((1800, -50, 732, 44), 460, 84, AREA, 4),
            (1460, 0)
        );
    }

    /// `show()` 後依記下的錨點與游標算出的開窗外框。
    fn opened(anchor: Option<Rect>, content_h: i32) -> (Rect, bool) {
        let mut f = fixture();
        if let Some(r) = anchor {
            f.input.set_anchor(r);
        }
        f.input.show();
        let (anchor, cursor) = f.input.placement_anchor();
        open_rect(anchor, cursor, content_h, 1.0, AREA)
    }

    #[test]
    fn show_uses_the_anchor_width() {
        assert_eq!(
            opened(Some((300, 400, 732, 44)), 84),
            ((300, 448, 732, 84), false)
        );
    }

    #[test]
    fn show_clamps_the_anchor_width_to_minimum() {
        assert_eq!(opened(Some((300, 400, 200, 44)), 84).0.2, 320);
    }

    #[test]
    fn show_without_any_anchor_uses_the_default_width() {
        assert_eq!(opened(None, 84).0.2, 460);
    }

    #[test]
    fn show_without_anchor_sits_at_the_cursor() {
        assert_eq!(opened(None, 84), ((640, 504, 460, 84), false));
    }

    #[test]
    fn clear_anchor_makes_show_use_the_cursor() {
        let mut f = fixture();
        f.input.set_anchor((300, 400, 500, 40));
        f.input.clear_anchor();
        f.input.show();
        assert_eq!(f.input.placement_anchor(), (None, CURSOR));
    }

    #[test]
    fn anchor_set_while_open_applies_on_the_next_show() {
        let mut f = fixture();
        f.input.show();
        f.input.set_anchor((300, 400, 500, 40));
        assert_eq!(f.input.placement_anchor(), (None, CURSOR));
        f.input.close();
        f.input.show();
        assert_eq!(f.input.placement_anchor().0, Some((300, 400, 500, 40)));
    }

    #[test]
    fn box_placed_above_the_anchor_grows_upward() {
        let (rect, above) = opened(Some((300, 1000, 500, 40)), 84);
        assert_eq!((rect, above), ((300, 912, 500, 84), true));
        assert_eq!(refit(rect, 120, above), (300, 876, 500, 120));
        assert_eq!(refit(rect, 120, false), (300, 912, 500, 120));
    }

    #[test]
    fn growth_after_dragging_pins_the_dragged_bottom() {
        let (rect, above) = opened(Some((300, 1000, 500, 40)), 84);
        let dragged = (rect.0 + 30, rect.1 - 200, rect.2, rect.3);
        let grown = refit(dragged, 150, above);
        assert_eq!(grown.1 + grown.3, dragged.1 + dragged.3);
        assert_eq!((grown.0, grown.2, grown.3), (330, 500, 150));
    }

    #[test]
    fn width_follows_the_anchor_in_physical_pixels_at_150_percent() {
        let area = (0, 0, 3840, 2160);
        let width = |anchor| open_rect(anchor, CURSOR, 100, 1.5, area).0.2;
        assert_eq!(width(Some((0, 0, 900, 60))), 900);
        assert_eq!(width(Some((0, 0, 300, 60))), 480);
        assert_eq!(width(None), 690);
        let (rect, _) = open_rect(None, CURSOR, 100, 1.5, area);
        assert_eq!((rect.0, rect.1), (640, 506));
    }

    // 內容

    struct State {
        f: Fixture,
        actions: Vec<BoxAction>,
    }

    fn harness(f: Fixture) -> Harness<'static, State> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(460.0, 160.0))
            .with_context_setup(|ctx| fonts_egui::install(ctx, "en-US"))
            .build_ui_state(
                |ui, s: &mut State| s.actions.extend(s.f.input.content(ui)),
                State {
                    f,
                    actions: Vec::new(),
                },
            );
        harness.run_steps(2);
        harness
    }

    fn opened_harness() -> Harness<'static, State> {
        let mut f = fixture();
        f.input.show();
        harness(f)
    }

    fn type_text(harness: &mut Harness<State>, text: &str) {
        harness.event(Event::Text(text.into()));
        harness.step();
    }

    #[test]
    fn enter_submits_and_escape_closes() {
        let mut harness = opened_harness();
        type_text(&mut harness, "hello");
        harness.key_press(Key::Enter);
        harness.step();
        assert_eq!(harness.state().actions, [BoxAction::Submit]);
        assert_eq!(harness.state().f.requests.borrow()[0].text, "hello");
        harness.key_press(Key::Escape);
        harness.step();
        assert_eq!(
            harness.state().actions,
            [BoxAction::Submit, BoxAction::Close]
        );
        assert!(!harness.state().f.input.is_open());
    }

    #[test]
    fn entry_is_disabled_while_translating() {
        let mut harness = opened_harness();
        let entry = |h: &Harness<State>| {
            h.get_by_role(egui::accesskit::Role::TextInput)
                .accesskit_node()
                .is_disabled()
        };
        assert!(!entry(&harness));
        type_text(&mut harness, "hello");
        harness.key_press(Key::Enter);
        harness.run_steps(2);
        assert!(entry(&harness));
        harness.get_by_label(&t("input.translating", &[]));
        let session = harness.state().f.requests.borrow()[0].session;
        harness
            .state_mut()
            .f
            .input
            .finish(session, Err("boom".into()));
        harness.run_steps(2);
        assert!(!entry(&harness));
    }

    #[test]
    fn error_message_urls_are_clickable() {
        let mut harness = opened_harness();
        type_text(&mut harness, "hello");
        harness.key_press(Key::Enter);
        harness.step();
        let session = harness.state().f.requests.borrow()[0].session;
        harness.state_mut().f.input.finish(
            session,
            Err("HTTP 401: see https://a.example/keys for a key".into()),
        );
        harness.run_steps(2);
        harness.get_by_role(egui::accesskit::Role::Link);
        harness.get_by_label("https://a.example/keys");
    }

    #[test]
    fn ime_enter_commits_composition_without_submitting() {
        let preedit = || {
            Event::Ime(ImeEvent::Preedit {
                text: "ㄋㄧˇ".into(),
                active_range_chars: None,
            })
        };
        let mut harness = opened_harness();
        #[allow(deprecated)]
        harness.event(Event::Ime(ImeEvent::Enabled));
        harness.event(preedit());
        harness.step();
        harness.key_press(Key::Enter);
        harness.step();
        assert!(harness.state().actions.is_empty());
        harness.event(Event::Ime(ImeEvent::Commit("你".into())));
        harness.step();
        harness.key_press(Key::Enter);
        harness.step();
        assert_eq!(harness.state().actions, [BoxAction::Submit]);
        assert_eq!(harness.state().f.requests.borrow()[0].text, "你");

        let mut harness = opened_harness();
        harness.event(preedit());
        harness.step();
        harness.key_press(Key::Escape);
        harness.step();
        assert!(harness.state().actions.is_empty());
        assert!(harness.state().f.input.is_open());
    }
}
