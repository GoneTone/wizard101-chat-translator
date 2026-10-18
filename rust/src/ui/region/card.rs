//! 框選翻譯的結果卡：辨識中／譯文／錯誤三態，貼在框選矩形下方，放不下翻到上方，再不行移到左右較寬的一側。
//!
//! 這裡只有狀態與 egui 內容；視窗由宿主托管。點一下或點 ✕ 關閉，拖曳是選字；原文與譯文排在同一個
//! galley 裡，選取能一路跨過兩段。選取自己做（同 overlay）：egui label 的選取拖出字外就不跟、收成游標時
//! 還會把全文當成選取。

use std::sync::Arc;
use std::time::Duration;

use eframe::egui::text::{CharIndex, LayoutJob, TextFormat};
use eframe::egui::{
    self, Color32, Context, CursorIcon, Event, FontId, Galley, Id, OpenUrl, PointerButton, Pos2,
    Sense, Stroke, StrokeKind, Vec2, pos2, vec2,
};

use crate::i18n::t;
use crate::logs::log;
use crate::ui::copy_menu::{self, MenuOutcome};
use crate::ui::fonts_egui::tk_pt;
use crate::ui::geometry::{Placement, Rect, Side, anchored_geometry, beside_geometry, is_click};
use crate::ui::overlay::scrollbar::{self, Scrollbar};
use crate::ui::palette::{
    BG, FG_BAR, FG_ERROR, FG_ORIGINAL, FG_PENDING, FG_TRANSLATED, FG_UPDATE, GRIP, SELECT_BG,
};
use crate::ui::region::region_box::{HANDLE, MARGIN};
use crate::ui::richtext::{linkify, parse_link_markup};
use crate::ui::text_select::{
    AUTOSCROLL_INTERVAL, autoscroll_pixels, char_range_text, highlight_rects, is_copy, wheel_delta,
};

/// 卡片寬度下限（px）：矩形再窄也不跟，譯文會擠成一長條。
pub const MIN_WIDTH: i32 = 240;
/// 移到側邊時的寬度下限（px）：文字再短也不縮成窄條，蓋到矩形一角無妨。
pub const SIDE_MIN_WIDTH: i32 = 360;
/// 高度下限（px）：內文再短也留這麼高；側邊被夾矮時也不低於此，其餘靠捲動。
pub const MIN_HEIGHT: i32 = 120;
/// 與框選矩形的間距（px）：讓出框選框露在框外的把手。
pub const ANCHOR_GAP: i32 = MARGIN + HANDLE / 2 + 2;
/// 內文左右留白（DIP）。
pub const PAD_X: i32 = 10;
/// 內文上下留白（DIP）。
pub const PAD_Y: i32 = 6;

const BORDER: f32 = 1.0;
const CLOSE_PAD_X: f32 = 6.0;
const CLOSE_PAD_Y: f32 = 1.0;
const PAD: Vec2 = vec2(PAD_X as f32, PAD_Y as f32);

/// 卡片顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardContent {
    /// 辨識與翻譯進行中。
    Pending,
    /// 譯文；`source` 是辨識出的原文，空字串就只顯示譯文，`text` 空字串代表沒辨識到文字。
    Text { text: String, source: String },
    /// 錯誤訊息。
    Error(String),
}

/// `content` 這一幀要宿主處理的事。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardAction {
    /// 使用者點一下卡片或 ✕：收掉卡片與框選框。
    Close,
    /// 已複製這麼多字元到剪貼簿。
    Copied(usize),
    /// 拖曳選字結束：宿主要讓卡片取得鍵盤焦點，Ctrl+C 才收得到。
    FocusForCopy,
}

/// 依內容大小決定卡片的位置（px）：`width` 是貼上下時的寬，`natural_w` 是內文不換行的寬。
///
/// `sticky` 是側邊時沿用側邊：側邊較寬、換行後變矮，拿這個高度重判會跳回下方再換行變高，來回不止。
pub fn place(
    rect: Rect,
    width: i32,
    natural_h: i32,
    natural_w: i32,
    area: Rect,
    sticky: Option<Side>,
) -> Placement {
    match sticky {
        Some(Side::Left | Side::Right) => beside_geometry(
            rect,
            natural_w,
            natural_h,
            area,
            ANCHOR_GAP,
            SIDE_MIN_WIDTH,
            MIN_HEIGHT,
        ),
        _ => anchored_geometry(
            rect,
            width,
            natural_h,
            area,
            ANCHOR_GAP,
            natural_w,
            SIDE_MIN_WIDTH,
            MIN_HEIGHT,
        ),
    }
}

/// 內文的一段：顯示文字、顏色、字級（pt），連結段帶網址。
struct Segment {
    text: String,
    color: Color32,
    pt: f32,
    url: Option<String>,
}

/// 內文各段；有原文時原文在上、空一行再接譯文，網址與 `[文字](網址)` 轉成連結段。
fn segments(content: &CardContent) -> Vec<Segment> {
    let blocks = match content {
        CardContent::Pending => vec![(t("region.working", &[]), FG_PENDING, 11.0)],
        CardContent::Text { text, source } => {
            let shown = if text.is_empty() {
                (t("region.no_text", &[]), FG_PENDING, 11.0)
            } else {
                (text.clone(), FG_TRANSLATED, 11.0)
            };
            if source.is_empty() {
                vec![shown]
            } else {
                vec![
                    (source.clone(), FG_ORIGINAL, 9.0),
                    ("\n\n".into(), FG_TRANSLATED, 11.0),
                    shown,
                ]
            }
        }
        CardContent::Error(message) => vec![(message.clone(), FG_ERROR, 11.0)],
    };
    blocks
        .into_iter()
        .flat_map(|(text, color, pt)| {
            parse_link_markup(&linkify(&text))
                .into_iter()
                .map(move |(text, url)| Segment {
                    color: if url.is_some() { FG_UPDATE } else { color },
                    text,
                    pt,
                    url,
                })
        })
        .collect()
}

struct Chrome {
    close: Arc<Galley>,
    hint: Arc<Galley>,
}

impl Chrome {
    fn new(ctx: &Context) -> Self {
        ctx.fonts_mut(|f| Self {
            close: f.layout_no_wrap("✕".into(), font(10.0), FG_BAR),
            hint: f.layout_no_wrap(t("region.close_hint", &[]), font(8.0), FG_PENDING),
        })
    }

    /// 內文以外佔的高度：邊框、✕ 列、提示列與留白。
    fn height(&self) -> f32 {
        2.0 * BORDER + self.close.size().y + 2.0 * CLOSE_PAD_Y + self.hint.size().y + 3.0 * PAD.y
    }
}

fn font(pt: f32) -> FontId {
    FontId::proportional(tk_pt(pt))
}

/// 卡片寬 `width`（DIP）時內文的換行寬度：扣掉邊框、左右留白與捲軸。
fn text_width(width: f32) -> f32 {
    width - 2.0 * BORDER - 2.0 * PAD.x - scrollbar::WIDTH
}

/// `local`（相對 galley 原點）正壓在哪個字元上；不在任何字上回 None。
fn char_under(galley: &Galley, local: Vec2) -> Option<usize> {
    let mut first = 0;
    for row in &galley.rows {
        let count = row.char_count_excluding_newline().0;
        if (row.min_y()..row.max_y()).contains(&local.y) {
            let x = |i: usize| row.pos.x + row.x_offset(CharIndex(i));
            return (0..count)
                .find(|&i| (x(i)..x(i + 1)).contains(&local.x))
                .map(|i| first + i);
        }
        first += row.char_count_including_newline().0;
    }
    None
}

/// 結果卡的狀態；每次框選建一張新的。
pub struct Card {
    rect: Rect,
    alpha: f32,
    segments: Vec<Segment>,
    side: Option<Side>,
    placed: Option<Placement>,
    /// 內文往上捲了多少（DIP）。
    scroll: f32,
    scrollbar: Scrollbar,
    /// 左鍵按下的位置，放開時判斷是點擊還是拖曳。
    press: Option<Pos2>,
    /// 選取兩端的字元索引；相同代表沒有選取。
    anchor: Option<usize>,
    focus: Option<usize>,
    dragging: bool,
    next_tick: f64,
    /// 右鍵選單的位置。
    menu: Option<Pos2>,
    clipboard: Box<dyn FnMut(&str) -> bool>,
}

impl Card {
    /// 貼在框選矩形 `rect`（螢幕 px）旁、背景不透明度 `alpha`，先顯示辨識中。
    pub fn new(rect: Rect, alpha: f32) -> Self {
        Self {
            rect,
            alpha,
            segments: segments(&CardContent::Pending),
            side: None,
            placed: None,
            scroll: 0.0,
            scrollbar: Scrollbar::default(),
            press: None,
            anchor: None,
            focus: None,
            dragging: false,
            next_tick: 0.0,
            menu: None,
            clipboard: Box::new(|_| false),
        }
    }

    /// 換掉剪貼簿寫入函式（預設寫不進去）；宿主帶入以卡片視窗為擁有者的 `clipboard::set_text`。
    pub fn set_clipboard(&mut self, sink: impl FnMut(&str) -> bool + 'static) {
        self.clipboard = Box::new(sink);
    }

    /// 換內容；落點重新判斷、捲回頂端，舊的選取作廢。
    pub fn set_content(&mut self, content: CardContent) {
        self.segments = segments(&content);
        self.side = None;
        self.scroll = 0.0;
        self.anchor = None;
        self.focus = None;
        self.dragging = false;
        self.close_menu();
    }

    /// 背景不透明度跟著 overlay 一起變。
    pub fn set_alpha(&mut self, alpha: f32) {
        self.alpha = alpha;
    }

    #[cfg(test)]
    pub(crate) fn alpha(&self) -> f32 {
        self.alpha
    }

    /// 卡片貼著的框選矩形（螢幕 px）。
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// 依目前內容算出卡片位置與大小（px），`area` 是框選矩形所在螢幕的工作區。
    ///
    /// 落在側邊時以目前的側邊寬度量高、沿用側邊（見 `place`），直到 `set_content` 換掉內容。
    pub fn layout(&mut self, ctx: &Context, area: Rect) -> Placement {
        let ppp = ctx.pixels_per_point();
        let width = MIN_WIDTH.max(self.rect.2.min(area.2));
        let measured = match (self.side, self.placed) {
            (Some(Side::Left | Side::Right), Some(placed)) => placed.w,
            _ => width,
        };
        let natural_h = (self.natural_height(ctx, measured as f32 / ppp) * ppp).ceil() as i32;
        let natural_w = (self.natural_width(ctx) * ppp).ceil() as i32;
        let placed = place(self.rect, width, natural_h, natural_w, area, self.side);
        self.side = Some(placed.side);
        if self.placed != Some(placed) {
            self.placed = Some(placed);
            log(format!(
                "[region] card placed (rect={:?}, natural_height={natural_h}, side={}, size={}x{}, at=({}, {}))",
                self.rect,
                format!("{:?}", placed.side).to_lowercase(),
                placed.w,
                placed.h,
                placed.x,
                placed.y
            ));
        }
        placed
    }

    /// 卡片寬 `width`（DIP）時，內容全部攤開需要的高度（DIP）。
    fn natural_height(&self, ctx: &Context, width: f32) -> f32 {
        let galley = ctx.fonts_mut(|f| f.layout_job(self.job(text_width(width))));
        Chrome::new(ctx).height() + galley.size().y
    }

    /// 內文不換行時的卡片寬度（DIP）。
    fn natural_width(&self, ctx: &Context) -> f32 {
        let galley = ctx.fonts_mut(|f| f.layout_job(self.job(f32::INFINITY)));
        galley.size().x + 2.0 * BORDER + 2.0 * PAD.x + scrollbar::WIDTH
    }

    fn job(&self, wrap: f32) -> LayoutJob {
        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap;
        for segment in &self.segments {
            let underline = match segment.url {
                Some(_) => Stroke::new(1.0, FG_UPDATE),
                None => Stroke::NONE,
            };
            job.append(
                &segment.text,
                0.0,
                TextFormat {
                    font_id: font(segment.pt),
                    color: segment.color,
                    underline,
                    ..Default::default()
                },
            );
        }
        job
    }

    /// 卡片顯示的全部文字：原文＋空行＋譯文，或單純譯文／提示／錯誤（連結只算顯示文字）。
    pub fn all_text(&self) -> String {
        self.segments.iter().map(|s| s.text.as_str()).collect()
    }

    /// 正規化後的選取範圍（字元索引 `[lo, hi)`）；沒有選取或收成游標則 None。
    fn span(&self) -> Option<(usize, usize)> {
        match (self.anchor, self.focus) {
            (Some(a), Some(f)) if a != f => Some((a.min(f), a.max(f))),
            _ => None,
        }
    }

    fn selection_text(&self) -> Option<String> {
        let (lo, hi) = self.span()?;
        Some(char_range_text(&self.all_text(), lo, hi))
    }

    /// 第 `index` 個字元所在連結的網址。
    fn link_at(&self, index: usize) -> Option<&str> {
        let mut start = 0;
        for segment in &self.segments {
            let range = start..start + segment.text.chars().count();
            if range.contains(&index) {
                return segment.url.as_deref();
            }
            start = range.end;
        }
        None
    }

    /// 畫出卡片並處理滑鼠與複製；卡片佔滿 `ui` 的範圍。
    pub fn content(&mut self, ui: &mut egui::Ui) -> Option<CardAction> {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        // 起訖點取按鍵事件自帶的位置：eframe 會把兩次重繪間的事件併成一幀
        let (buttons, right_clicks, pointer, down, time, copy) = ctx.input(|i| {
            let mut buttons = Vec::new();
            let mut right_clicks = Vec::new();
            for event in &i.events {
                if let Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } = event
                {
                    match button {
                        PointerButton::Primary => buttons.push((*pos, *pressed)),
                        PointerButton::Secondary if *pressed => right_clicks.push(*pos),
                        _ => {}
                    }
                }
            }
            (
                buttons,
                right_clicks,
                i.pointer.latest_pos(),
                i.pointer.primary_down(),
                i.time,
                i.events.iter().any(is_copy),
            )
        });

        // ---- 版面：✕ 列在上、提示列在下，內文與捲軸夾在中間 ----
        let area = ui.max_rect();
        let inner = area.shrink(BORDER);
        let chrome = Chrome::new(&ctx);
        let close_size = chrome.close.size() + 2.0 * vec2(CLOSE_PAD_X, CLOSE_PAD_Y);
        let close_rect =
            egui::Rect::from_min_size(pos2(inner.max.x - close_size.x, inner.min.y), close_size);
        let hint_pos = pos2(
            inner.min.x + PAD.x,
            inner.max.y - PAD.y - chrome.hint.size().y,
        );
        let text_rect = egui::Rect::from_min_max(
            pos2(inner.min.x + PAD.x, close_rect.max.y + PAD.y),
            pos2(inner.max.x - PAD.x - scrollbar::WIDTH, hint_pos.y - PAD.y),
        );
        let track = egui::Rect::from_min_max(
            pos2(text_rect.max.x, text_rect.min.y),
            pos2(text_rect.max.x + scrollbar::WIDTH, text_rect.max.y),
        );

        // ---- 捲動：捲軸拖曳、滾輪、拖曳選字到內文外的自動捲動 ----
        let galley = ctx.fonts_mut(|f| f.layout_job(self.job(text_rect.width())));
        let (content_h, view_h) = (galley.size().y, text_rect.height());
        if let Some(fraction) = self.scrollbar.drag(ui, track) {
            self.scroll = fraction as f32 * content_h;
        }
        if ui.rect_contains_pointer(area) {
            self.scroll -= wheel_delta(ui, view_h);
        }
        if self.dragging
            && let Some(p) = pointer
        {
            if time >= self.next_tick {
                self.next_tick = time + AUTOSCROLL_INTERVAL;
                self.scroll += autoscroll_pixels(
                    p.y.floor() as i32,
                    text_rect.min.y as i32,
                    text_rect.max.y as i32 - 1,
                ) as f32;
            }
            ctx.request_repaint_after(Duration::from_secs_f64(AUTOSCROLL_INTERVAL));
        }
        self.scroll = self.scroll.clamp(0.0, (content_h - view_h).max(0.0));
        let origin = text_rect.min - vec2(0.0, self.scroll);
        let caret_at = |p: Pos2| galley.cursor_from_pos(p - origin).index.0;

        // ---- 互動：✕、點擊與拖曳選字、右鍵選單、Ctrl+C ----
        let mut close = false;
        let mut copied = None;
        let mut focus = false;
        let menu_rect = self.menu.map(|at| copy_menu::rect(ui, at));
        for (pos, pressed) in buttons {
            if pressed {
                // 捲軸與選單上的按放不算點卡片，選取也留著
                if !area.contains(pos)
                    || track.contains(pos)
                    || menu_rect.is_some_and(|r| r.contains(pos))
                {
                    continue;
                }
                if close_rect.contains(pos) {
                    close = true;
                    continue;
                }
                self.press = Some(pos);
                self.anchor = None;
                self.focus = None;
                if text_rect.contains(pos) {
                    let index = caret_at(pos);
                    self.anchor = Some(index);
                    self.focus = Some(index);
                    self.dragging = true;
                    self.next_tick = time + AUTOSCROLL_INTERVAL;
                    if let Some(url) =
                        char_under(&galley, pos - origin).and_then(|i| self.link_at(i))
                    {
                        log(format!("[ui] opening link from message: {url}"));
                        ctx.open_url(OpenUrl::new_tab(url));
                    }
                }
            } else if let Some(start) = self.press.take() {
                if self.dragging {
                    self.dragging = false;
                    self.focus = Some(caret_at(pos));
                }
                let d = (pos - start) * ppp;
                if is_click(d.x.round() as i32, d.y.round() as i32) {
                    close = true;
                } else {
                    focus = true;
                }
            }
        }
        if self.dragging {
            if let Some(p) = pointer {
                self.focus = Some(caret_at(p));
            }
            self.dragging = down;
        }
        if let Some(at) = self.menu {
            match copy_menu::show(ui, Id::unique("region_card_copy_menu"), at) {
                MenuOutcome::Open => {}
                MenuOutcome::Copy => {
                    self.close_menu();
                    copied = match self.selection_text() {
                        Some(selected) => self.copy(&selected, "selection"),
                        None => self.copy(&self.all_text(), "text"),
                    };
                }
                MenuOutcome::Dismissed => self.close_menu(),
            }
        }
        if let Some(&at) = right_clicks.iter().rfind(|p| text_rect.contains(**p)) {
            self.menu = Some(at);
        }
        if copy && let Some(selected) = self.selection_text() {
            copied = self.copy(&selected, "selection").or(copied);
        }

        // ---- 繪製 ----
        let painter = ui.painter().clone();
        painter.rect_filled(inner, 0.0, BG.gamma_multiply(self.alpha));
        let border = Stroke::new(BORDER, GRIP.gamma_multiply(self.alpha));
        painter.rect_stroke(area, 0.0, border, StrokeKind::Inside);
        if ui.rect_contains_pointer(area) {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
        ui.interact(close_rect, Id::unique("region_card_close"), Sense::hover())
            .on_hover_text(t("tooltip.close", &[]));
        painter.galley(
            close_rect.min + vec2(CLOSE_PAD_X, CLOSE_PAD_Y),
            chrome.close,
            FG_BAR,
        );
        painter.galley(hint_pos, chrome.hint, FG_PENDING);
        let text_painter = painter.with_clip_rect(text_rect);
        if let Some((lo, hi)) = self.span() {
            for mark in highlight_rects(&galley, lo, hi) {
                text_painter.rect_filled(mark.translate(origin.to_vec2()), 0.0, SELECT_BG);
            }
        }
        text_painter.galley(origin, galley, FG_TRANSLATED);
        let (first, last) = if content_h > view_h {
            let top = self.scroll / content_h;
            (top, top + view_h / content_h)
        } else {
            (0.0, 1.0)
        };
        self.scrollbar
            .paint(ui, track, f64::from(first), f64::from(last));

        if close {
            Some(CardAction::Close)
        } else if let Some(chars) = copied {
            Some(CardAction::Copied(chars))
        } else {
            focus.then_some(CardAction::FocusForCopy)
        }
    }

    /// 寫進剪貼簿並回傳字數；空字串不寫 —— 會清掉使用者原本的剪貼簿內容。
    fn copy(&mut self, text: &str, kind: &str) -> Option<usize> {
        if text.is_empty() || !(self.clipboard)(text) {
            return None;
        }
        let chars = text.chars().count();
        log(format!("[region] copied {kind} ({chars} chars)"));
        Some(chars)
    }

    fn close_menu(&mut self) {
        if self.menu.take().is_some() {
            log("[ui] copy popup hidden");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use eframe::egui::{
        Key, Modifiers, MouseWheelSource, MouseWheelUnit, OutputCommand, Shape, TouchPhase,
        epaint::TextShape,
    };
    use egui_kittest::Harness;

    use super::*;
    use crate::ui::fonts_egui;
    use crate::ui::overlay::scrollbar::{THUMB, THUMB_HOVER};

    const RECT: Rect = (300, 200, 400, 120);
    const AREA: Rect = (0, 0, 1920, 1040);
    /// 上下都塞不下長譯文的矮工作區。
    const SHORT_AREA: Rect = (0, 0, 1920, 300);
    const LONG: &str = "這一行譯文很長，長到用矩形的寬度排會換成好幾行，";
    const SOURCE: &str = "Talk to Merle";
    const BOTH: &str = "Talk to Merle\n\n譯文";

    struct State {
        card: Card,
        actions: Vec<CardAction>,
        copied: Rc<RefCell<Vec<String>>>,
        area: egui::Rect,
    }

    fn harness_sized(card: Card, size: Vec2) -> Harness<'static, State> {
        let copied = Rc::new(RefCell::new(Vec::new()));
        let mut card = card;
        let sink = Rc::clone(&copied);
        card.set_clipboard(move |text| {
            sink.borrow_mut().push(text.to_owned());
            true
        });
        let state = State {
            card,
            actions: Vec::new(),
            copied,
            area: egui::Rect::NOTHING,
        };
        let mut h = Harness::builder()
            .with_size(size)
            .with_context_setup(|ctx| {
                fonts_egui::install(ctx, "zh-TW");
                ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
            })
            .build_ui_state(
                |ui, s: &mut State| {
                    s.area = ui.max_rect();
                    s.actions.extend(s.card.content(ui));
                },
                state,
            );
        h.step();
        h.step();
        h
    }

    fn harness(content: CardContent) -> Harness<'static, State> {
        let mut card = Card::new(RECT, 0.8);
        card.set_content(content);
        harness_sized(card, vec2(400.0, 220.0))
    }

    fn text(text: &str, source: &str) -> CardContent {
        CardContent::Text {
            text: text.to_owned(),
            source: source.to_owned(),
        }
    }

    fn copied(h: &Harness<State>) -> Vec<String> {
        h.state().copied.borrow().clone()
    }

    fn text_shapes(h: &Harness<State>) -> Vec<TextShape> {
        h.output()
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Text(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    fn shape_with(h: &Harness<State>, text: &str) -> Option<TextShape> {
        text_shapes(h).into_iter().find(|s| s.galley.text() == text)
    }

    /// 內文 galley 每一段的（文字、顏色、字級、底線色）。
    fn sections(h: &Harness<State>, text: &str) -> Vec<(String, Color32, f32, Color32)> {
        let shape = shape_with(h, text).unwrap_or_else(|| panic!("no text shape {text:?}"));
        let job = &shape.galley.job;
        job.sections
            .iter()
            .map(|s| {
                (
                    job.text[s.byte_range.start.0..s.byte_range.end.0].to_owned(),
                    s.format.color,
                    s.format.font_id.size,
                    s.format.underline.color,
                )
            })
            .collect()
    }

    fn colours(h: &Harness<State>, text: &str) -> Vec<Color32> {
        sections(h, text).into_iter().map(|s| s.1).collect()
    }

    fn label_rect(h: &Harness<State>, text: &str) -> egui::Rect {
        let shape = shape_with(h, text).unwrap();
        egui::Rect::from_min_size(shape.pos, shape.galley.size())
    }

    fn highlights(h: &Harness<State>) -> Vec<egui::Rect> {
        h.output()
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Rect(r) if r.fill == SELECT_BG => Some(r.rect),
                _ => None,
            })
            .collect()
    }

    fn press(h: &mut Harness<State>, p: Pos2) {
        h.hover_at(p);
        h.step();
        h.drag_at(p);
        h.step();
    }

    fn release(h: &mut Harness<State>, p: Pos2) {
        h.drop_at(p);
        h.step();
        h.step();
    }

    fn click(h: &mut Harness<State>, p: Pos2) {
        press(h, p);
        release(h, p);
    }

    fn drag(h: &mut Harness<State>, from: Pos2, to: Pos2) {
        press(h, from);
        h.hover_at(to);
        h.step();
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
        h.step();
    }

    fn ctrl_c(h: &mut Harness<State>) {
        h.event(Event::Copy);
        h.step();
    }

    fn menu_rect(h: &Harness<State>) -> Option<egui::Rect> {
        shape_with(h, &t("menu.copy", &[]))
            .map(|s| egui::Rect::from_min_size(s.pos, s.galley.size()).expand2(vec2(4.0, 2.0)))
    }

    fn menu_copy(h: &mut Harness<State>, at: Pos2) {
        right_click(h, at);
        let menu = menu_rect(h).expect("menu shown");
        click(h, menu.center());
    }

    /// 從原文第一個字拖到原文那一行右邊的字外：選到整行原文。
    fn select_source(h: &mut Harness<State>, all: &str) {
        let label = label_rect(h, all);
        let from = label.left_top() + vec2(0.5, 5.0);
        let to = Pos2::new(h.state().area.max.x - 30.0, from.y);
        drag(h, from, to);
    }

    #[test]
    fn pending_shows_the_working_notice_in_the_pending_colour() {
        let h = harness(CardContent::Pending);
        let working = t("region.working", &[]);
        assert_eq!(colours(&h, &working), [FG_PENDING]);
        assert_eq!(h.state().card.all_text(), working);
    }

    #[test]
    fn text_replaces_the_pending_notice_in_the_translated_colour() {
        let mut h = harness(CardContent::Pending);
        h.state_mut()
            .card
            .set_content(text("譯文第一行\n第二行", ""));
        h.step();
        assert_eq!(colours(&h, "譯文第一行\n第二行"), [FG_TRANSLATED]);
        assert_eq!(h.state().card.all_text(), "譯文第一行\n第二行");
    }

    #[test]
    fn source_sits_dimmed_and_smaller_above_the_translation_after_a_blank_line() {
        let h = harness(text("譯文", SOURCE));
        assert_eq!(h.state().card.all_text(), BOTH);
        let sections = sections(&h, BOTH);
        let (source, translation) = (&sections[0], sections.last().unwrap());
        assert_eq!(
            (source.0.as_str(), source.1, source.2),
            (SOURCE, FG_ORIGINAL, tk_pt(9.0))
        );
        // 分隔的空行與譯文同字型，append 會併成同一段
        assert_eq!(
            (translation.0.as_str(), translation.1, translation.2),
            ("\n\n譯文", FG_TRANSLATED, tk_pt(11.0))
        );
    }

    #[test]
    fn text_without_source_shows_only_the_translation() {
        let mut h = harness(text("譯文", SOURCE));
        h.state_mut().card.set_content(text("譯文", ""));
        h.step();
        assert_eq!(h.state().card.all_text(), "譯文");
        assert!(shape_with(&h, "譯文").is_some());
    }

    #[test]
    fn empty_text_shows_the_no_text_notice() {
        let h = harness(text("", ""));
        let notice = t("region.no_text", &[]);
        assert_eq!(colours(&h, &notice), [FG_PENDING]);
        assert_eq!(h.state().card.all_text(), notice);
    }

    #[test]
    fn error_is_red() {
        let h = harness(CardContent::Error("HTTP 401: bad key".into()));
        assert_eq!(colours(&h, "HTTP 401: bad key"), [FG_ERROR]);
        assert_eq!(h.state().card.all_text(), "HTTP 401: bad key");
    }

    #[test]
    fn urls_are_underlined_links_and_markup_shows_only_its_text() {
        let message = "Get a key at https://example.com/keys now";
        let h = harness(CardContent::Error(message.into()));
        assert_eq!(h.state().card.all_text(), message);
        let link = sections(&h, message)
            .into_iter()
            .find(|s| s.0 == "https://example.com/keys")
            .expect("link section");
        assert_eq!((link.1, link.3), (FG_UPDATE, FG_UPDATE));
        assert_eq!(colours(&h, message)[0], FG_ERROR);

        let h = harness(text("[docs](https://a.example) here", ""));
        assert_eq!(h.state().card.all_text(), "docs here");
        assert!(shape_with(&h, "docs here").is_some());
    }

    /// 不換行時 `text` 以 `pt` 排出的寬度，加上卡片自己的邊框、內距與捲軸。
    fn width_of(h: &Harness<State>, text: &str, pt: f32) -> f32 {
        let galley = h
            .ctx
            .fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font(pt), Color32::WHITE));
        galley.size().x + 2.0 * BORDER + 2.0 * PAD.x + scrollbar::WIDTH
    }

    #[test]
    fn rich_label_natural_width_is_the_widest_line_unwrapped() {
        let h = harness(text(
            "短
這是一行比較長的文字",
            "",
        ));
        let natural = h.state().card.natural_width(&h.ctx);
        assert_eq!(natural, width_of(&h, "這是一行比較長的文字", 11.0));
    }

    #[test]
    fn rich_label_natural_width_measures_each_block_with_its_own_font() {
        let h = harness(text("同樣的字", "同樣的字"));
        let natural = h.state().card.natural_width(&h.ctx);
        assert_eq!(natural, width_of(&h, "同樣的字", 11.0));
        assert!(natural > width_of(&h, "同樣的字", 9.0));
    }

    #[test]
    fn rich_label_natural_width_counts_the_link_label_not_the_markup() {
        let h = harness(text("[點我](https://a.example/very/long/path)", ""));
        let natural = h.state().card.natural_width(&h.ctx);
        assert_eq!(natural, width_of(&h, "點我", 11.0));
    }

    #[test]
    fn clicking_a_link_opens_it_and_closes_the_card() {
        let mut h = harness(CardContent::Error("https://example.com/keys".into()));
        let link = label_rect(&h, "https://example.com/keys");
        press(&mut h, link.left_top() + vec2(5.0, 5.0));
        let opened: Vec<String> = h
            .output()
            .platform_output
            .commands
            .iter()
            .filter_map(|c| match c {
                OutputCommand::OpenUrl(open) => Some(open.url.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(opened, ["https://example.com/keys"]);
        release(&mut h, link.left_top() + vec2(5.0, 5.0));
        assert_eq!(h.state().actions, [CardAction::Close]);
    }

    #[test]
    fn close_affordances_are_shown_apart_from_the_content() {
        let h = harness(CardContent::Pending);
        assert!(shape_with(&h, "✕").is_some());
        assert!(shape_with(&h, &t("region.close_hint", &[])).is_some());
        assert_eq!(h.state().card.all_text(), t("region.working", &[]));
    }

    #[test]
    fn close_button_tooltip_says_close() {
        let mut h = harness(CardContent::Pending);
        let close = label_rect(&h, "✕");
        h.hover_at(close.center());
        for _ in 0..3 {
            h.step();
        }
        assert!(shape_with(&h, &t("tooltip.close", &[])).is_some());
    }

    #[test]
    fn background_follows_the_alpha() {
        let mut h = harness(CardContent::Pending);
        let fills = |h: &Harness<State>| {
            h.output()
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    Shape::Rect(r) => Some(r.fill),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(fills(&h).contains(&BG.gamma_multiply(0.8)));
        h.state_mut().card.set_alpha(0.5);
        h.step();
        assert!(fills(&h).contains(&BG.gamma_multiply(0.5)));
    }

    #[test]
    fn border_surrounds_the_background_instead_of_stacking_under_it() {
        let h = harness(CardContent::Pending);
        let grip = GRIP.gamma_multiply(0.8);
        let rects = || {
            h.output().shapes.iter().filter_map(|c| match &c.shape {
                Shape::Rect(r) => Some(r.clone()),
                _ => None,
            })
        };
        // 半透明的底色疊在半透明的邊框色上，合成後的不透明度會比 overlay 高
        assert!(!rects().any(|r| r.fill == grip));
        assert!(rects().any(|r| r.stroke.color == grip));
    }

    #[test]
    fn card_sits_below_the_rect_with_its_width() {
        let h = harness(CardContent::Pending);
        let ctx = h.ctx.clone();
        let mut card = Card::new(RECT, 0.8);
        let placed = card.layout(&ctx, AREA);
        // 「辨識中」只有一行：自然高度比下限矮
        assert_eq!(
            placed,
            Placement {
                x: 300,
                y: 200 + 120 + ANCHOR_GAP,
                w: 400,
                h: MIN_HEIGHT,
                side: Side::Below,
            }
        );
        let narrow = Card::new((300, 200, 40, 20), 0.8).layout(&ctx, AREA);
        assert_eq!(narrow.w, MIN_WIDTH);
    }

    #[test]
    fn card_flips_above_when_no_room_below() {
        let placed = place((300, 900, 400, 120), 400, 200, 400, AREA, None);
        assert_eq!(
            placed,
            Placement {
                x: 300,
                y: 900 - 200 - ANCHOR_GAP,
                w: 400,
                h: 200,
                side: Side::Above,
            }
        );
    }

    #[test]
    fn card_moves_to_the_wider_side_when_neither_fits() {
        let placed = place((300, 100, 400, 900), 400, 200, 900, AREA, None);
        assert_eq!((placed.side, placed.w), (Side::Right, 900));
        // 已在側邊時直接沿用側邊，不再判上下
        let sticky = place(RECT, 400, 50, 900, AREA, Some(Side::Right));
        assert_eq!(sticky.side, Side::Right);

        let h = harness(CardContent::Pending);
        let ctx = h.ctx.clone();
        let mut card = Card::new((300, 100, 400, 100), 0.8);
        card.set_content(text(&LONG.repeat(2), ""));
        let placed = card.layout(&ctx, SHORT_AREA);
        assert_eq!(placed.side, Side::Right);
        assert!(placed.w > 400, "side card takes the text width: {placed:?}");
    }

    #[test]
    fn side_placement_sticks_until_the_content_changes() {
        let h = harness(CardContent::Pending);
        let ctx = h.ctx.clone();
        let area = (0, 0, 1920, 380);
        let rect = (300, 100, 400, 100);
        let mut card = Card::new(rect, 0.8);
        card.set_content(text(&LONG.repeat(6), ""));
        let first = card.layout(&ctx, area);
        assert_eq!(first.side, Side::Right);
        // 前提：以側邊的寬度換行後矮到放得下矩形下方
        let ppp = ctx.pixels_per_point();
        let rewrapped = (card.natural_height(&ctx, first.w as f32 / ppp) * ppp).ceil() as i32;
        assert_eq!(
            place(rect, 400, rewrapped, first.w, area, None).side,
            Side::Below
        );
        for _ in 0..5 {
            assert_eq!(card.layout(&ctx, area).side, Side::Right);
        }
        card.set_content(text("短", ""));
        assert_eq!(card.layout(&ctx, area).side, Side::Below);
    }

    const LINES: usize = 60;

    fn capped() -> Harness<'static, State> {
        let h = harness(CardContent::Pending);
        let ctx = h.ctx.clone();
        let mut card = Card::new((300, 100, 400, 100), 0.8);
        let lines: Vec<String> = (0..LINES).map(|i| format!("第 {i} 行")).collect();
        card.set_content(text(&lines.join("\n"), ""));
        let placed = card.layout(&ctx, SHORT_AREA);
        assert_eq!(placed.h, SHORT_AREA.3);
        harness_sized(card, vec2(placed.w as f32, placed.h as f32))
    }

    fn thumb(h: &Harness<State>) -> Option<egui::Rect> {
        h.output().shapes.iter().find_map(|c| match &c.shape {
            Shape::Rect(r) if r.fill == THUMB || r.fill == THUMB_HOVER => Some(r.rect),
            _ => None,
        })
    }

    /// 捲動中內文 galley 的左上角。
    fn text_origin(h: &Harness<State>) -> Pos2 {
        text_shapes(h)
            .into_iter()
            .find(|s| s.galley.text().starts_with("第 0 行"))
            .unwrap()
            .pos
    }

    #[test]
    fn capped_card_scrolls_with_the_wheel_and_shows_a_thin_scrollbar() {
        let mut h = capped();
        let thumb = thumb(&h).expect("thumb shown");
        assert!(thumb.width() <= scrollbar::WIDTH);
        let before = text_origin(&h).y;
        h.hover_at(h.state().area.center());
        h.event(Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0.0, -3.0),
            phase: TouchPhase::Move,
            source: MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        });
        h.step();
        h.step();
        assert!(text_origin(&h).y < before, "content should scroll up");
    }

    #[test]
    fn clicking_the_scrollbar_keeps_the_card_and_the_selection() {
        let mut h = capped();
        let first_line = "第 0 行";
        let origin = text_origin(&h);
        let to = Pos2::new(h.state().area.max.x - 30.0, origin.y + 5.0);
        drag(&mut h, origin + vec2(0.5, 5.0), to);
        h.state_mut().actions.clear();
        let thumb = thumb(&h).unwrap();
        click(&mut h, thumb.center());
        assert!(h.state().actions.is_empty(), "{:?}", h.state().actions);
        ctrl_c(&mut h);
        assert_eq!(copied(&h), [first_line]);
    }

    #[test]
    fn dragging_below_a_capped_card_autoscrolls_and_grows_the_selection() {
        let mut h = capped();
        let origin = text_origin(&h);
        press(&mut h, origin + vec2(0.5, 5.0));
        let below = Pos2::new(origin.x + 20.0, h.state().area.max.y - 3.0);
        h.hover_at(below);
        for _ in 0..30 {
            h.step();
        }
        assert!(text_origin(&h).y < origin.y, "should scroll down");
        release(&mut h, below);
        ctrl_c(&mut h);
        let selected = copied(&h).pop().expect("selection copied");
        assert!(selected.starts_with("第 0 行\n第 1 行"), "{selected:?}");
        assert!(selected.lines().count() > 15, "{selected:?}");
    }

    #[test]
    fn a_click_closes_and_a_drag_does_not() {
        let mut h = harness(text("很長很長的譯文內容", SOURCE));
        let label = label_rect(&h, "Talk to Merle\n\n很長很長的譯文內容");
        click(&mut h, label.left_top() + vec2(2.0, 5.0));
        assert_eq!(h.state().actions, [CardAction::Close]);

        let mut h = harness(text("很長很長的譯文內容", SOURCE));
        let from = label.left_top() + vec2(2.0, 5.0);
        drag(&mut h, from, from + vec2(80.0, 0.0));
        assert!(!h.state().actions.contains(&CardAction::Close));
    }

    #[test]
    fn drag_selection_asks_to_borrow_focus() {
        let mut h = harness(text("很長很長的譯文內容", SOURCE));
        let label = label_rect(&h, "Talk to Merle\n\n很長很長的譯文內容");
        let from = label.left_top() + vec2(2.0, 5.0);
        drag(&mut h, from, from + vec2(80.0, 0.0));
        assert_eq!(h.state().actions, [CardAction::FocusForCopy]);
        assert!(!highlights(&h).is_empty());
    }

    #[test]
    fn close_button_closes_on_press() {
        let mut h = harness(CardContent::Pending);
        let close = label_rect(&h, "✕");
        press(&mut h, close.center());
        assert_eq!(h.state().actions, [CardAction::Close]);
    }

    #[test]
    fn right_click_does_not_close_the_card() {
        let mut h = harness(text("譯文", SOURCE));
        let label = label_rect(&h, BOTH);
        right_click(&mut h, label.left_top() + vec2(5.0, 5.0));
        assert!(h.state().actions.is_empty());
        assert!(menu_rect(&h).is_some(), "copy menu shown");
    }

    #[test]
    fn right_click_menu_copies_selection_or_everything() {
        let all = "Talk to Merle\n\n譯文第一行";
        let mut h = harness(text("譯文第一行", SOURCE));
        let label = label_rect(&h, all);
        menu_copy(&mut h, label.center());
        assert_eq!(copied(&h), [all]);
        assert_eq!(h.state().actions, [CardAction::Copied(all.chars().count())]);
        assert!(menu_rect(&h).is_none(), "menu closes after copying");

        select_source(&mut h, all);
        menu_copy(&mut h, label.center());
        assert_eq!(copied(&h), [all, SOURCE]);
        // 複製後選取留著，畫面上的反白與 Ctrl+C 複製到的一致
        assert!(!highlights(&h).is_empty());
        ctrl_c(&mut h);
        assert_eq!(copied(&h), [all, SOURCE, SOURCE]);
    }

    #[test]
    fn dragging_past_the_text_selects_to_its_end() {
        let mut h = harness(text("譯文", SOURCE));
        let label = label_rect(&h, BOTH);
        let below = Pos2::new(h.state().area.max.x - 30.0, h.state().area.max.y - 3.0);
        drag(&mut h, label.left_top() + vec2(0.5, 5.0), below);
        ctrl_c(&mut h);
        assert_eq!(copied(&h), [BOTH]);
    }

    #[test]
    fn a_collapsed_selection_is_no_selection() {
        let mut h = harness(text("譯文", SOURCE));
        let label = label_rect(&h, BOTH);
        // 拖過門檻再拉回原處：位移算拖曳，但兩端落在同一個字元邊界
        let from = label.left_top() + vec2(0.5, 5.0);
        press(&mut h, from);
        h.hover_at(from + vec2(60.0, 0.0));
        h.step();
        h.hover_at(from + vec2(-8.0, 0.0));
        h.step();
        release(&mut h, from + vec2(-8.0, 0.0));
        assert_eq!(h.state().actions, [CardAction::FocusForCopy]);
        assert!(highlights(&h).is_empty());
        ctrl_c(&mut h);
        assert!(
            copied(&h).is_empty(),
            "nothing selected, clipboard untouched"
        );
        menu_copy(&mut h, label.center());
        assert_eq!(copied(&h), [BOTH]);
    }

    #[test]
    fn new_content_drops_the_old_selection() {
        let mut h = harness(text("譯文", SOURCE));
        select_source(&mut h, BOTH);
        h.state_mut().card.set_content(text("新譯文", ""));
        h.step();
        assert!(highlights(&h).is_empty());
        let label = label_rect(&h, "新譯文");
        menu_copy(&mut h, label.center());
        assert_eq!(copied(&h), ["新譯文"]);
    }

    #[test]
    fn ctrl_c_copies_the_selection_through_our_clipboard() {
        let mut h = harness(text("譯文", SOURCE));
        ctrl_c(&mut h);
        assert!(
            copied(&h).is_empty(),
            "nothing selected, clipboard untouched"
        );

        select_source(&mut h, BOTH);
        h.state_mut().actions.clear();
        ctrl_c(&mut h);
        assert_eq!(copied(&h), [SOURCE]);
        assert_eq!(h.state().actions, [CardAction::Copied(13)]);
        // 大寫 C（Ctrl+Shift+C）到 egui 是帶 Shift 的 Key 事件
        h.key_press_modifiers(Modifiers::CTRL | Modifiers::SHIFT, Key::C);
        h.step();
        assert_eq!(copied(&h), [SOURCE, SOURCE]);
    }
}
