//! overlay 訊息的框選：字元級、可跨訊息；拖出視口自動捲動、Ctrl+C 與右鍵「複製」選單。
//!
//! 端點以「第幾則」定位，訊息被移除時以 seq 重新對位（見 `Selection::sync`）；反白由 `MessageList` 繪製。

use std::time::Duration;

use eframe::egui::{Id, Pos2, Rect, Ui, Vec2};

use crate::logs::log;
use crate::ui::clipboard;
use crate::ui::copy_menu::{self, MenuOutcome};
use crate::ui::overlay::frame::{BAR_HEIGHT, Win32Shell, edge_under};
use crate::ui::overlay::message_list::{MessageList, Row};
use crate::ui::overlay::model::{Message, OverlayModel, ViewKey, fingerprint};
use crate::ui::overlay::outlined::{TEXT_ORIGIN, line_height};
use crate::ui::text_select::{AUTOSCROLL_INTERVAL, autoscroll_pixels, char_range_text, is_copy};
use crate::ui::win32;

/// 選取端點：第幾則訊息、該則第幾行（0＝原文、1＝譯文）、行內第幾個字元；欄位順序即閱讀順序。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Caret {
    pub msg: usize,
    pub line: usize,
    pub index: usize,
}

/// 第 `msg` 則第 `line` 行落在選取範圍內的字元區間 `[lo, hi)`（`hi` 可能超過行長，代表到行尾）。
pub fn line_range((start, end): (Caret, Caret), msg: usize, line: usize) -> Option<(usize, usize)> {
    let at = (msg, line);
    if at < (start.msg, start.line) || at > (end.msg, end.line) {
        return None;
    }
    let lo = if at == (start.msg, start.line) {
        start.index
    } else {
        0
    };
    let hi = if at == (end.msg, end.line) {
        end.index
    } else {
        usize::MAX
    };
    Some((lo, hi))
}

/// 選取範圍內的文字：同一則訊息的行以換行相接，訊息之間空一行 —— 貼出去才看得出哪幾句原本是同一則。
pub fn selected_text(messages: &[Vec<String>], start: Caret, end: Caret) -> String {
    if start >= end {
        return String::new();
    }
    (start.msg..=end.msg)
        .map(|msg| {
            let lines = messages[msg].iter().enumerate().filter_map(|(line, text)| {
                let (lo, hi) = line_range((start, end), msg, line)?;
                Some(char_range_text(text, lo, hi))
            });
            lines.collect::<Vec<_>>().join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 框選對桌面的操作；測試換成假的。
pub trait Desktop {
    /// 讓 overlay 取得鍵盤焦點，Ctrl+C 才收得到；失敗回 `false`。
    fn take_focus(&mut self) -> bool;
    /// 把文字寫進系統剪貼簿；失敗回 `false`。
    fn copy(&mut self, text: &str) -> bool;
}

impl Desktop for Win32Shell {
    fn take_focus(&mut self) -> bool {
        win32::force_foreground(self.0)
    }
    fn copy(&mut self, text: &str) -> bool {
        clipboard::set_text(self.0, text)
    }
}

/// 顯示中分頁的選取狀態、拖曳中的自動捲動與右鍵選單。
#[derive(Default)]
pub struct Selection {
    anchor: Option<Caret>,
    focus: Option<Caret>,
    /// anchor、focus 所在訊息的 seq：前方訊息被移除時據此重新對位。
    seqs: [u64; 2],
    /// 選取範圍內訊息的內容雜湊：範圍內的譯文被回填就清掉選取。
    content: u64,
    key: Option<ViewKey>,
    dragging: bool,
    next_tick: f64,
    /// 右鍵選單的位置（視窗內 DIP）。
    menu: Option<Pos2>,
    menu_rect: Option<Rect>,
}

impl Selection {
    /// 正規化後的 `(start, end)`；沒有選取或選取為空則 None。
    pub fn span(&self) -> Option<(Caret, Caret)> {
        match (self.anchor, self.focus) {
            (Some(a), Some(f)) if a != f => Some((a.min(f), a.max(f))),
            _ => None,
        }
    }

    /// 是否正在拖曳選取。
    pub fn dragging(&self) -> bool {
        self.dragging
    }

    /// 目前選取到的文字；`messages` 是顯示中分頁的訊息。
    pub fn text(&self, messages: &[Message]) -> String {
        let Some((start, end)) = self.span() else {
            return String::new();
        };
        let Some(span) = messages.get(start.msg..=end.msg) else {
            return String::new();
        };
        let texts: Vec<Vec<String>> = span
            .iter()
            .map(|m| vec![m.original.clone(), m.translated.clone()])
            .collect();
        let rebase = |c: Caret| Caret {
            msg: c.msg - start.msg,
            ..c
        };
        selected_text(&texts, rebase(start), rebase(end))
    }

    /// 框選起手：清掉舊選取，`pos` 落在可見訊息的某一行才開始拖曳並回 `true`。
    pub fn begin(&mut self, list: &MessageList, messages: &[Message], pos: Pos2) -> bool {
        let caret = hit(list, messages, pos);
        self.clear("new press");
        let Some(caret) = caret else {
            return false;
        };
        self.anchor = Some(caret);
        self.focus = Some(caret);
        self.dragging = true;
        self.remember(messages);
        log(format!(
            "[ui] selection begin message={} line={} index={}",
            caret.msg, caret.line, caret.index
        ));
        true
    }

    /// 拖曳中：focus 移到 `pos`，拖出全部可見訊息的上下界就夾到最前／最後。
    pub fn extend(&mut self, list: &MessageList, messages: &[Message], pos: Pos2) {
        if !self.dragging {
            return;
        }
        if let Some(caret) = clamped(list, messages, pos) {
            self.focus = Some(caret);
            self.remember(messages);
        }
    }

    /// 放開滑鼠。
    pub fn finish(&mut self, messages: &[Message]) {
        if !self.dragging {
            return;
        }
        self.dragging = false;
        let Some((start, end)) = self.span() else {
            log("[ui] selection end empty");
            return;
        };
        log(format!(
            "[ui] selection end start={}:{}:{} end={}:{}:{} chars={}",
            start.msg,
            start.line,
            start.index,
            end.msg,
            end.line,
            end.index,
            self.text(messages).chars().count()
        ));
    }

    /// 清除選取；`reason` 只進 log，方便事後定位「選取莫名消失」。
    pub fn clear(&mut self, reason: &str) {
        if self.anchor.is_none() {
            return;
        }
        self.anchor = None;
        self.focus = None;
        self.dragging = false;
        log(format!("[ui] selection cleared ({reason})"));
    }

    /// 每幀在 `list.show` 之前呼叫：處理按下／拖曳／放開、自動捲動、Ctrl+C 與右鍵選單。
    ///
    /// 直接讀指標而不註冊 egui 元件：元件的命中半徑會吃掉視窗邊緣的縮放帶；`ui` 要是整個視窗的 Ui。
    pub fn show(
        &mut self,
        ui: &Ui,
        list: &mut MessageList,
        model: &OverlayModel,
        desktop: &mut dyn Desktop,
    ) {
        let messages = model.current_view().map_or(&[][..], |v| &v.messages);
        self.sync(model.current_key(), messages);
        self.show_menu(ui, messages, desktop);

        let area = ui.max_rect();
        let (pressed, secondary, down, pos, time, copy) = ui.input(|i| {
            let p = &i.pointer;
            (
                p.primary_pressed().then(|| p.press_origin()).flatten(),
                p.secondary_pressed().then(|| p.press_origin()).flatten(),
                p.primary_down(),
                p.latest_pos(),
                i.time,
                i.events.iter().any(is_copy),
            )
        });
        if let Some(p) = pressed
            && !self.menu_rect.is_some_and(|r| r.contains(p))
        {
            self.close_menu();
            if list.viewport().contains(p)
                && edge_under(area, p).is_none()
                && self.begin(list, messages, p)
            {
                log("[ui] forcing keyboard focus");
                if desktop.take_focus() {
                    log("[ui] selection took keyboard focus");
                } else {
                    log("[ui] selection focus failed: SetForegroundWindow refused");
                }
                self.next_tick = time + AUTOSCROLL_INTERVAL;
            }
        }
        if self.dragging {
            if let Some(p) = pos {
                if time >= self.next_tick {
                    self.next_tick = time + AUTOSCROLL_INTERVAL;
                    let viewport = list.viewport();
                    let step = autoscroll_pixels(
                        p.y.floor() as i32,
                        viewport.min.y as i32,
                        viewport.max.y as i32 - 1,
                    );
                    // 視圖往上捲＝內容往下移
                    list.scroll_by(-step as f32);
                }
                self.extend(list, messages, p);
                ui.ctx()
                    .request_repaint_after(Duration::from_secs_f64(AUTOSCROLL_INTERVAL));
            }
            if !down {
                self.finish(messages);
            }
        }
        list.set_dragging(self.dragging);
        if let Some(p) = secondary
            && area.contains(p)
            && p.y >= area.min.y + BAR_HEIGHT
            && self.span().is_some()
        {
            self.menu = Some(p);
        }
        if copy {
            self.copy(messages, desktop);
        }
    }

    /// 把選取的文字寫進剪貼簿；沒有選取就不動 —— 寫入空字串會清掉使用者原本的剪貼簿內容。
    fn copy(&self, messages: &[Message], desktop: &mut dyn Desktop) {
        let text = self.text(messages);
        if !text.is_empty() && desktop.copy(&text) {
            log(format!(
                "[ui] copied selection chars={}",
                text.chars().count()
            ));
        }
    }

    /// 換分頁就清掉選取；前方訊息被移除時兩端重新對位，端點那則被移除或範圍內譯文被回填則清掉。
    fn sync(&mut self, key: Option<ViewKey>, messages: &[Message]) {
        if key != self.key {
            self.key = key;
            self.clear("view switched");
        }
        let (Some(anchor), Some(focus)) = (self.anchor, self.focus) else {
            return;
        };
        let find = |seq| messages.binary_search_by_key(&seq, |m| m.seq).ok();
        let (Some(a), Some(f)) = (find(self.seqs[0]), find(self.seqs[1])) else {
            self.clear("row destroyed");
            return;
        };
        self.anchor = Some(Caret { msg: a, ..anchor });
        self.focus = Some(Caret { msg: f, ..focus });
        if self.span().is_some() && fingerprint(&messages[a.min(f)..=a.max(f)]) != self.content {
            self.clear("translated line replaced");
        }
    }

    fn remember(&mut self, messages: &[Message]) {
        if let (Some(a), Some(f)) = (self.anchor, self.focus) {
            self.seqs = [messages[a.msg].seq, messages[f.msg].seq];
            self.content = fingerprint(&messages[a.msg.min(f.msg)..=a.msg.max(f.msg)]);
        }
    }

    fn close_menu(&mut self) {
        self.menu_rect = None;
        if self.menu.take().is_some() {
            log("[ui] copy popup hidden");
        }
    }

    /// 右鍵選單：點「複製」複製選取並關閉，在選單外按下就關閉。
    fn show_menu(&mut self, ui: &Ui, messages: &[Message], desktop: &mut dyn Desktop) {
        if self.span().is_none() {
            self.close_menu();
        }
        let Some(at) = self.menu else {
            return;
        };
        match copy_menu::show(ui, Id::unique("overlay_copy_menu"), at) {
            MenuOutcome::Open => self.menu_rect = Some(copy_menu::rect(ui, at)),
            MenuOutcome::Copy => {
                self.close_menu();
                self.copy(messages, desktop);
            }
            MenuOutcome::Dismissed => self.close_menu(),
        }
    }
}

/// 上一幀畫出、且與視口相交的訊息列與它在 `messages` 中的索引，由上到下。
fn visible<'a>(list: &'a MessageList, messages: &[Message]) -> Vec<(usize, &'a Row)> {
    let viewport = list.viewport();
    list.rows()
        .iter()
        .filter(|r| {
            let (origin, galley) = &r.lines[1];
            r.lines[0].0.y < viewport.max.y && origin.y + line_height(galley) > viewport.min.y
        })
        .filter_map(|r| {
            let msg = messages.binary_search_by_key(&r.seq, |m| m.seq).ok()?;
            Some((msg, r))
        })
        .collect()
}

fn caret_in(row: &Row, msg: usize, line: usize, pos: Pos2) -> Caret {
    let (origin, galley) = &row.lines[line];
    let local = pos - *origin - Vec2::splat(TEXT_ORIGIN);
    Caret {
        msg,
        line,
        index: galley.cursor_from_pos(local).index.0,
    }
}

/// `pos` 落在哪一行的哪個字元；不在視口內或不在任何一行上回 None。
fn hit(list: &MessageList, messages: &[Message], pos: Pos2) -> Option<Caret> {
    if !list.viewport().contains(pos) {
        return None;
    }
    visible(list, messages).into_iter().find_map(|(msg, row)| {
        (0..2).find_map(|line| {
            let (origin, galley) = &row.lines[line];
            (origin.y..origin.y + line_height(galley))
                .contains(&pos.y)
                .then(|| caret_in(row, msg, line, pos))
        })
    })
}

/// 拖曳點換算成端點：垂直方向決定哪一則的哪一行，超出可見訊息上下界就夾到最前／最後；沒有可見訊息回 None。
fn clamped(list: &MessageList, messages: &[Message], pos: Pos2) -> Option<Caret> {
    let rows = visible(list, messages);
    let &(first, top) = rows.first()?;
    let &(last, _) = rows.last()?;
    if pos.y < top.lines[0].0.y {
        return Some(Caret {
            msg: first,
            line: 0,
            index: 0,
        });
    }
    for &(msg, row) in &rows {
        for line in 0..2 {
            let (origin, galley) = &row.lines[line];
            if pos.y < origin.y + line_height(galley) {
                return Some(caret_in(row, msg, line, pos));
            }
        }
    }
    Some(Caret {
        msg: last,
        line: 1,
        index: messages[last].translated.chars().count(),
    })
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use eframe::egui::{Event, Key, Modifiers, PointerButton, Shape};
    use egui_kittest::Harness;

    use super::*;
    use crate::i18n::t;
    use crate::ui::fonts_egui;
    use crate::ui::geometry::{EDGE, Edge};
    use crate::ui::overlay::frame::Frame;
    use crate::ui::overlay::frame::tests::FakeShell;
    use crate::ui::palette::{BAR, SELECT_BG};

    const SIZE: Vec2 = Vec2::new(460.0, 300.0);
    /// 假視窗的螢幕位置與大小；被縮放或移動過就會變。
    const WINDOW: (i32, i32, i32, i32) = (1000, 500, 460, 300);

    fn caret(msg: usize, line: usize, index: usize) -> Caret {
        Caret { msg, line, index }
    }

    #[derive(Default)]
    struct FakeDesktop {
        focused: usize,
        copied: Vec<String>,
    }

    impl Desktop for FakeDesktop {
        fn take_focus(&mut self) -> bool {
            self.focused += 1;
            true
        }
        fn copy(&mut self, text: &str) -> bool {
            self.copied.push(text.to_owned());
            true
        }
    }

    struct State {
        /// 外框與內容疊法同 overlay：邊上的按壓由外框拿去縮放。
        frame: Frame,
        shell: FakeShell,
        model: OverlayModel,
        list: MessageList,
        selection: Selection,
        desktop: FakeDesktop,
        t0: Instant,
        /// 整個視窗的 Ui 範圍（harness 四周有留白，不等於 `SIZE`）。
        area: Rect,
    }

    fn add(model: &mut OverlayModel, t0: Instant, i: u64, original: &str, translated: &str) {
        let at = t0 + Duration::from_secs(i);
        model.add_message(1, &[0], original, translated, Some(i), false, None, at);
    }

    fn harness(texts: &[(&str, &str)]) -> Harness<'static, State> {
        let t0 = Instant::now();
        let mut model = OverlayModel::new(200, 60);
        for (i, (original, translated)) in texts.iter().enumerate() {
            add(&mut model, t0, i as u64, original, translated);
        }
        let state = State {
            frame: Frame::new(1.0, None, |_, _, _, _| {}),
            shell: FakeShell {
                cursor: (WINDOW.0, WINDOW.1),
                rect: WINDOW,
                ready: true,
            },
            model,
            list: MessageList::new(),
            selection: Selection::default(),
            desktop: FakeDesktop::default(),
            t0,
            area: Rect::NOTHING,
        };
        let mut h = Harness::builder()
            .with_size(SIZE)
            .with_context_setup(|ctx| fonts_egui::install(ctx, "zh-TW"))
            .build_ui_state(
                |ui, s: &mut State| {
                    let area = ui.max_rect();
                    s.area = area;
                    s.frame.show(ui, &mut s.shell, "t", None, "");
                    let rect = Rect::from_min_max(area.min + Vec2::new(0.0, BAR_HEIGHT), area.max);
                    s.selection.show(ui, &mut s.list, &s.model, &mut s.desktop);
                    s.list.show(ui, rect, &s.model, None, s.selection.span());
                },
                state,
            );
        h.step();
        h.step();
        h
    }

    fn numbered(count: u64) -> Harness<'static, State> {
        let texts: Vec<(String, String)> = (0..count)
            .map(|i| (format!("orig {i}"), format!("trans {i}")))
            .collect();
        let refs: Vec<(&str, &str)> = texts.iter().map(|(o, t)| (&o[..], &t[..])).collect();
        harness(&refs)
    }

    fn messages<'a>(h: &'a Harness<State>) -> &'a [Message] {
        &h.state().model.current_view().unwrap().messages
    }

    fn text(h: &Harness<State>) -> String {
        h.state().selection.text(messages(h))
    }

    /// 第 `msg` 則（畫在畫面上）第 `line` 行、行原點右方 `dx` 處的垂直中線點。
    fn at(h: &Harness<State>, msg: usize, line: usize, dx: f32) -> Pos2 {
        let seq = messages(h)[msg].seq;
        let row = h.state().list.rows().iter().find(|r| r.seq == seq).unwrap();
        let (origin, galley) = &row.lines[line];
        Pos2::new(origin.x + dx, origin.y + line_height(galley) / 2.0)
    }

    /// 行首字元左緣的點。
    fn start_of(h: &Harness<State>, msg: usize, line: usize) -> Pos2 {
        at(h, msg, line, TEXT_ORIGIN)
    }

    /// 該行右側、仍在視口內的點（夾到行尾）。
    fn end_of(h: &Harness<State>, msg: usize, line: usize) -> Pos2 {
        let right = h.state().list.viewport().max.x - 1.0;
        Pos2::new(right, at(h, msg, line, 0.0).y)
    }

    /// 假游標跟著指標換算成螢幕座標：外框若在縮放，視窗大小就會跟著變。
    fn point(h: &mut Harness<State>, p: Pos2) {
        let v = p - h.state().area.min;
        h.state_mut().shell.cursor = (WINDOW.0 + v.x as i32, WINDOW.1 + v.y as i32);
        h.hover_at(p);
    }

    fn press(h: &mut Harness<State>, p: Pos2) {
        point(h, p);
        h.drag_at(p);
        h.step();
    }

    fn move_to(h: &mut Harness<State>, p: Pos2) {
        point(h, p);
        h.step();
    }

    fn release(h: &mut Harness<State>, p: Pos2) {
        point(h, p);
        h.drop_at(p);
        h.step();
    }

    fn select(h: &mut Harness<State>, from: Pos2, to: Pos2) {
        press(h, from);
        move_to(h, to);
        release(h, to);
    }

    fn right_click(h: &mut Harness<State>, p: Pos2) {
        h.hover_at(p);
        for pressed in [true, false] {
            h.event(Event::PointerButton {
                pos: p,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        h.step();
    }

    fn highlights(h: &Harness<State>) -> Vec<Rect> {
        h.output()
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Rect(r) if r.fill == SELECT_BG => Some(r.rect),
                _ => None,
            })
            .collect()
    }

    fn row_origin(h: &Harness<State>, seq: u64) -> Option<Pos2> {
        let rows = h.state().list.rows();
        rows.iter().find(|r| r.seq == seq).map(|r| r.lines[0].0)
    }

    #[test]
    fn selected_text_matches_python() {
        let v = |rows: &[&[&str]]| -> Vec<Vec<String>> {
            rows.iter()
                .map(|m| m.iter().map(|s| s.to_string()).collect())
                .collect()
        };
        let one = v(&[&["abcdef", "xyz"]]);
        assert_eq!(selected_text(&one, caret(0, 0, 1), caret(0, 0, 4)), "bcd");
        assert_eq!(
            selected_text(&one, caret(0, 0, 4), caret(0, 1, 2)),
            "ef\nxy"
        );
        assert_eq!(selected_text(&one, caret(0, 0, 2), caret(0, 0, 2)), "");
        let three_lines = v(&[&["abc", "mid", "xyz"]]);
        assert_eq!(
            selected_text(&three_lines, caret(0, 0, 1), caret(0, 2, 2)),
            "bc\nmid\nxy"
        );
        let messages = v(&[&["a1", "a2"], &["b1", "b2"], &["c1", "c2"]]);
        assert_eq!(
            selected_text(&messages, caret(0, 1, 1), caret(2, 0, 1)),
            "2\n\nb1\nb2\n\nc"
        );
        let cjk = v(&[&["原文一", "譯文一"], &["原文二", "譯文二"]]);
        assert_eq!(
            selected_text(&cjk, caret(0, 0, 0), caret(1, 1, 3)),
            "原文一\n譯文一\n\n原文二\n譯文二"
        );
    }

    #[test]
    fn press_on_a_line_starts_a_selection_and_takes_focus() {
        let mut h = harness(&[("original text", "translated text")]);
        let p = start_of(&h, 0, 0);
        press(&mut h, p);
        assert!(h.state().selection.dragging());
        assert_eq!(h.state().desktop.focused, 1);
    }

    #[test]
    fn press_outside_every_message_starts_nothing() {
        let mut h = harness(&[("original text", "translated text")]);
        // 訊息下方的空白、視口右側的捲軸區
        let below = Pos2::new(200.0, h.state().area.max.y - 40.0);
        let right = Pos2::new(h.state().list.viewport().max.x + 3.0, at(&h, 0, 0, 0.0).y);
        for p in [below, right] {
            press(&mut h, p);
            assert!(!h.state().selection.dragging(), "{p:?}");
            release(&mut h, p);
        }
        assert_eq!(h.state().desktop.focused, 0);
    }

    #[test]
    fn a_bare_press_is_not_a_selection() {
        let mut h = harness(&[("original text", "translated text")]);
        let p = start_of(&h, 0, 0);
        press(&mut h, p);
        release(&mut h, p);
        assert!(!h.state().selection.dragging());
        assert_eq!(h.state().selection.span(), None);
        assert_eq!(text(&h), "");
    }

    #[test]
    fn dragging_across_both_lines_selects_both() {
        let mut h = harness(&[("original text", "translated text")]);
        let (from, to) = (start_of(&h, 0, 0), end_of(&h, 0, 1));
        select(&mut h, from, to);
        assert_eq!(text(&h), "original text\ntranslated text");
    }

    #[test]
    fn reverse_drag_normalizes_span() {
        let mut h = harness(&[("original text", "translated text")]);
        let (from, to) = (end_of(&h, 0, 1), start_of(&h, 0, 0));
        select(&mut h, from, to);
        assert_eq!(
            h.state().selection.span(),
            Some((caret(0, 0, 0), caret(0, 1, "translated text".len())))
        );
        assert_eq!(text(&h), "original text\ntranslated text");
    }

    #[test]
    fn dragging_past_the_messages_clamps_to_their_ends() {
        let mut h = harness(&[("original text", "translated text")]);
        let (first, last) = (start_of(&h, 0, 0), end_of(&h, 0, 1));
        select(&mut h, first, last + Vec2::new(0.0, 500.0));
        assert_eq!(text(&h), "original text\ntranslated text");
        select(&mut h, last, first - Vec2::new(0.0, 500.0));
        assert_eq!(text(&h), "original text\ntranslated text");
    }

    #[test]
    fn selection_spans_messages() {
        let mut h = harness(&[("orig one", "trans one"), ("orig two", "trans two")]);
        let (from, to) = (start_of(&h, 0, 0), end_of(&h, 1, 1));
        select(&mut h, from, to);
        assert_eq!(text(&h), "orig one\ntrans one\n\norig two\ntrans two");
    }

    #[test]
    fn selection_spans_three_messages_and_highlights_every_line_under_its_text() {
        let mut h = harness(&[
            ("orig one", "trans one"),
            ("orig two", "trans two"),
            ("orig three", "trans three"),
        ]);
        let (from, to) = (start_of(&h, 0, 0), end_of(&h, 2, 1));
        select(&mut h, from, to);
        assert_eq!(text(&h).matches("\n\n").count(), 2);
        assert!(text(&h).starts_with("orig one") && text(&h).ends_with("trans three"));
        let shapes = &h.output().shapes;
        let marks: Vec<usize> = shapes
            .iter()
            .enumerate()
            .filter(|(_, c)| matches!(&c.shape, Shape::Rect(r) if r.fill == SELECT_BG))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(marks.len(), 6, "one highlight per selected line");
        // 反白要壓在該行文字之下：緊接著就是該行的描邊字
        for i in marks {
            let Shape::Rect(mark) = &shapes[i].shape else {
                unreachable!()
            };
            let Shape::Text(next) = &shapes[i + 1].shape else {
                panic!("highlight {i} not followed by its text");
            };
            assert!((next.pos.y - mark.rect.min.y).abs() <= 1.0);
        }
    }

    #[test]
    fn prune_of_selected_message_clears_selection() {
        let mut h = numbered(3);
        let (from, to) = (start_of(&h, 0, 0), end_of(&h, 1, 1));
        select(&mut h, from, to);
        assert!(h.state().selection.span().is_some());
        let t0 = h.state().t0;
        h.state_mut().model.prune(t0 + Duration::from_secs(60));
        h.step();
        assert_eq!(h.state().selection.span(), None);
        assert!(highlights(&h).is_empty());
    }

    #[test]
    fn prune_before_the_selection_keeps_the_text() {
        let mut h = numbered(3);
        let (from, to) = (start_of(&h, 1, 0), end_of(&h, 2, 1));
        select(&mut h, from, to);
        let before = text(&h);
        assert_eq!(before, "orig 1\ntrans 1\n\norig 2\ntrans 2");
        let t0 = h.state().t0;
        h.state_mut().model.prune(t0 + Duration::from_secs(60));
        h.step();
        assert_eq!(messages(&h).len(), 2);
        assert_eq!(text(&h), before);
    }

    #[test]
    fn max_messages_overflow_clears_a_selection_in_the_dropped_row() {
        let mut h = numbered(2);
        let (from, to) = (start_of(&h, 0, 0), end_of(&h, 0, 1));
        select(&mut h, from, to);
        let t0 = h.state().t0;
        let model = &mut h.state_mut().model;
        model.set_limits(2, 60);
        add(model, t0, 2, "orig 2", "trans 2");
        h.step();
        assert_eq!(h.state().selection.span(), None);
    }

    #[test]
    fn switching_views_clears_the_selection() {
        use crate::reader::chat_tabs::TabState;
        let tabs = |current| TabState {
            tabs: vec![(0, "Main".into()), (1, "Group".into())],
            current,
        };
        for switch in ["tab", "client"] {
            let mut h = numbered(1);
            h.state_mut().model.set_tabs(1, Some(tabs(0)));
            h.step();
            let (from, to) = (start_of(&h, 0, 0), end_of(&h, 0, 1));
            select(&mut h, from, to);
            assert!(h.state().selection.span().is_some(), "{switch}");
            let model = &mut h.state_mut().model;
            match switch {
                "tab" => model.set_tabs(1, Some(tabs(1))),
                _ => model.set_foreground_slot(Some(2)),
            }
            h.step();
            assert_eq!(h.state().selection.span(), None, "{switch}");
        }
    }

    #[test]
    fn filling_a_selected_translation_clears_the_selection() {
        let mut h = numbered(2);
        let (from, to) = (start_of(&h, 0, 0), end_of(&h, 1, 1));
        select(&mut h, from, to);
        h.state_mut().model.update_message(1, "replaced", false);
        h.step();
        assert_eq!(h.state().selection.span(), None);
    }

    #[test]
    fn clear_removes_the_highlight_and_the_selection() {
        let mut h = numbered(1);
        let (from, to) = (start_of(&h, 0, 0), end_of(&h, 0, 1));
        select(&mut h, from, to);
        assert!(!highlights(&h).is_empty());
        h.state_mut().selection.clear("test");
        h.step();
        assert_eq!(text(&h), "");
        assert!(highlights(&h).is_empty());
    }

    #[test]
    fn selected_text_survives_scrolling_out_of_view_and_rewrapping() {
        let mut h = numbered(40);
        let last = messages(&h).len() - 1;
        let (from, to) = (start_of(&h, last - 1, 0), end_of(&h, last, 1));
        select(&mut h, from, to);
        let before = text(&h);
        h.state_mut().list.scroll_by(2000.0);
        h.step();
        assert!(
            row_origin(&h, 39).is_none(),
            "precondition: selection scrolled away"
        );
        assert_eq!(text(&h), before);
        h.set_size(Vec2::new(300.0, SIZE.y));
        h.step();
        assert_eq!(text(&h), before);
    }

    #[test]
    fn hit_testing_follows_the_scroll_position() {
        let mut h = numbered(40);
        let p = Pos2::new(
            h.state().list.viewport().min.x + 40.0,
            h.state().area.center().y,
        );
        let under = |h: &Harness<State>| {
            let rows = h.state().list.rows();
            let row = rows.iter().find(|r| {
                let (origin, galley) = &r.lines[1];
                r.lines[0].0.y <= p.y && p.y < origin.y + line_height(galley)
            });
            row.unwrap().seq
        };
        let expected = under(&h);
        select(&mut h, p, p + Vec2::new(500.0, 0.0));
        assert!(
            format!("orig {expected}").ends_with(&text(&h)),
            "{}",
            text(&h)
        );
        h.state_mut().list.scroll_by(200.0);
        h.step();
        let scrolled = under(&h);
        assert_ne!(scrolled, expected);
        select(&mut h, p, p + Vec2::new(500.0, 0.0));
        assert!(
            format!("orig {scrolled}").ends_with(&text(&h)),
            "{}",
            text(&h)
        );
    }

    #[test]
    fn rows_scrolled_out_of_view_are_not_hit() {
        let mut h = numbered(40);
        let viewport = h.state().list.viewport();
        // 視口上方（標題列一帶）按下不會選到捲出視野的訊息
        press(&mut h, Pos2::new(200.0, viewport.min.y - 5.0));
        assert!(!h.state().selection.dragging());
        assert_eq!(h.state().desktop.focused, 0);
    }

    #[test]
    fn press_in_a_resize_band_resizes_instead_of_selecting() {
        // 下緣帶、最新一則譯文那一行高度的左緣帶
        for edge in [Edge::S, Edge::W] {
            let mut h = numbered(40);
            let (area, last) = (h.state().area, messages(&h).len() - 1);
            let p = match edge {
                Edge::S => Pos2::new(200.0, area.max.y - EDGE as f32 / 2.0),
                _ => Pos2::new(area.min.x + EDGE as f32 / 2.0, start_of(&h, last, 1).y),
            };
            assert_eq!(edge_under(h.state().area, p), Some(edge), "precondition");
            press(&mut h, p);
            move_to(&mut h, p + Vec2::new(-20.0, 20.0));
            assert!(!h.state().selection.dragging(), "{edge:?}");
            assert_eq!(h.state().desktop.focused, 0, "{edge:?}");
            release(&mut h, p + Vec2::new(-20.0, 20.0));
            assert_ne!(h.state().shell.rect, WINDOW, "{edge:?} should resize");
        }
    }

    #[test]
    fn dragging_from_the_first_character_selects_instead_of_resizing() {
        let mut h = numbered(40);
        let last = messages(&h).len() - 1;
        // 最新一則最靠近左下角的縮放帶
        let (from, to) = (start_of(&h, last, 1), end_of(&h, last, 1));
        assert_eq!(edge_under(h.state().area, from), None);
        select(&mut h, from, to);
        assert_eq!(text(&h), format!("trans {last}"));
        assert_eq!(h.state().shell.rect, WINDOW);
    }

    #[test]
    fn dragging_above_the_viewport_scrolls_and_grows_the_selection() {
        let mut h = numbered(40);
        let last = messages(&h).len() - 1;
        let from = start_of(&h, last, 0);
        press(&mut h, from);
        let viewport = h.state().list.viewport();
        let above = Pos2::new(viewport.min.x + 40.0, viewport.min.y - 60.0);
        move_to(&mut h, above);
        let (first_seq, grown) = (h.state().list.rows()[0].seq, text(&h).len());
        for _ in 0..10 {
            h.step();
        }
        assert!(h.state().list.rows()[0].seq < first_seq, "should scroll up");
        assert!(
            text(&h).len() > grown,
            "selection should grow with the scroll"
        );
        release(&mut h, above);
        let stopped = h.state().list.rows()[0].seq;
        for _ in 0..5 {
            h.step();
        }
        assert_eq!(
            h.state().list.rows()[0].seq,
            stopped,
            "autoscroll must stop on release"
        );
    }

    #[test]
    fn releasing_keeps_the_view_until_new_content_arrives() {
        let mut h = numbered(40);
        let last = messages(&h).len() - 1;
        let (from, to) = (start_of(&h, last, 0), end_of(&h, last, 1));
        press(&mut h, from);
        move_to(&mut h, to);
        let t0 = h.state().t0;
        add(&mut h.state_mut().model, t0, 40, "orig 40", "trans 40");
        h.step();
        release(&mut h, to);
        let row = row_origin(&h, 39);
        for _ in 0..3 {
            h.step();
        }
        assert_eq!(
            row_origin(&h, 39),
            row,
            "view must stay put for the right-click copy"
        );
        assert_eq!(text(&h), "orig 39\ntrans 39");
        add(&mut h.state_mut().model, t0, 41, "orig 41", "trans 41");
        h.step();
        assert_eq!(h.state().list.rows().last().unwrap().seq, 41);
    }

    #[test]
    fn ctrl_c_with_caps_lock_copies() {
        let mut h = numbered(1);
        h.key_press_modifiers(Modifiers::CTRL, Key::C);
        h.step();
        assert!(
            h.state().desktop.copied.is_empty(),
            "nothing selected, clipboard untouched"
        );
        let (from, to) = (start_of(&h, 0, 0), end_of(&h, 0, 1));
        select(&mut h, from, to);
        // 大寫 C 以 Shift 模擬，到 egui 是帶 Shift 的 Key 事件；一般 Ctrl+C（含 Caps Lock）是 Copy 事件
        h.key_press_modifiers(Modifiers::CTRL | Modifiers::SHIFT, Key::C);
        h.step();
        h.event(Event::Copy);
        h.step();
        assert_eq!(h.state().desktop.copied, ["orig 0\ntrans 0"; 2]);
    }

    #[test]
    fn ctrl_c_copies_while_the_copy_menu_is_open() {
        let mut h = numbered(1);
        let (from, to) = (start_of(&h, 0, 0), end_of(&h, 0, 1));
        select(&mut h, from, to);
        right_click(&mut h, Pos2::new(200.0, 150.0));
        h.step();
        assert!(h.state().selection.menu_rect.is_some(), "menu must be open");
        h.event(Event::Copy);
        h.step();
        assert_eq!(
            h.state().desktop.copied,
            ["orig 0
trans 0"]
        );
        assert!(h.state().selection.span().is_some());
    }

    #[test]
    fn context_menu_only_with_selection() {
        let mut h = numbered(1);
        let spot = Pos2::new(200.0, 150.0);
        right_click(&mut h, spot);
        h.step();
        assert_eq!(h.state().selection.menu_rect, None);

        let (from, to) = (start_of(&h, 0, 0), end_of(&h, 0, 1));
        select(&mut h, from, to);
        right_click(&mut h, spot);
        h.step();
        let menu = h.state().selection.menu_rect.expect("menu shown");
        assert_eq!(menu.min, spot);
        let label = t("menu.copy", &[]);
        assert!(
            h.output()
                .shapes
                .iter()
                .any(|c| matches!(&c.shape, Shape::Text(s) if s.galley.text() == label))
        );
        // 點選單：複製並關閉，選取留著
        h.hover_at(menu.center());
        h.step();
        h.drag_at(menu.center());
        h.step();
        h.drop_at(menu.center());
        h.step();
        assert_eq!(h.state().desktop.copied, ["orig 0\ntrans 0"]);
        assert_eq!(h.state().selection.menu_rect, None);
        assert!(h.state().selection.span().is_some());
    }

    #[test]
    fn hovering_the_menu_swaps_its_colours() {
        let mut h = numbered(1);
        let (from, to) = (start_of(&h, 0, 0), end_of(&h, 0, 1));
        select(&mut h, from, to);
        right_click(&mut h, Pos2::new(200.0, 150.0));
        h.step();
        let menu = h.state().selection.menu_rect.unwrap();
        let row_fill = |h: &Harness<State>| {
            h.output().shapes.iter().find_map(|c| match &c.shape {
                Shape::Rect(r) if r.rect == menu.shrink(1.0) => Some(r.fill),
                _ => None,
            })
        };
        assert_eq!(row_fill(&h), Some(BAR));
        h.hover_at(menu.center());
        h.step();
        h.step();
        assert_eq!(row_fill(&h), Some(SELECT_BG));
    }

    #[test]
    fn pressing_anywhere_else_closes_the_menu() {
        for elsewhere in ["message", "title bar", "scrollbar"] {
            let mut h = numbered(3);
            let (from, to) = (start_of(&h, 0, 0), end_of(&h, 0, 1));
            select(&mut h, from, to);
            right_click(&mut h, Pos2::new(200.0, 200.0));
            h.step();
            assert!(h.state().selection.menu_rect.is_some());
            let p = match elsewhere {
                "message" => start_of(&h, 1, 0),
                "title bar" => Pos2::new(200.0, h.state().area.min.y + BAR_HEIGHT / 2.0),
                _ => Pos2::new(h.state().list.viewport().max.x + 3.0, 100.0),
            };
            press(&mut h, p);
            release(&mut h, p);
            assert_eq!(h.state().selection.menu_rect, None, "{elsewhere}");
        }
    }

    #[test]
    fn menu_stays_inside_the_overlay() {
        let mut h = numbered(1);
        let (from, to) = (start_of(&h, 0, 0), end_of(&h, 0, 1));
        select(&mut h, from, to);
        let area = h.state().area;
        right_click(&mut h, area.max - Vec2::splat(3.0));
        h.step();
        let menu = h.state().selection.menu_rect.unwrap();
        assert!(area.shrink(3.0).contains_rect(menu), "{menu:?} {area:?}");
    }
}
