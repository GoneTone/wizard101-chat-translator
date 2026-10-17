//! overlay 的訊息清單繪製：每則原文＋譯文兩行描邊字，只排可見範圍，黏底、錨定閱讀位置、細捲軸與滾輪。
//!
//! 捲動位置記成「視口頂端那則訊息＋它的列頂偏移」而非像素原點：上方訊息被移除、某列變高或換行寬度改變時，
//! 正在讀的那幾行都留在原處；不在視口內的訊息不排版，捲動範圍用估計高度。

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use eframe::egui::{
    Color32, Event, FontId, Galley, Label, MouseWheelUnit, Painter, Pos2, Rect, RichText, Ui, Vec2,
};

use crate::i18n::t;
use crate::logs::log;
use crate::ui::fonts_egui::tk_pt;
use crate::ui::geometry::EDGE;
use crate::ui::overlay::frame::{GRIP_SIZE, status_color};
use crate::ui::overlay::model::{Message, OverlayModel, ViewKey, fingerprint};
use crate::ui::overlay::outlined::{self, TEXT_ORIGIN, line_height};
use crate::ui::overlay::scrollbar::{self, Scrollbar};
use crate::ui::overlay::selection::{self, Caret};
use crate::ui::palette::SELECT_BG;

/// 視圖底緣比例達到這個值就算在底部。
pub const STICK_THRESHOLD: f64 = 0.999;
/// 每則訊息上下各留的間距（列與列之間共 4px）。
pub const ROW_PAD: f32 = 2.0;
const ORIGINAL_PT: f32 = 9.0;
const TRANSLATED_PT: f32 = 11.0;
const PLACEHOLDER_PT: f32 = 11.0;
const MIN_WRAP: f32 = 80.0;
const WRAP_MARGIN: f32 = 12.0;
const SLOW_RELAYOUT_MS: u128 = 50;

/// 視圖底緣接近最底時，新訊息應自動跟到底；使用者往上捲時則否。
pub fn should_stick_to_bottom(view_bottom_fraction: f64) -> bool {
    view_bottom_fraction >= STICK_THRESHOLD
}

/// 一則可見訊息排好的兩行（原文、譯文）：行原點（螢幕座標）與排版結果，供框選 hit-test。
pub struct Row {
    /// 對應 `Message::seq`。
    pub seq: u64,
    /// 原文行與譯文行。
    pub lines: [(Pos2, Arc<Galley>); 2],
}

/// 本幀使用者的捲動操作。
enum Scroll {
    /// 內容往下移的距離（DIP），負值往上。
    By(f32),
    /// 把可見範圍頂端移到捲動範圍的這個比例。
    To(f64),
}

/// 排版的起點：最新一則貼底，或第 `index` 則的列頂放在視口內 `top` 處。
enum Start {
    Bottom,
    At(usize, f32),
}

struct Placed {
    index: usize,
    top: f32,
    height: f32,
    original: Arc<Galley>,
    translated: Arc<Galley>,
}

/// 訊息清單的捲動與排版狀態；訊息本身在 `OverlayModel`。
pub struct MessageList {
    key: Option<ViewKey>,
    follow: bool,
    dragging: bool,
    /// 拖曳選取放開後維持原位，直到內容或視口大小改變。
    held: bool,
    /// 拖曳或維持原位期間上一幀的（內容雜湊, 視口大小）。
    seen: Option<(u64, Vec2)>,
    /// 待套用的捲動量（內容往下移的 DIP），框選自動捲動用。
    pending: f32,
    viewport: Rect,
    anchor: Option<(u64, f32)>,
    /// 每個分頁、每則訊息上次排版時的（換行寬度, 整列高度），估計未排版訊息的高度用。
    heights: HashMap<ViewKey, BTreeMap<u64, (f32, f32)>>,
    wrap: f32,
    rows: Vec<Row>,
    scrollbar: Scrollbar,
}

impl Default for MessageList {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageList {
    /// 黏在底部、尚未顯示任何分頁的清單。
    pub fn new() -> Self {
        Self {
            key: None,
            follow: true,
            dragging: false,
            held: false,
            seen: None,
            pending: 0.0,
            viewport: Rect::NOTHING,
            anchor: None,
            heights: HashMap::new(),
            wrap: 0.0,
            rows: Vec::new(),
            scrollbar: Scrollbar::default(),
        }
    }

    /// 是否黏在底部（新訊息自動捲到底）。
    pub fn follow(&self) -> bool {
        self.follow
    }

    /// 拖曳選取期間暫停自動捲到底：畫面被新訊息拉走的話，游標下的字會整個換掉。
    ///
    /// 放開後也先不貼底，等內容或視口大小改變才恢復，免得右鍵複製前選取就被拉走。
    pub fn set_dragging(&mut self, dragging: bool) {
        self.held |= self.dragging && !dragging;
        self.dragging = dragging;
    }

    /// 下一幀把內容往下移 `dy`（DIP，負值往上），框選拖出視口時的自動捲動用。
    pub fn scroll_by(&mut self, dy: f32) {
        self.pending += dy;
    }

    /// 上一幀的訊息視口（不含捲軸與右側縮放帶）。
    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    /// 上一幀畫出的可見訊息，由上到下。
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// 在 `rect`（標題列與分頁列以下的整塊內容區）畫出顯示中分頁的訊息，`selected` 範圍內的字墊上反白；
    /// 沒有訊息時大字置中顯示狀態。
    pub fn show(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        model: &OverlayModel,
        status: Option<&str>,
        selected: Option<(Caret, Caret)>,
    ) {
        let viewport = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.max.x - scrollbar::WIDTH - EDGE as f32, rect.max.y),
        );
        self.viewport = viewport;
        let mut dy = std::mem::take(&mut self.pending);
        // 底部讓出縮放把手：捲軸鋪到底會被它壓住，也會蓋到右下角的縮放帶
        let track = Rect::from_min_max(
            Pos2::new(viewport.max.x, rect.min.y),
            Pos2::new(viewport.max.x + scrollbar::WIDTH, rect.max.y - GRIP_SIZE),
        );
        let key = model.current_key();
        let messages = model.current_view().map_or(&[][..], |v| &v.messages);
        if key != self.key {
            self.key = key;
            self.follow = true;
            self.held = false;
            self.anchor = None;
            if let Some((slot, tab)) = key {
                log(format!(
                    "[ui] view switched (slot={slot}, tab={tab}, messages={})",
                    messages.len()
                ));
            }
        }
        self.rows.clear();
        let Some(key) = key.filter(|_| !messages.is_empty()) else {
            self.anchor = None;
            if let Some(state) = status {
                placeholder(ui, rect, state);
            }
            return;
        };

        let started = Instant::now();
        let wrap = (viewport.width() - WRAP_MARGIN).max(MIN_WRAP);
        let rewrapped = wrap != self.wrap;
        self.wrap = wrap;
        let mut scroll = self.scrollbar.drag(ui, track).map(Scroll::To);
        if ui.rect_contains_pointer(rect) {
            dy += wheel_delta(ui, viewport.height());
        }
        if dy != 0.0 {
            scroll = Some(Scroll::By(dy));
        }
        if self.dragging || self.held {
            let seen = Some((fingerprint(messages), viewport.size()));
            self.held &= seen == self.seen;
            self.seen = seen;
        }

        let painter = ui.painter_at(viewport);
        let mut layout = Layout {
            painter: &painter,
            messages,
            heights: self.heights.entry(key).or_default(),
            wrap,
            viewport_h: viewport.height(),
        };
        layout.forget_removed();
        let one_line = layout.one_line();
        let start = match scroll {
            Some(Scroll::To(fraction)) => layout.start_at_fraction(fraction, one_line),
            Some(Scroll::By(dy)) => anchored(self.anchor, messages, dy),
            None if self.follow && !self.dragging && !self.held => Start::Bottom,
            None => anchored(self.anchor, messages, 0.0),
        };
        let placed = layout.place(start);
        let (first, last, content_h) = layout.fractions(&placed, one_line);

        // 不取被視口上緣切到的那則：它長高（譯文回填）時錨在它的列頂，視口內的內容就會整段下移
        let anchor = placed
            .iter()
            .find(|p| p.top + ROW_PAD >= 0.0)
            .or(placed.back())
            .expect("a non-empty view places at least one row");
        self.anchor = Some((messages[anchor.index].seq, anchor.top));
        if scroll.is_some() {
            let follow = should_stick_to_bottom(last);
            if follow != self.follow {
                self.follow = follow;
                log(format!(
                    "[ui] auto-follow {} by user scroll (messages={} follow={follow} yview_bottom={last:.4} content_h={content_h:.0} canvas_h={:.0})",
                    if follow { "enabled" } else { "disabled" },
                    messages.len(),
                    viewport.height()
                ));
            }
        }

        for p in placed {
            let message = &messages[p.index];
            let original = viewport.min + Vec2::new(0.0, p.top + ROW_PAD);
            let translated = original + Vec2::new(0.0, line_height(&p.original));
            let lines = [
                (original, &p.original, message.original_color()),
                (translated, &p.translated, message.translated_color()),
            ];
            for (line, (origin, galley, color)) in lines.into_iter().enumerate() {
                if let Some((lo, hi)) =
                    selected.and_then(|s| selection::line_range(s, p.index, line))
                {
                    let text = origin.to_vec2() + Vec2::splat(TEXT_ORIGIN);
                    for r in selection::highlight_rects(galley, lo, hi) {
                        painter.rect_filled(r.translate(text), 0.0, SELECT_BG);
                    }
                }
                outlined::paint(&painter, origin, galley, color);
            }
            self.rows.push(Row {
                seq: message.seq,
                lines: [(original, p.original), (translated, p.translated)],
            });
        }
        self.scrollbar.paint(ui, track, first, last);

        let ms = started.elapsed().as_millis();
        if rewrapped && ms >= SLOW_RELAYOUT_MS {
            log(format!(
                "[ui] view relaid out (slot={}, tab={}, drawn={}, messages={}, wrap={wrap}, ms={ms})",
                key.0,
                key.1,
                self.rows.len(),
                messages.len()
            ));
        }
    }
}

/// 從錨點那則（已被移除就改用其後第一則）排起，列頂再往下移 `dy`。
fn anchored(anchor: Option<(u64, f32)>, messages: &[Message], dy: f32) -> Start {
    let Some((seq, top)) = anchor else {
        return Start::Bottom;
    };
    let index = messages.partition_point(|m| m.seq < seq);
    if index == messages.len() {
        Start::Bottom
    } else {
        Start::At(index, top + dy)
    }
}

/// 本幀的滾輪量換成內容位移（DIP）：一格捲視口高度的十分之一，與 Tk 的 `yview_scroll(units)` 相同。
fn wheel_delta(ui: &Ui, viewport_h: f32) -> f32 {
    ui.input(|i| {
        i.events
            .iter()
            .map(|e| match e {
                Event::MouseWheel { unit, delta, .. } => match unit {
                    MouseWheelUnit::Point => delta.y,
                    MouseWheelUnit::Line => delta.y * viewport_h / 10.0,
                    MouseWheelUnit::Page => delta.y * viewport_h,
                },
                _ => 0.0,
            })
            .sum()
    })
}

fn placeholder(ui: &mut Ui, rect: Rect, state: &str) {
    let text = t(&format!("status.{state}"), &[]);
    let font = FontId::proportional(tk_pt(PLACEHOLDER_PT));
    let size = ui
        .painter()
        .layout_no_wrap(text.clone(), font.clone(), Color32::PLACEHOLDER)
        .size();
    let label = RichText::new(text).font(font).color(status_color(state));
    let at = Rect::from_center_size(rect.center(), size);
    ui.place(at, Label::new(label).selectable(false).extend());
}

/// 一幀的排版：只排視口內的訊息，其餘以上次的高度依換行寬度比例估計。
struct Layout<'a> {
    painter: &'a Painter,
    messages: &'a [Message],
    heights: &'a mut BTreeMap<u64, (f32, f32)>,
    wrap: f32,
    viewport_h: f32,
}

impl Layout<'_> {
    /// 丟掉已被移除（比最舊一則還舊）的訊息的高度記錄。
    fn forget_removed(&mut self) {
        let oldest = self.messages[0].seq;
        if self
            .heights
            .first_key_value()
            .is_some_and(|(&k, _)| k < oldest)
        {
            self.heights.retain(|&k, _| k >= oldest);
        }
    }

    fn galley(&self, text: &str, pt: f32) -> Arc<Galley> {
        self.painter.layout(
            text.to_owned(),
            FontId::proportional(tk_pt(pt)),
            Color32::PLACEHOLDER,
            self.wrap,
        )
    }

    /// 兩種字型各一行的整列高度：沒排過或單行訊息的估計值。
    fn one_line(&self) -> f32 {
        line_height(&self.galley("x", ORIGINAL_PT))
            + line_height(&self.galley("x", TRANSLATED_PT))
            + 2.0 * ROW_PAD
    }

    fn estimate(&self, message: &Message, one_line: f32) -> f32 {
        match self.heights.get(&message.seq) {
            Some(&(wrap, height)) if height > one_line => (height * wrap / self.wrap).max(one_line),
            _ => one_line,
        }
    }

    fn measure(&mut self, index: usize, top: f32) -> Placed {
        let message = &self.messages[index];
        let original = self.galley(&message.original, ORIGINAL_PT);
        let translated = self.galley(&message.translated, TRANSLATED_PT);
        let height = line_height(&original) + line_height(&translated) + 2.0 * ROW_PAD;
        self.heights.insert(message.seq, (self.wrap, height));
        Placed {
            index,
            top,
            height,
            original,
            translated,
        }
    }

    /// 捲到比例 `fraction` 時的排版起點；可見範圍碰到捲動範圍底端就貼底。
    fn start_at_fraction(&self, fraction: f64, one_line: f32) -> Start {
        let estimates: Vec<f32> = self
            .messages
            .iter()
            .map(|m| self.estimate(m, one_line))
            .collect();
        let total: f32 = estimates.iter().sum();
        let target = fraction as f32 * total;
        if target + self.viewport_h >= total {
            return Start::Bottom;
        }
        let mut above = 0.0;
        for (index, height) in estimates.iter().enumerate() {
            if above + height > target {
                return Start::At(index, above - target);
            }
            above += height;
        }
        Start::Bottom
    }

    /// 從起點往上下排到填滿視口；排到頭或尾還留空就整段推過去貼齊（內容比視口矮時貼頂）。
    fn place(&mut self, start: Start) -> VecDeque<Placed> {
        let last = self.messages.len() - 1;
        let mut placed = VecDeque::new();
        let first = match start {
            Start::Bottom => {
                let mut row = self.measure(last, 0.0);
                row.top = self.viewport_h - row.height;
                row
            }
            Start::At(index, top) => self.measure(index, top),
        };
        placed.push_back(first);
        self.fill(&mut placed);
        let bottom = placed.back().map_or(0.0, |p| p.top + p.height);
        if placed.back().is_some_and(|p| p.index == last) && bottom < self.viewport_h {
            shift(&mut placed, self.viewport_h - bottom);
            self.fill(&mut placed);
        }
        let top = placed.front().map_or(0.0, |p| p.top);
        if placed.front().is_some_and(|p| p.index == 0) && top > 0.0 {
            shift(&mut placed, -top);
            self.fill(&mut placed);
        }
        placed
    }

    fn fill(&mut self, placed: &mut VecDeque<Placed>) {
        while let Some(p) = placed.back()
            && p.index + 1 < self.messages.len()
            && p.top + p.height < self.viewport_h
        {
            let (index, top) = (p.index + 1, p.top + p.height);
            let row = self.measure(index, top);
            placed.push_back(row);
        }
        while let Some(p) = placed.front()
            && p.index > 0
            && p.top > 0.0
        {
            let (index, bottom) = (p.index - 1, p.top);
            let mut row = self.measure(index, 0.0);
            row.top = bottom - row.height;
            placed.push_front(row);
        }
    }

    /// 可見範圍在捲動範圍中的（頂端比例, 底端比例）與估計的內容總高。
    fn fractions(&self, placed: &VecDeque<Placed>, one_line: f32) -> (f64, f64, f32) {
        let (Some(front), Some(back)) = (placed.front(), placed.back()) else {
            return (0.0, 1.0, 0.0);
        };
        let above: f32 = self.messages[..front.index]
            .iter()
            .map(|m| self.estimate(m, one_line))
            .sum();
        let below: f32 = self.messages[back.index + 1..]
            .iter()
            .map(|m| self.estimate(m, one_line))
            .sum();
        let top = front.top - above;
        let total = back.top + back.height + below - top;
        if total <= 0.0 {
            return (0.0, 1.0, total);
        }
        let first = f64::from(-top / total).clamp(0.0, 1.0);
        let last = f64::from((self.viewport_h - top) / total).clamp(0.0, 1.0);
        (first, last, total)
    }
}

fn shift(placed: &mut VecDeque<Placed>, dy: f32) {
    for p in placed {
        p.top += dy;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use eframe::egui::{self, Modifiers, MouseWheelSource, RawInput, Shape, TouchPhase};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::reader::chat_tabs::TabState;
    use crate::ui::fonts_egui;
    use crate::ui::overlay::scrollbar::{THUMB, THUMB_HOVER};

    const SIZE: Vec2 = Vec2::new(460.0, 300.0);

    struct State {
        model: OverlayModel,
        list: MessageList,
        status: Option<&'static str>,
        rect: Rect,
        t0: Instant,
    }

    fn harness(model: OverlayModel) -> Harness<'static, State> {
        let state = State {
            model,
            list: MessageList::new(),
            status: None,
            rect: Rect::NOTHING,
            t0: Instant::now(),
        };
        let mut harness = Harness::builder()
            .with_size(SIZE)
            .with_context_setup(|ctx| fonts_egui::install(ctx, "zh-TW"))
            .build_ui_state(
                |ui, s: &mut State| {
                    s.rect = ui.max_rect();
                    s.list.show(ui, s.rect, &s.model, s.status, None);
                },
                state,
            );
        harness.step();
        harness
    }

    /// 第 i 則：夠長，窄視窗下一定會換行；msg_id 與加入時刻都取 i。
    fn add(h: &mut Harness<State>, i: u64, tabs: &[i32], pending: bool) {
        let s = h.state_mut();
        let at = s.t0 + Duration::from_secs(i);
        s.model.add_message(
            1,
            tabs,
            &format!("message number {i}: a chat line long enough to wrap"),
            &format!("第 {i} 則譯文，內容夠長，窄視窗下一定會換行"),
            Some(i),
            pending,
            None,
            at,
        );
    }

    fn filled(count: u64, max: usize, fade: u64) -> Harness<'static, State> {
        let mut h = harness(OverlayModel::new(max, fade));
        for i in 0..count {
            add(&mut h, i, &[0], false);
        }
        h.step();
        h
    }

    fn row_bottom(row: &Row) -> f32 {
        let (origin, galley) = &row.lines[1];
        origin.y + line_height(galley) + ROW_PAD
    }

    fn messages<'a>(h: &'a Harness<State>) -> &'a [Message] {
        &h.state().model.current_view().unwrap().messages
    }

    fn at_bottom(h: &Harness<State>) -> bool {
        let last = h.state().list.rows().last().unwrap();
        last.seq == messages(h).last().unwrap().seq
            && (row_bottom(last) - h.state().rect.max.y).abs() < 0.01
    }

    /// 該則原文行在螢幕上的 y；不在可見範圍時為 None。
    fn row_y(h: &Harness<State>, seq: u64) -> Option<f32> {
        let rows = h.state().list.rows();
        rows.iter().find(|r| r.seq == seq).map(|r| r.lines[0].0.y)
    }

    /// 正在讀的一則（列頂沒被視口上緣切到的第一則）的 seq 與 y。
    fn reading(h: &Harness<State>) -> (u64, f32) {
        let top = h.state().rect.min.y;
        let rows = h.state().list.rows();
        let row = rows.iter().find(|r| r.lines[0].0.y >= top).unwrap();
        (row.seq, row.lines[0].0.y)
    }

    /// 可見的列上下相接、且蓋滿視口（內容比視口矮或捲到頭時例外）。
    fn covered(h: &Harness<State>) -> bool {
        let rows = h.state().list.rows();
        let rect = h.state().rect;
        let all = messages(h);
        let contiguous = rows
            .windows(2)
            .all(|w| (w[1].lines[0].0.y - row_bottom(&w[0]) - ROW_PAD).abs() < 0.01);
        let top = rows[0].lines[0].0.y - ROW_PAD;
        let bottom = row_bottom(rows.last().unwrap());
        let top_ok = top <= rect.min.y || (rows[0].seq == all[0].seq && top == rect.min.y);
        let bottom_ok = bottom >= rect.max.y || rows.last().unwrap().seq == all.last().unwrap().seq;
        contiguous && top_ok && bottom_ok
    }

    fn wheel_at(h: &mut Harness<State>, pos: Pos2, notches: f32) {
        h.hover_at(pos);
        h.event(Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: Vec2::new(0.0, notches),
            phase: TouchPhase::Move,
            source: MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }

    fn wheel(h: &mut Harness<State>, notches: f32) {
        let center = h.state().rect.center();
        wheel_at(h, center, notches);
    }

    fn thumb(h: &Harness<State>) -> Option<Rect> {
        h.output().shapes.iter().find_map(|c| match &c.shape {
            Shape::Rect(r) if r.fill == THUMB || r.fill == THUMB_HOVER => Some(r.rect),
            _ => None,
        })
    }

    fn drag_thumb_to(h: &mut Harness<State>, y: f32) {
        let from = thumb(h).expect("scrollbar visible").center();
        h.hover_at(from);
        h.drag_at(from);
        h.step();
        let to = Pos2::new(from.x, y);
        h.hover_at(to);
        h.step();
        h.drop_at(to);
        h.step();
    }

    #[test]
    fn stick_threshold_matches_python() {
        assert!(should_stick_to_bottom(1.0));
        assert!(should_stick_to_bottom(0.9995));
        assert!(!should_stick_to_bottom(0.4));
    }

    #[test]
    fn new_message_follows_when_at_bottom() {
        let mut h = filled(40, 200, 0);
        assert!(h.state().list.follow());
        assert!(at_bottom(&h));
        add(&mut h, 40, &[0], false);
        h.step();
        assert!(at_bottom(&h));
    }

    #[test]
    fn does_not_follow_after_user_scrolls_up() {
        let mut h = filled(40, 200, 0);
        wheel(&mut h, 5.0);
        assert!(!h.state().list.follow());
        let (seq, y) = reading(&h);
        for i in 40..45 {
            add(&mut h, i, &[0], false);
        }
        h.step();
        assert_eq!(row_y(&h, seq), Some(y));
        assert!(!at_bottom(&h));
        // 捲回底部後恢復自動跟隨
        wheel(&mut h, -1000.0);
        assert!(h.state().list.follow());
        add(&mut h, 45, &[0], false);
        h.step();
        assert!(at_bottom(&h));
    }

    #[test]
    fn wheel_scrolls_a_tenth_of_the_viewport_per_notch() {
        let mut h = filled(40, 200, 0);
        let (seq, y) = reading(&h);
        wheel(&mut h, 1.0);
        assert_eq!(row_y(&h, seq), Some(y + h.state().rect.height() / 10.0));
    }

    #[test]
    fn wheel_works_over_the_scrollbar_and_the_margin() {
        for inset in [10.0, 3.0] {
            let mut h = filled(40, 200, 0);
            let (seq, y) = reading(&h);
            let rect = h.state().rect;
            wheel_at(&mut h, Pos2::new(rect.max.x - inset, rect.center().y), 1.0);
            assert!(row_y(&h, seq).unwrap() > y, "inset={inset}");
        }
    }

    #[test]
    fn anchor_keeps_reading_position_on_prune() {
        let mut h = filled(40, 200, 60);
        wheel(&mut h, 8.0);
        let (seq, y) = reading(&h);
        let t0 = h.state().t0;
        h.state_mut().model.prune(t0 + Duration::from_secs(10 + 60));
        h.step();
        assert_eq!(messages(&h).len(), 29);
        assert_eq!(row_y(&h, seq), Some(y));
    }

    #[test]
    fn message_cap_keeps_scrolled_view_in_place() {
        let mut h = filled(40, 40, 0);
        wheel(&mut h, 8.0);
        let (seq, y) = reading(&h);
        for i in 100..105 {
            add(&mut h, i, &[0], false);
        }
        h.step();
        assert_eq!(row_y(&h, seq), Some(y));
    }

    #[test]
    fn filling_a_pending_translation_above_keeps_scrolled_view_in_place() {
        let mut h = filled(40, 200, 0);
        add(&mut h, 500, &[0], true);
        for i in 600..610 {
            add(&mut h, i, &[0], false);
        }
        h.step();
        wheel(&mut h, 2.0);
        let (seq, y) = reading(&h);
        let pending = messages(&h)[40].seq;
        assert!(
            row_y(&h, pending).is_none(),
            "precondition: pending row above the viewport"
        );
        let long = "終於補上的譯文，刻意寫得很長，長到足以讓那一列撐成好幾行。".repeat(4);
        h.state_mut().model.update_message(500, &long, false);
        h.step();
        assert_eq!(row_y(&h, seq), Some(y));
    }

    #[test]
    fn dragging_a_selection_suspends_auto_follow_until_content_changes() {
        let mut h = filled(40, 200, 60);
        h.state_mut().list.set_dragging(true);
        let (seq, y) = reading(&h);
        add(&mut h, 40, &[0], false);
        h.step();
        assert_eq!(row_y(&h, seq), Some(y));
        let t0 = h.state().t0;
        h.state_mut().model.prune(t0 + Duration::from_secs(5 + 60));
        h.step();
        assert_eq!(row_y(&h, seq), Some(y));
        // 放開後畫面留在原處，右鍵複製前選取不會被拉走；等下一則訊息進來才貼底
        h.state_mut().list.set_dragging(false);
        h.step();
        h.step();
        assert_eq!(row_y(&h, seq), Some(y));
        add(&mut h, 41, &[0], false);
        h.step();
        assert!(at_bottom(&h));
    }

    #[test]
    fn a_resize_after_releasing_a_selection_resumes_following() {
        let mut h = filled(40, 200, 0);
        h.state_mut().list.set_dragging(true);
        h.step();
        add(&mut h, 40, &[0], false);
        h.step();
        h.state_mut().list.set_dragging(false);
        h.step();
        assert!(!at_bottom(&h));
        h.set_size(Vec2::new(300.0, SIZE.y));
        h.step();
        assert!(at_bottom(&h));
    }

    #[test]
    fn scroll_by_moves_the_content_on_the_next_frame() {
        let mut h = filled(40, 200, 0);
        let (seq, y) = reading(&h);
        h.state_mut().list.scroll_by(30.0);
        h.step();
        assert_eq!(row_y(&h, seq), Some(y + 30.0));
        assert!(!h.state().list.follow());
    }

    #[test]
    fn placeholder_shown_when_empty() {
        let mut h = harness(OverlayModel::new(10, 0));
        h.state_mut().status = Some("locating");
        h.step();
        let label = t("status.locating", &[]);
        let center = h.get_by_label(&label).rect().center();
        assert!((center - h.state().rect.center()).length() < 1.0);
        add(&mut h, 0, &[0], false);
        h.step();
        assert!(h.query_by_label(&label).is_none());
    }

    #[test]
    fn layout_follows_the_old_frame_geometry() {
        let mut h = harness(OverlayModel::new(10, 0));
        add(&mut h, 0, &[0], false);
        add(&mut h, 1, &[0], false);
        h.step();
        let rows = h.state().list.rows();
        let rect = h.state().rect;
        assert_eq!(rows.len(), 2);
        // 兩行上下相接、列與列之間 4px，第一則的行原點在內容頂端下 2px
        let mut top = rect.min.y + ROW_PAD;
        for row in rows {
            let [(original, galley), (translated, _)] = &row.lines;
            assert_eq!(*original, Pos2::new(rect.min.x, top));
            assert_eq!(
                *translated,
                Pos2::new(rect.min.x, top + line_height(galley))
            );
            top = row_bottom(row) + ROW_PAD;
        }
        // 譯文回填變高：下方的訊息跟著往下推
        let before = row_y(&h, 1).unwrap();
        let long = "這則譯文刻意寫得很長，長到一定會多換一行以上，後面那則才需要往下移".repeat(3);
        h.state_mut().model.update_message(0, &long, false);
        h.step();
        assert!(row_y(&h, 1).unwrap() > before);
        assert!(covered(&h));
    }

    #[test]
    fn rows_are_painted_in_the_message_colours() {
        let mut h = harness(OverlayModel::new(10, 0));
        add(&mut h, 0, &[0], true);
        h.step();
        let fills: Vec<Color32> = h
            .output()
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Text(t) => Some(t.fallback_color),
                _ => None,
            })
            .collect();
        let message = &messages(&h)[0];
        assert_eq!(fills.len(), 18);
        assert_eq!(fills[8], message.original_color());
        assert_eq!(fills[17], message.translated_color());
    }

    #[test]
    fn follow_mode_keeps_the_newest_message_at_the_bottom_after_a_resize() {
        let mut h = filled(120, 200, 0);
        for size in [
            Vec2::new(300.0, SIZE.y),
            Vec2::new(240.0, SIZE.y),
            Vec2::new(600.0, SIZE.y),
            Vec2::new(SIZE.x, 200.0),
            Vec2::new(SIZE.x, 900.0),
        ] {
            h.set_size(size);
            h.step();
            assert!(at_bottom(&h) && covered(&h), "{size:?}");
            // 只排視口內的訊息，且全部用新的換行寬度
            let rows = h.state().list.rows();
            assert!(rows.len() < 60, "{size:?} laid out {}", rows.len());
            let wrap = h.state().rect.width() - scrollbar::WIDTH - EDGE as f32 - WRAP_MARGIN;
            for (_, galley) in rows.iter().flat_map(|r| &r.lines) {
                assert!(galley.size().x <= wrap + 0.5, "{size:?}");
            }
        }
    }

    #[test]
    fn wheeling_up_after_a_resize_never_shows_a_gap() {
        let mut h = filled(120, 200, 0);
        h.set_size(Vec2::new(300.0, SIZE.y));
        h.step();
        for _ in 0..120 {
            wheel(&mut h, 3.0);
            assert!(covered(&h));
        }
        let first = messages(&h)[0].seq;
        assert_eq!(row_y(&h, first), Some(h.state().rect.min.y + ROW_PAD));
    }

    #[test]
    fn pruning_the_rows_in_view_lands_on_the_oldest_remaining_message() {
        let mut h = filled(120, 200, 60);
        wheel(&mut h, 40.0);
        let last_visible = h.state().list.rows().last().unwrap().seq;
        let t0 = h.state().t0;
        h.state_mut()
            .model
            .prune(t0 + Duration::from_secs(last_visible + 60));
        h.step();
        let oldest = messages(&h)[0].seq;
        assert_eq!(oldest, last_visible + 1);
        assert_eq!(row_y(&h, oldest), Some(h.state().rect.min.y + ROW_PAD));
        assert!(covered(&h));
        assert!(!h.state().list.follow());
    }

    #[test]
    fn reading_history_keeps_the_viewport_top_message_in_place_on_resize() {
        let mut h = filled(120, 200, 0);
        wheel(&mut h, 30.0);
        let (seq, y) = reading(&h);
        h.set_size(Vec2::new(300.0, SIZE.y));
        h.step();
        assert_eq!(row_y(&h, seq), Some(y));
        assert!(!h.state().list.follow());
    }

    #[test]
    fn switching_views_returns_to_following_the_bottom() {
        let mut h = harness(OverlayModel::new(200, 0));
        let tabs = |current| TabState {
            tabs: vec![(0, "Main".into()), (1, "Group".into())],
            current,
        };
        h.state_mut().model.set_tabs(1, Some(tabs(0)));
        for i in 0..40 {
            add(&mut h, i, &[0, 1], false);
        }
        h.step();
        wheel(&mut h, 5.0);
        assert!(!h.state().list.follow());
        h.state_mut().model.set_tabs(1, Some(tabs(1)));
        h.step();
        h.state_mut().model.set_tabs(1, Some(tabs(0)));
        h.step();
        assert!(h.state().list.follow());
        assert!(at_bottom(&h));
    }

    #[test]
    fn scrollbar_hidden_when_content_fits() {
        let mut h = harness(OverlayModel::new(10, 0));
        add(&mut h, 0, &[0], false);
        h.step();
        assert!(thumb(&h).is_none());
        assert!(thumb(&filled(40, 200, 0)).is_some());
    }

    #[test]
    fn scrollbar_stays_clear_of_the_resize_bands_and_grip() {
        let mut h = filled(40, 200, 0);
        wheel(&mut h, 1000.0);
        let top = thumb(&h).unwrap();
        wheel(&mut h, -1000.0);
        let bottom = thumb(&h).unwrap();
        let rect = h.state().rect;
        for t in [top, bottom] {
            assert!(t.max.x <= rect.max.x - EDGE as f32, "{t:?}");
        }
        assert!(bottom.max.y <= rect.max.y - GRIP_SIZE + 1.0, "{bottom:?}");
    }

    #[test]
    fn dragging_the_scrollbar_to_the_top_lands_on_the_first_message() {
        let mut h = filled(120, 200, 0);
        // 變窄：上方待排的訊息比估計的高
        h.set_size(Vec2::new(240.0, SIZE.y));
        h.step();
        drag_thumb_to(&mut h, -100.0);
        let first = messages(&h)[0].seq;
        assert_eq!(row_y(&h, first), Some(h.state().rect.min.y + ROW_PAD));
        assert!(!h.state().list.follow());
    }

    #[test]
    fn dragging_the_scrollbar_to_the_bottom_lands_on_the_newest_message() {
        let mut h = filled(120, 200, 0);
        drag_thumb_to(&mut h, 60.0);
        assert!(!h.state().list.follow());
        // 變窄：下方待排的訊息比估計的高
        h.set_size(Vec2::new(200.0, SIZE.y));
        h.step();
        drag_thumb_to(&mut h, 1000.0);
        assert!(at_bottom(&h));
        assert!(h.state().list.follow());
    }

    #[test]
    #[ignore = "release-only budget; CI runs `cargo test --release -- --ignored relayout`"]
    fn relayout_1000_messages_within_budget() {
        const LONG: &str = "這是一段普通長度的聊天測試訊息，寬畫布一行放得下";
        let ctx = egui::Context::default();
        fonts_egui::install(&ctx, "zh-TW");
        let mut model = OverlayModel::new(1000, 0);
        let now = Instant::now();
        for i in 0..1000 {
            let original = format!("wizard chat line number {i}: hello there");
            let translated = format!("{LONG} {i}");
            model.add_message(1, &[0], &original, &translated, None, false, None, now);
        }
        let mut list = MessageList::new();
        let mut frame = |width: f32| {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 300.0))),
                ..Default::default()
            };
            let started = Instant::now();
            let mut output = ctx.run_ui(input, |ui| {
                list.show(ui, ui.max_rect(), &model, None, None);
            });
            let elapsed = started.elapsed();
            output.textures_delta.clear();
            elapsed
        };
        frame(460.0);
        frame(460.0);
        let mut times: Vec<Duration> = (0..20).map(|i| frame(240.0 + 11.0 * i as f32)).collect();
        times.sort();
        let median = times[times.len() / 2];
        assert!(median < Duration::from_millis(8), "{times:?}");
    }
}
