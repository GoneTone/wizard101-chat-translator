//! 框選翻譯的結果卡：辨識中／譯文／錯誤三態，貼在框選矩形下方，放不下翻到上方，再不行移到左右較寬的一側。
//!
//! 這裡只有狀態與 egui 內容；視窗由宿主托管。點一下或點 ✕ 關閉，拖曳是選字；原文與譯文同在一顆
//! 可選取的 label 裡，選取能一路跨過兩段。

use std::sync::Arc;

use eframe::egui::containers::scroll_area::{ScrollBarVisibility, ScrollSource};
use eframe::egui::plugin::Plugin;
use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{
    self, Color32, Context, CursorIcon, Event, FontId, Galley, Id, Key, Label, OutputCommand,
    PointerButton, Pos2, RawInput, ScrollArea, Sense, UiBuilder, ViewportIdMap, pos2, vec2,
};

use crate::i18n::t;
use crate::logs::log;
use crate::ui::copy_menu::{self, MenuOutcome};
use crate::ui::fonts_egui::tk_pt;
use crate::ui::geometry::{Placement, Rect, Side, anchored_geometry, beside_geometry, is_click};
use crate::ui::overlay::scrollbar::{self, Scrollbar};
use crate::ui::palette::{BG, FG_BAR, FG_ERROR, FG_ORIGINAL, FG_PENDING, FG_TRANSLATED, GRIP};
use crate::ui::region::region_box::{HANDLE, MARGIN};

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
const PAD: egui::Vec2 = vec2(PAD_X as f32, PAD_Y as f32);

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

/// 卡片視窗收到、但要繞過 egui 交給卡片的輸入與輸出。
#[derive(Default)]
struct Inbox {
    right_clicks: Vec<Pos2>,
    copies: Vec<String>,
    settled: Option<String>,
}

/// 只轉接顯示過卡片的 viewport；egui 內建的 label 選取比它先註冊、先跑，這裡的 `on_end_pass` 才收得到複製。
#[derive(Default)]
struct Bridge {
    inboxes: ViewportIdMap<Inbox>,
}

impl Bridge {
    /// 取走目前 viewport 累積的事件，並從此開始轉接這個 viewport。
    fn take(ctx: &Context) -> Inbox {
        let viewport = ctx.viewport_id();
        let handle = ctx.plugin_or_default::<Self>();
        let mut bridge = handle.lock();
        std::mem::take(bridge.inboxes.entry(viewport).or_default())
    }
}

impl Plugin for Bridge {
    fn debug_name(&self) -> &'static str {
        "RegionCardBridge"
    }

    fn input_hook(&mut self, _ctx: &Context, input: &mut RawInput) {
        let Some(inbox) = self.inboxes.get_mut(&input.viewport_id) else {
            return;
        };
        // 右鍵按下會把 label 的選取收成游標；選單要的是右鍵之前的選取，所以右鍵不交給 egui
        input.events.retain(|event| match event {
            Event::PointerButton {
                pos,
                button: PointerButton::Secondary,
                pressed,
                ..
            } => {
                if *pressed {
                    inbox.right_clicks.push(*pos);
                }
                false
            }
            _ => true,
        });
    }

    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(inbox) = self.inboxes.get_mut(&ctx.viewport_id()) else {
            return;
        };
        let before = inbox.copies.len();
        // eframe 沒開 clipboard，egui 自己的複製送不出去：攔下來改走我們的剪貼簿
        ctx.output_mut(|o| {
            o.commands.retain(|command| match command {
                OutputCommand::CopyText(text) => {
                    inbox.copies.push(text.clone());
                    false
                }
                OutputCommand::TextSelectionSettled(text) => {
                    inbox.settled = Some(text.clone());
                    true
                }
                _ => true,
            });
        });
        if inbox.copies.len() > before {
            ctx.request_repaint();
        }
    }
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

/// 結果卡的狀態；每次框選建一張新的。
pub struct Card {
    rect: Rect,
    alpha: f32,
    content: CardContent,
    /// 每換一次內容加一：內文 label 換 id，egui 留在舊文字上的選取就自動作廢。
    generation: u64,
    side: Option<Side>,
    placed: Option<Placement>,
    /// 左鍵按下的位置，放開時判斷是點擊還是拖曳。
    press: Option<Pos2>,
    /// egui 回報的 label 選取文字；沒有選取為 None。
    selection: Option<String>,
    /// 右鍵選單的位置、開啟當下要複製的文字與其種類（供 log）。
    menu: Option<(Pos2, String, &'static str)>,
    scrollbar: Scrollbar,
    clipboard: Box<dyn FnMut(&str) -> bool>,
}

impl Card {
    /// 貼在框選矩形 `rect`（螢幕 px）旁、背景不透明度 `alpha`，先顯示辨識中。
    pub fn new(rect: Rect, alpha: f32) -> Self {
        Self {
            rect,
            alpha,
            content: CardContent::Pending,
            generation: 0,
            side: None,
            placed: None,
            press: None,
            selection: None,
            menu: None,
            scrollbar: Scrollbar::default(),
            clipboard: Box::new(|_| false),
        }
    }

    /// 換掉剪貼簿寫入函式（預設寫不進去）；宿主帶入以卡片視窗為擁有者的 `clipboard::set_text`。
    pub fn set_clipboard(&mut self, sink: impl FnMut(&str) -> bool + 'static) {
        self.clipboard = Box::new(sink);
    }

    /// 換內容；落點重新判斷，舊的選取作廢。
    pub fn set_content(&mut self, content: CardContent) {
        self.content = content;
        self.generation += 1;
        self.side = None;
        self.selection = None;
        self.close_menu();
    }

    /// 背景不透明度跟著 overlay 一起變。
    pub fn set_alpha(&mut self, alpha: f32) {
        self.alpha = alpha;
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

    /// 內文各段的（文字、顏色、字級 pt）；有原文時原文在上、空一行再接譯文。
    fn blocks(&self) -> Vec<(String, Color32, f32)> {
        match &self.content {
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
        }
    }

    fn job(&self, wrap: f32) -> LayoutJob {
        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap;
        for (text, color, pt) in self.blocks() {
            job.append(
                &text,
                0.0,
                TextFormat {
                    font_id: font(pt),
                    color,
                    ..Default::default()
                },
            );
        }
        job
    }

    /// 卡片顯示的全部文字：原文＋空行＋譯文，或單純譯文／提示／錯誤。
    pub fn all_text(&self) -> String {
        self.blocks().into_iter().map(|(text, ..)| text).collect()
    }

    /// 畫出卡片並處理滑鼠與複製；卡片佔滿 `ui` 的範圍。
    pub fn content(&mut self, ui: &mut egui::Ui) -> Option<CardAction> {
        let ctx = ui.ctx().clone();
        let inbox = Bridge::take(&ctx);
        if let Some(text) = inbox.settled {
            self.selection = Some(text);
        }
        // 起訖點取按鍵事件自帶的位置：eframe 會把兩次重繪間的事件併成一幀
        let (buttons, escape) = ctx.input(|i| {
            let buttons: Vec<(Pos2, bool)> = i
                .events
                .iter()
                .filter_map(|e| match e {
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        ..
                    } => Some((*pos, *pressed)),
                    _ => None,
                })
                .collect();
            (buttons, i.key_pressed(Key::Escape))
        });
        if escape {
            self.selection = None;
        }

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

        let painter = ui.painter().clone();
        painter.rect_filled(area, 0.0, GRIP.gamma_multiply(self.alpha));
        painter.rect_filled(inner, 0.0, BG.gamma_multiply(self.alpha));
        if ui.rect_contains_pointer(area) {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
        let close = ui
            .interact(close_rect, Id::unique("region_card_close"), Sense::click())
            .on_hover_text(t("tooltip.close", &[]));
        painter.galley(
            close_rect.min + vec2(CLOSE_PAD_X, CLOSE_PAD_Y),
            chrome.close,
            FG_BAR,
        );
        painter.galley(hint_pos, chrome.hint, FG_PENDING);

        let galley = ctx.fonts_mut(|f| f.layout_job(self.job(text_rect.width())));
        let mut scroll = ScrollArea::vertical()
            .id_salt(("region_card_text", self.generation))
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
            .scroll_source(ScrollSource::MOUSE_WHEEL)
            .auto_shrink(false);
        if let Some(fraction) = self.scrollbar.drag(ui, track) {
            scroll = scroll.vertical_scroll_offset(fraction as f32 * galley.size().y);
        }
        let out = ui
            .scope_builder(UiBuilder::new().max_rect(text_rect), |ui| {
                scroll.show(ui, |ui| ui.add(Label::new(galley).selectable(true)))
            })
            .inner;
        let (content_h, view_h) = (out.content_size.y, out.inner_rect.height());
        let (first, last) = if content_h > view_h {
            let top = out.state.clamped_offset().y / content_h;
            (top, top + view_h / content_h)
        } else {
            (0.0, 1.0)
        };
        self.scrollbar
            .paint(ui, track, f64::from(first), f64::from(last));

        // ---- 互動：選單、右鍵、點擊與拖曳、Ctrl+C ----
        let mut close_card = close.clicked();
        let mut copied = None;
        let mut focus = false;
        let menu_rect = self.menu.as_ref().map(|(at, ..)| copy_menu::rect(ui, *at));
        if let Some((at, ..)) = self.menu {
            match copy_menu::show(ui, Id::unique("region_card_copy_menu"), at) {
                MenuOutcome::Open => {}
                MenuOutcome::Copy => {
                    if let Some((_, text, kind)) = self.menu.take() {
                        log("[ui] copy popup hidden");
                        copied = self.copy(&text, kind);
                    }
                }
                MenuOutcome::Dismissed => self.close_menu(),
            }
        }
        if let Some(&at) = inbox
            .right_clicks
            .iter()
            .rfind(|p| out.inner_rect.contains(**p))
        {
            let (text, kind) = match &self.selection {
                Some(selected) => (selected.clone(), "selection"),
                None => (self.all_text(), "text"),
            };
            self.menu = Some((at, text, kind));
        }
        let ppp = ctx.pixels_per_point();
        for (pos, pressed) in buttons {
            if pressed {
                if area.contains(pos)
                    && !track.contains(pos)
                    && !menu_rect.is_some_and(|r| r.contains(pos))
                {
                    self.press = Some(pos);
                    self.selection = None;
                }
            } else if let Some(start) = self.press.take() {
                let d = (pos - start) * ppp;
                if is_click(d.x.round() as i32, d.y.round() as i32) {
                    close_card = true;
                } else {
                    focus = true;
                }
            }
        }
        for text in inbox.copies {
            copied = self.copy(&text, "selection").or(copied);
        }

        if close_card {
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
        Color32, Event, Modifiers, MouseWheelSource, MouseWheelUnit, PointerButton, Shape,
        TouchPhase, Vec2, epaint::TextShape, vec2,
    };
    use egui_kittest::Harness;

    use super::*;
    use crate::i18n::t;
    use crate::ui::fonts_egui::{self, tk_pt};
    use crate::ui::overlay::scrollbar::{self, THUMB, THUMB_HOVER};
    use crate::ui::palette::{FG_ERROR, FG_ORIGINAL, FG_PENDING, FG_TRANSLATED};

    const RECT: Rect = (300, 200, 400, 120);
    const AREA: Rect = (0, 0, 1920, 1040);
    /// 上下都塞不下長譯文的矮工作區。
    const SHORT_AREA: Rect = (0, 0, 1920, 300);
    const LONG: &str = "這一行譯文很長，長到用矩形的寬度排會換成好幾行，";

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

    /// 內文 galley 每一段的（文字、顏色、字級）。
    fn sections(h: &Harness<State>, text: &str) -> Vec<(String, Color32, f32)> {
        let shape = shape_with(h, text).unwrap_or_else(|| panic!("no text shape {text:?}"));
        let job = &shape.galley.job;
        job.sections
            .iter()
            .map(|s| {
                (
                    job.text[s.byte_range.start.0..s.byte_range.end.0].to_owned(),
                    s.format.color,
                    s.format.font_id.size,
                )
            })
            .collect()
    }

    fn colours(h: &Harness<State>, text: &str) -> Vec<Color32> {
        sections(h, text).into_iter().map(|(_, c, _)| c).collect()
    }

    fn label_rect(h: &Harness<State>, text: &str) -> egui::Rect {
        let shape = shape_with(h, text).unwrap();
        egui::Rect::from_min_size(shape.pos, shape.galley.size())
    }

    fn click(h: &mut Harness<State>, p: Pos2) {
        h.hover_at(p);
        h.step();
        h.drag_at(p);
        h.step();
        h.drop_at(p);
        h.step();
    }

    fn drag(h: &mut Harness<State>, from: Pos2, to: Pos2) {
        h.hover_at(from);
        h.step();
        h.drag_at(from);
        h.step();
        h.hover_at(to);
        h.step();
        h.drop_at(to);
        h.step();
        h.step();
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

    fn menu_rect(h: &Harness<State>) -> Option<egui::Rect> {
        let label = t("menu.copy", &[]);
        shape_with(h, &label)
            .map(|s| egui::Rect::from_min_size(s.pos, s.galley.size()).expand2(vec2(4.0, 2.0)))
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
        let h = harness(text("譯文", "Talk to Merle"));
        let all = "Talk to Merle\n\n譯文";
        assert_eq!(h.state().card.all_text(), all);
        let sections = sections(&h, all);
        let (source, translation) = (&sections[0], sections.last().unwrap());
        assert_eq!(
            (source.0.as_str(), source.1, source.2),
            ("Talk to Merle", FG_ORIGINAL, tk_pt(9.0))
        );
        // 分隔的空行與譯文同字型，append 會併成同一段
        assert_eq!(
            (translation.0.as_str(), translation.1, translation.2),
            (
                "

譯文",
                FG_TRANSLATED,
                tk_pt(11.0)
            )
        );
    }

    #[test]
    fn text_without_source_shows_only_the_translation() {
        let mut h = harness(text("譯文", "Talk to Merle"));
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
        assert!(fills(&h).contains(&crate::ui::palette::BG.gamma_multiply(0.8)));
        h.state_mut().card.set_alpha(0.5);
        h.step();
        assert!(fills(&h).contains(&crate::ui::palette::BG.gamma_multiply(0.5)));
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

    fn capped() -> Harness<'static, State> {
        let h = harness(CardContent::Pending);
        let ctx = h.ctx.clone();
        let mut card = Card::new((300, 100, 400, 100), 0.8);
        let lines: Vec<String> = (0..60).map(|i| format!("第 {i} 行")).collect();
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

    #[test]
    fn capped_card_scrolls_with_the_wheel_and_shows_a_thin_scrollbar() {
        let mut h = capped();
        let thumb = thumb(&h).expect("thumb shown");
        assert!(thumb.width() <= scrollbar::WIDTH);
        let first = |h: &Harness<State>| {
            text_shapes(h)
                .into_iter()
                .find(|s| s.galley.text().starts_with("第 0 行"))
                .unwrap()
                .pos
                .y
        };
        let before = first(&h);
        h.hover_at(h.state().area.center());
        h.event(Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0.0, -3.0),
            phase: TouchPhase::Move,
            source: MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        });
        for _ in 0..5 {
            h.step();
        }
        assert!(first(&h) < before, "content should scroll up");
    }

    #[test]
    fn clicking_the_scrollbar_does_not_close_the_card() {
        let mut h = capped();
        let thumb = thumb(&h).unwrap();
        click(&mut h, thumb.center());
        assert!(h.state().actions.is_empty(), "{:?}", h.state().actions);
    }

    #[test]
    fn a_click_closes_and_a_drag_does_not() {
        let mut h = harness(text("很長很長的譯文內容", "Talk to Merle"));
        let label = label_rect(&h, "Talk to Merle\n\n很長很長的譯文內容");
        click(&mut h, label.left_top() + vec2(2.0, 5.0));
        assert_eq!(h.state().actions, [CardAction::Close]);

        let mut h = harness(text("很長很長的譯文內容", "Talk to Merle"));
        let from = label.left_top() + vec2(2.0, 5.0);
        drag(&mut h, from, from + vec2(80.0, 0.0));
        assert!(!h.state().actions.contains(&CardAction::Close));
    }

    #[test]
    fn drag_selection_asks_to_borrow_focus() {
        let mut h = harness(text("很長很長的譯文內容", "Talk to Merle"));
        let label = label_rect(&h, "Talk to Merle\n\n很長很長的譯文內容");
        let from = label.left_top() + vec2(2.0, 5.0);
        drag(&mut h, from, from + vec2(80.0, 0.0));
        assert_eq!(h.state().actions, [CardAction::FocusForCopy]);
    }

    #[test]
    fn close_button_closes() {
        let mut h = harness(CardContent::Pending);
        let close = label_rect(&h, "✕");
        click(&mut h, close.center());
        assert_eq!(h.state().actions, [CardAction::Close]);
    }

    #[test]
    fn right_click_does_not_close_the_card() {
        let mut h = harness(text("譯文", "Talk to Merle"));
        let label = label_rect(&h, "Talk to Merle\n\n譯文");
        right_click(&mut h, label.left_top() + vec2(5.0, 5.0));
        assert!(h.state().actions.is_empty());
        assert!(menu_rect(&h).is_some(), "copy menu shown");
    }

    /// 從原文第一個字拖到原文行尾（原文是最寬的一行）：選到整行原文。
    fn select_source(h: &mut Harness<State>, all: &str) {
        let label = label_rect(h, all);
        let from = label.left_top() + vec2(0.5, 5.0);
        let to = Pos2::new(label.max.x - 0.5, from.y);
        drag(h, from, to);
    }

    #[test]
    fn right_click_menu_copies_selection_or_everything() {
        let all = "Talk to Merle\n\n譯文第一行";
        let mut h = harness(text("譯文第一行", "Talk to Merle"));
        let label = label_rect(&h, all);
        right_click(&mut h, label.center());
        let menu = menu_rect(&h).expect("menu shown");
        click(&mut h, menu.center());
        assert_eq!(*h.state().copied.borrow(), [all]);
        assert_eq!(h.state().actions, [CardAction::Copied(all.chars().count())]);
        assert!(menu_rect(&h).is_none(), "menu closes after copying");

        select_source(&mut h, all);
        right_click(&mut h, label.center());
        let menu = menu_rect(&h).expect("menu shown");
        click(&mut h, menu.center());
        assert_eq!(*h.state().copied.borrow(), [all, "Talk to Merle"]);
    }

    #[test]
    fn new_content_drops_the_old_selection() {
        let all = "Talk to Merle\n\n譯文";
        let mut h = harness(text("譯文", "Talk to Merle"));
        select_source(&mut h, all);
        h.state_mut().card.set_content(text("新譯文", ""));
        h.step();
        let label = label_rect(&h, "新譯文");
        right_click(&mut h, label.center());
        let menu = menu_rect(&h).expect("menu shown");
        click(&mut h, menu.center());
        assert_eq!(*h.state().copied.borrow(), ["新譯文"]);
    }

    #[test]
    fn ctrl_c_copies_the_selection_through_our_clipboard() {
        let all = "Talk to Merle\n\n譯文";
        let mut h = harness(text("譯文", "Talk to Merle"));
        h.event(Event::Copy);
        h.step();
        h.step();
        assert!(
            h.state().copied.borrow().is_empty(),
            "nothing selected, clipboard untouched"
        );

        select_source(&mut h, all);
        h.state_mut().actions.clear();
        h.event(Event::Copy);
        h.step();
        h.step();
        assert_eq!(*h.state().copied.borrow(), ["Talk to Merle"]);
        assert_eq!(h.state().actions, [CardAction::Copied(13)]);
    }

    #[test]
    fn the_card_leaves_room_for_the_box_handles() {
        const { assert!(ANCHOR_GAP >= MARGIN + HANDLE / 2) };
        const { assert!(SIDE_MIN_WIDTH > MIN_WIDTH) };
    }
}
