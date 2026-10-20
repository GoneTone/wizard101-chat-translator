//! overlay 視窗外框：半透明底、標題列（圖示、名稱、狀態、⛶⚙─✕）、拖曳移動、四邊四角縮放與記憶位置。

use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, FontId, Image, Label, Layout, Pos2, Response,
    RichText, Role, Sense, Stroke, TextStyle, TextWrapMode, TextureId, Tooltip, Ui, UiBuilder,
    Vec2, WidgetInfo, WidgetText,
};

use crate::config::Geometry;
use crate::i18n::t;
use crate::logs::log;
use crate::ui::fonts_egui::tk_pt;
use crate::ui::geometry::{
    EDGE, Edge, Rect, centered_position, edge_at, moved_to, point_in_rect, pull_onscreen,
    resized_edge,
};
use crate::ui::icon::app_icon_texture;
use crate::ui::palette::{
    BAR, BG, CLOSE_HOVER, FG_BAR, FG_ERROR, FG_HINT, FG_TRANSLATED, GRIP, OUTLINE, TAB_ACCENT,
    TAB_FG_ACTIVE,
};
use crate::ui::win32;

/// 視窗最小寬度（DIP）。
pub const MIN_WIDTH: i32 = 200;
/// 視窗最小高度（DIP）。
pub const MIN_HEIGHT: i32 = 90;

/// 狀態字的顏色（文字由 i18n 依 state key 取得）。
pub const STATUS_COLORS: [(&str, Color32); 6] = [
    ("locating", Color32::from_rgb(0xe0, 0xb0, 0x50)),
    ("listening", Color32::from_rgb(0x7d, 0xc8, 0x7d)),
    ("translating", TAB_ACCENT),
    ("waiting_game", FG_HINT),
    // 要使用者動手（以管理員重開、更新）才解得掉，用錯誤色
    ("access_denied", FG_ERROR),
    ("version_mismatch", FG_ERROR),
];

/// 標題列高度（DIP）。
pub const BAR_HEIGHT: f32 = 28.0;
const BAR_ICON: f32 = 18.0;
/// 右下角抓取紋的邊長（DIP）。
pub const GRIP_SIZE: f32 = 16.0;

/// 狀態 key 對應的文字（標題列小字與空清單大字共用）。
pub fn status_text(state: &str) -> String {
    t(&format!("status.{state}"), &[])
}

/// 狀態 key 對應的顏色；未知的 key 用標題列文字色。
pub fn status_color(state: &str) -> Color32 {
    STATUS_COLORS
        .iter()
        .find(|(key, _)| *key == state)
        .map_or(FG_BAR, |(_, color)| *color)
}

/// 外框對視窗與游標的操作，一律實體像素；測試換成假的。
pub trait Shell {
    /// 游標的螢幕座標。
    fn cursor(&self) -> (i32, i32);
    /// 視窗目前的矩形。
    fn rect(&self) -> Rect;
    /// 移動／縮放視窗；失敗（例如視窗還不存在）回 `false`。
    fn set_rect(&mut self, rect: Rect) -> bool;
    /// 所有螢幕的矩形。
    fn monitors(&self) -> Vec<Rect>;
    /// 含 (x, y) 那顆螢幕每 DIP 的像素數。
    fn scale_at(&self, x: i32, y: i32) -> f32;
    /// 等 DWM 把已呈現的畫面合成上螢幕；測試裡不必等。
    fn wait_for_composition(&self) {}
    /// 讓 DWM 不顯示（或恢復顯示）視窗；對系統而言視窗仍是可見的。
    fn set_cloaked(&mut self, _cloaked: bool) {}
    /// 讓視窗取得鍵盤焦點（框選的 Ctrl+C 才收得到）；被系統拒絕回 `false`。
    fn take_focus(&mut self) -> bool;
}

/// 以 HWND 操作真正的視窗。
pub struct Win32Shell(pub isize);

impl Shell for Win32Shell {
    fn cursor(&self) -> (i32, i32) {
        win32::cursor_pos()
    }
    fn rect(&self) -> Rect {
        win32::window_rect(self.0)
    }
    fn set_rect(&mut self, rect: Rect) -> bool {
        win32::set_window_rect(self.0, rect)
    }
    fn monitors(&self) -> Vec<Rect> {
        win32::all_monitors()
    }
    fn scale_at(&self, x: i32, y: i32) -> f32 {
        win32::scale_at(x, y)
    }
    fn wait_for_composition(&self) {
        win32::wait_for_composition();
    }
    fn set_cloaked(&mut self, cloaked: bool) {
        win32::set_cloaked(self.0, cloaked);
    }
    fn take_focus(&mut self) -> bool {
        win32::force_foreground(self.0)
    }
}

/// 標題列上被按下的按鈕。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clicked {
    /// ⛶ 開始框選。
    Region,
    /// ⚙ 開設定。
    Settings,
    /// ─ 縮成泡泡。
    Minimize,
    /// ✕ 關閉。
    Close,
}

/// 最小尺寸換算成實體像素（`scale` 為每 DIP 的像素數）。
fn min_size(scale: f32) -> (i32, i32) {
    let px = |dip: i32| (dip as f32 * scale).round() as i32;
    (px(MIN_WIDTH), px(MIN_HEIGHT))
}

/// 啟動時的視窗矩形：沒存過位置就擺主螢幕正中央，存過但整個在畫面外就拉回來。
///
/// `monitors` 是所有螢幕矩形（實體像素，主螢幕含原點），`scale` 是每 DIP 的像素數。
pub fn initial_rect(saved: &Geometry, monitors: &[Rect], scale: f32) -> Rect {
    let (min_w, min_h) = min_size(scale);
    let (w, h) = (saved.width.max(min_w), saved.height.max(min_h));
    let primary = monitors
        .iter()
        .find(|&&(mx, my, mw, mh)| point_in_rect(0, 0, mx, my, mw, mh))
        .or(monitors.first())
        .map_or((0, 0), |&(_, _, mw, mh)| (mw, mh));
    let (cx, cy) = centered_position(primary.0, primary.1, w, h);
    let (x, y) = (saved.x.unwrap_or(cx), saved.y.unwrap_or(cy));
    let (px, py) = pull_onscreen(x, y, w, h, monitors);
    if (px, py) != (x, y) {
        log(format!(
            "[ui] overlay was offscreen, moved back: {w}x{h}+{x}+{y} -> +{px}+{py}"
        ));
    }
    (px, py, w, h)
}

#[derive(Clone, Copy)]
struct Gesture {
    cursor: (i32, i32),
    start: Rect,
    last: Rect,
    /// `None`＝拖曳標題列移動。
    edge: Option<Edge>,
}

/// overlay 的外框；每幀由 `show` 畫出並處理拖曳與縮放。
pub struct Frame {
    alpha: f32,
    initial: Option<Geometry>,
    placement_failed: bool,
    gesture: Option<Gesture>,
    on_geometry_change: Box<dyn FnMut(i32, i32, i32, i32)>,
}

impl Frame {
    /// `initial`（設定裡存的位置大小）在視窗可擺放後經 `initial_rect` 套用一次，`None` 表示不動；
    /// 拖曳或縮放放開時呼叫 `on_geometry_change`。
    pub fn new(
        alpha: f32,
        initial: Option<Geometry>,
        on_geometry_change: impl FnMut(i32, i32, i32, i32) + 'static,
    ) -> Self {
        Self {
            alpha,
            initial,
            placement_failed: false,
            gesture: None,
            on_geometry_change: Box::new(on_geometry_change),
        }
    }

    /// 底色的不透明度。
    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    /// 即時套用新的底色不透明度（文字與標題列恆為不透明）。
    pub fn set_alpha(&mut self, alpha: f32) {
        self.alpha = alpha;
    }

    /// 畫出外框並處理拖曳與縮放；回傳這幀被按下的標題列按鈕。`region_hotkey` 顯示在 ⛶ 的提示框。
    ///
    /// 要在內容之前呼叫：之後加入的元件疊在外框的拖曳區之上、優先接到滑鼠。
    pub fn show(
        &mut self,
        ui: &mut Ui,
        shell: &mut dyn Shell,
        title: &str,
        status: Option<&str>,
        region_hotkey: &str,
    ) -> Option<Clicked> {
        let ppp = ui.ctx().pixels_per_point();
        if let Some(saved) = &self.initial {
            let rect = initial_rect(saved, &shell.monitors(), ppp);
            if shell.set_rect(rect) {
                self.initial = None;
            } else if !self.placement_failed {
                // 視窗還沒建好時會失敗；保留設定值下一幀再試，只記一次
                self.placement_failed = true;
                log(format!(
                    "[ui] overlay placement failed, will retry: geometry={}",
                    describe(rect)
                ));
            }
        }
        let area = ui.max_rect();
        let bar = egui::Rect::from_min_size(area.min, Vec2::new(area.width(), BAR_HEIGHT));
        let grip = grip_rect(area);
        let painter = ui.painter();
        painter.rect_filled(area, 0.0, BG.gamma_multiply(self.alpha));
        painter.rect_filled(bar, 0.0, BAR);
        paint_grip(painter, grip);

        let response = ui.interact(area, ui.scope_id().with("overlay_frame"), Sense::drag());
        let edge_under = |p: Pos2| edge_under(area, p);
        if response.drag_started()
            && let Some(p) = response.interact_pointer_pos()
        {
            let edge = edge_under(p);
            // 標題列以外、也不在邊上的按壓留給內容（框選）
            if edge.is_some() || bar.contains(p) {
                let start = shell.rect();
                if let Some(edge) = edge {
                    log(format!(
                        "[ui] overlay resize start edge={} geometry={}",
                        edge.as_str(),
                        describe(start)
                    ));
                }
                self.gesture = Some(Gesture {
                    cursor: shell.cursor(),
                    start,
                    last: start,
                    edge,
                });
            }
        }
        if let Some(g) = &mut self.gesture {
            if response.dragged() {
                let (cx, cy) = shell.cursor();
                let (dx, dy) = (cx - g.cursor.0, cy - g.cursor.1);
                let (x, y, w, h) = g.start;
                let next = match g.edge {
                    Some(edge) => {
                        let (min_w, min_h) = min_size(ppp);
                        resized_edge(edge, x, y, w, h, dx, dy, min_w, min_h)
                    }
                    None => {
                        let (nx, ny) = moved_to(x, y, dx, dy);
                        (nx, ny, w, h)
                    }
                };
                if next != g.last {
                    g.last = next;
                    shell.set_rect(next);
                }
            }
            if response.drag_stopped() {
                let edge = g.edge;
                self.gesture = None;
                let (x, y, w, h) = shell.rect();
                if let Some(edge) = edge {
                    log(format!(
                        "[ui] overlay resize end edge={} geometry={}",
                        edge.as_str(),
                        describe((x, y, w, h))
                    ));
                }
                (self.on_geometry_change)(x, y, w, h);
            }
        }

        let pointer = response.hover_pos();
        let edge = match self.gesture {
            Some(g) => g.edge,
            None => pointer.and_then(edge_under),
        };
        if let Some(edge) = edge {
            ui.ctx().set_cursor_icon(resize_cursor(edge));
        } else if self.gesture.is_some() || pointer.is_some_and(|p| bar.contains(p)) {
            ui.ctx().set_cursor_icon(CursorIcon::Move);
        }

        let icon = app_icon_texture(ui.ctx(), 48);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(bar)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| title_bar(ui, icon, title, status, region_hotkey),
        )
        .inner
    }
}

/// 視窗 `area` 內的 `p` 壓在哪條縮放邊／角（右下角抓取紋算右下角），都不是則 `None`。
pub fn edge_under(area: egui::Rect, p: Pos2) -> Option<Edge> {
    if grip_rect(area).contains(p) {
        return Some(Edge::Se);
    }
    let v = p - area.min;
    edge_at(
        v.x.floor() as i32,
        v.y.floor() as i32,
        0,
        0,
        area.width() as i32,
        area.height() as i32,
    )
}

fn grip_rect(area: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(area.max - Vec2::splat(GRIP_SIZE), Vec2::splat(GRIP_SIZE))
}

/// 標題列內容；按鈕與狀態先由右往左排，名稱只拿剩下的寬度，太長時被截的是名稱。
fn title_bar(
    ui: &mut Ui,
    icon: TextureId,
    title: &str,
    status: Option<&str>,
    region_hotkey: &str,
) -> Option<Clicked> {
    ui.spacing_mut().item_spacing.x = 0.0;
    let small = FontId::proportional(tk_pt(8.0));
    let bar = ui.max_rect();
    // 視窗上緣與右緣的縮放帶不給按鈕；egui 會把半徑內的按壓吸給附近按鈕，要多讓出這段
    let inset = EDGE as f32 + ui.style().interaction.interact_radius;
    let hit = egui::Rect::from_min_max(
        bar.min + Vec2::new(inset, inset),
        Pos2::new(bar.max.x - inset, bar.max.y),
    );
    let mut clicked = None;
    for (glyph, action, tooltip) in [
        ("✕", Clicked::Close, "tooltip.close"),
        ("─", Clicked::Minimize, "tooltip.minimize"),
        ("⚙", Clicked::Settings, "tooltip.settings"),
        ("⛶", Clicked::Region, "tooltip.region"),
    ] {
        let button = bar_button(ui, glyph, action == Clicked::Close, hit);
        // 按住期間收起提示，與舊版按下即收一致
        let button = if button.is_pointer_button_down_on() {
            button
        } else {
            let mut tip = flat_tooltip(&button);
            tip.popup = tip
                .popup
                .at_position(tooltip_anchor(ui.ctx(), button.id))
                .gap(TOOLTIP_OFFSET);
            tip.show(|ui| {
                let text = t(tooltip, &[("hotkey", region_hotkey)]);
                ui.label(
                    RichText::new(text)
                        .font(FontId::proportional(tk_pt(9.0)))
                        .color(FG_TRANSLATED),
                );
            });
            button
        };
        if button.clicked() {
            clicked = Some(action);
        }
    }
    ui.add_space(6.0);
    if let Some(state) = status {
        let text = RichText::new(status_text(state))
            .font(small)
            .color(status_color(state));
        ui.add(Label::new(text).selectable(false));
        ui.add_space(6.0);
    }
    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(6.0);
        ui.add(Image::new((icon, Vec2::splat(BAR_ICON))));
        ui.add_space(4.0);
        let text = RichText::new(title)
            .font(FontId::proportional(tk_pt(9.0)))
            .color(FG_BAR);
        let room = ui.available_width();
        let name = WidgetText::from(text).into_galley(
            ui,
            Some(TextWrapMode::Truncate),
            room,
            TextStyle::Body,
        );
        // 窄到連「…」都放不下時不畫名稱，免得壓到狀態字
        if name.size().x <= room {
            ui.add(Label::new(name).selectable(false));
        }
    });
    clicked
}

/// 提示框與游標的距離（右下方），避免蓋住游標。
const TOOLTIP_OFFSET: f32 = 12.0;

/// 不帶陰影的提示框（egui 預設樣式的彈出框有往右下偏的陰影，舊版沒有）。
pub(crate) fn flat_tooltip(response: &Response) -> Tooltip<'static> {
    let mut tip = Tooltip::for_enabled(response);
    let frame = egui::Frame::popup(&response.ctx.global_style()).shadow(egui::Shadow::NONE);
    tip.popup = tip.popup.frame(frame);
    tip
}

/// 提示框的錨點：出現那一刻的游標位置，之後不跟著游標走（同舊版）。
fn tooltip_anchor(ctx: &egui::Context, widget: egui::Id) -> Pos2 {
    let key = widget.with("tooltip_anchor");
    let pointer = ctx.pointer_hover_pos().unwrap_or_default();
    if Tooltip::was_tooltip_open_last_frame(ctx, widget) {
        ctx.data(|d| d.get_temp(key)).unwrap_or(pointer)
    } else {
        ctx.data_mut(|d| d.insert_temp(key, pointer));
        pointer
    }
}

/// 標題列按鈕：與標題列同高的正方形，滑過時填底、按下再亮一階；`close` 用紅底白字。
///
/// 只有落在 `hit` 內的部分接滑鼠；也接拖曳，否則按鈕上的按壓會被底下的拖曳區當成移動視窗。
pub(crate) fn bar_button(ui: &mut Ui, glyph: &str, close: bool, hit: egui::Rect) -> Response {
    let (square, node) = ui.allocate_exact_size(Vec2::splat(BAR_HEIGHT), Sense::hover());
    node.widget_info(|| WidgetInfo::labeled(Role::Button, true, glyph));
    let button = ui
        .interact(
            square.intersect(hit),
            node.id.with("hit"),
            Sense::click_and_drag(),
        )
        .on_hover_cursor(CursorIcon::PointingHand);
    let (hover_fill, hover_fg) = if close {
        (CLOSE_HOVER, Color32::WHITE)
    } else {
        (GRIP, TAB_FG_ACTIVE)
    };
    let fg = if button.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(square, 0.0, hover_fill.lerp_to_gamma(Color32::WHITE, 0.12));
        hover_fg
    } else if button.hovered() {
        ui.painter().rect_filled(square, 0.0, hover_fill);
        hover_fg
    } else {
        FG_BAR
    };
    ui.painter().text(
        square.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(tk_pt(10.0)),
        fg,
    );
    button
}

/// 右下角抓取紋：三道斜線，亮色配描邊，任何背景上都看得到。
fn paint_grip(painter: &egui::Painter, grip: egui::Rect) {
    for inset in [4.0, 9.0, 14.0] {
        let a = grip.min + Vec2::new(GRIP_SIZE - inset, GRIP_SIZE - 2.0);
        let b = grip.min + Vec2::new(GRIP_SIZE - 2.0, GRIP_SIZE - inset);
        painter.line_segment([a, b], Stroke::new(4.0, OUTLINE));
        painter.line_segment([a, b], Stroke::new(2.0, FG_BAR));
    }
}

fn resize_cursor(edge: Edge) -> CursorIcon {
    match edge {
        Edge::N | Edge::S => CursorIcon::ResizeVertical,
        Edge::W | Edge::E => CursorIcon::ResizeHorizontal,
        Edge::Nw | Edge::Se => CursorIcon::ResizeNwSe,
        Edge::Ne | Edge::Sw => CursorIcon::ResizeNeSw,
    }
}

/// Tk 的 `WxH+X+Y` 寫法，與 Python log 相同。
pub(crate) fn describe((x, y, w, h): Rect) -> String {
    format!("{w}x{h}+{x}+{y}")
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Instant;

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::ui::fonts_egui;
    use crate::ui::overlay::banners::{Banners, Release};
    use crate::ui::overlay::message_list::{self, MessageList};
    use crate::ui::overlay::model::OverlayModel;
    use crate::ui::overlay::outlined;

    const START: Rect = (1000, 500, 640, 420);

    pub(crate) struct FakeShell {
        pub(crate) cursor: (i32, i32),
        pub(crate) rect: Rect,
        /// `false` 模擬視窗還沒建好（HWND 為 0）。
        pub(crate) ready: bool,
        /// `take_focus` 被呼叫的次數。
        pub(crate) focused: usize,
    }

    impl Shell for FakeShell {
        fn cursor(&self) -> (i32, i32) {
            self.cursor
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn set_rect(&mut self, rect: Rect) -> bool {
            if self.ready {
                self.rect = rect;
            }
            self.ready
        }
        fn monitors(&self) -> Vec<Rect> {
            vec![(0, 0, 1920, 1080)]
        }
        fn scale_at(&self, _x: i32, _y: i32) -> f32 {
            1.0
        }
        fn take_focus(&mut self) -> bool {
            self.focused += 1;
            true
        }
    }

    struct State {
        frame: Frame,
        shell: FakeShell,
        title: String,
        area: egui::Rect,
        clicked: Option<Clicked>,
        /// 疊在外框上的橫幅與訊息清單（驗證內容不擋縮放帶）。
        content: Option<(Banners, MessageList, OverlayModel)>,
    }

    fn setup(
        title: &str,
        size: Vec2,
        ppp: f32,
    ) -> (Harness<'static, State>, Rc<RefCell<Vec<Rect>>>) {
        let saved = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&saved);
        let state = State {
            frame: Frame::new(0.8, None, move |x, y, w, h| {
                sink.borrow_mut().push((x, y, w, h))
            }),
            shell: FakeShell {
                cursor: (0, 0),
                rect: START,
                ready: true,
                focused: 0,
            },
            title: title.into(),
            area: egui::Rect::NOTHING,
            clicked: None,
            content: None,
        };
        let mut harness = Harness::builder()
            .with_size(size)
            .with_pixels_per_point(ppp)
            .with_context_setup(|ctx| fonts_egui::install(ctx, "en-US"))
            .build_ui_state(
                |ui, s: &mut State| {
                    s.area = ui.max_rect();
                    let clicked = s.frame.show(
                        ui,
                        &mut s.shell,
                        &s.title,
                        Some("listening"),
                        "ctrl+shift+space",
                    );
                    s.clicked = s.clicked.or(clicked);
                    if let Some((banners, list, model)) = &mut s.content {
                        let mut rect = s.area;
                        rect.min.y += BAR_HEIGHT;
                        let rect = banners.show(ui, rect);
                        list.show(ui, rect, model, None, None);
                    }
                },
                state,
            );
        harness.step();
        (harness, saved)
    }

    /// 從視窗內 `from` 拖到 `to`（皆為相對外框左上角的 DIP）；假游標跟著換算成螢幕實體像素。
    fn drag(harness: &mut Harness<State>, from: Vec2, to: Vec2, ppp: f32) {
        let origin = harness.state().area.min;
        let screen = |v: Vec2| (START.0 + (v.x * ppp) as i32, START.1 + (v.y * ppp) as i32);
        harness.state_mut().shell.cursor = screen(from);
        harness.hover_at(origin + from);
        harness.drag_at(origin + from);
        harness.step();
        for k in 1..=3 {
            let v = from + (to - from) * (k as f32 / 3.0);
            harness.state_mut().shell.cursor = screen(v);
            harness.hover_at(origin + v);
            harness.step();
        }
        harness.drop_at(origin + to);
        harness.step();
    }

    /// `square` 這幀被填的底色（沒填為 `None`）與 `glyph` 的字色。
    fn painted(
        harness: &Harness<State>,
        square: egui::Rect,
        glyph: &str,
    ) -> (Option<Color32>, Color32) {
        let shapes = &harness.output().shapes;
        let fill = shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.rect == square => Some(r.fill),
            _ => None,
        });
        let fg = shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == glyph => Some(t.fallback_color),
                _ => None,
            })
            .unwrap();
        (fill, fg)
    }

    #[test]
    fn title_bar_items_are_vertically_centred_and_buttons_are_bar_tall_squares() {
        let (harness, _) = setup("Title", Vec2::new(640.0, 420.0), 1.0);
        let area = harness.state().area;
        let mid = area.min.y + BAR_HEIGHT / 2.0;
        let mut right = area.max.x;
        for glyph in ["✕", "─", "⚙", "⛶"] {
            let r = harness.get_by_label(glyph).rect();
            assert_eq!(r.size(), Vec2::splat(BAR_HEIGHT), "{glyph}");
            assert_eq!((r.min.y, r.max.x), (area.min.y, right), "{glyph}");
            right = r.min.x;
        }
        let status = t("status.listening", &[]);
        for node in [
            harness.get_by_role(egui::accesskit::Role::Image),
            harness.get_by(|n| n.value().as_deref() == Some("Title")),
            harness.get_by(|n| n.value().as_deref() == Some(status.as_str())),
        ] {
            let r = node.rect();
            assert!(
                (r.center().y - mid).abs() <= 0.5,
                "{r:?} not centred on {mid}"
            );
        }
    }

    #[test]
    fn hovering_or_pressing_a_title_button_fills_it() {
        for (glyph, fill, fg) in [
            ("⚙", GRIP, TAB_FG_ACTIVE),
            ("─", GRIP, TAB_FG_ACTIVE),
            ("✕", CLOSE_HOVER, Color32::WHITE),
        ] {
            let (mut harness, _) = setup("t", Vec2::new(640.0, 420.0), 1.0);
            let square = harness.get_by_label(glyph).rect();
            assert_eq!(
                painted(&harness, square, glyph),
                (None, FG_BAR),
                "{glyph} idle"
            );
            harness.hover_at(square.center());
            harness.run_steps(2);
            assert_eq!(
                painted(&harness, square, glyph),
                (Some(fill), fg),
                "{glyph} hover"
            );
            harness.drag_at(square.center());
            harness.run_steps(2);
            let pressed = fill.lerp_to_gamma(Color32::WHITE, 0.12);
            assert_eq!(
                painted(&harness, square, glyph),
                (Some(pressed), fg),
                "{glyph} pressed"
            );
        }
    }

    #[test]
    fn window_edge_bands_over_a_title_button_still_resize() {
        let (mut harness, saved) = setup("t", Vec2::new(640.0, 420.0), 1.0);
        let gear = harness.get_by_label("⚙").rect().center() - harness.state().area.min;
        drag(
            &mut harness,
            Vec2::new(gear.x, 2.0),
            Vec2::new(gear.x, -28.0),
            1.0,
        );
        assert_eq!(*saved.borrow(), [(1000, 470, 640, 450)]);

        let (mut harness, saved) = setup("t", Vec2::new(640.0, 420.0), 1.0);
        let right = harness.state().area.width() - 2.0;
        drag(
            &mut harness,
            Vec2::new(right, 2.0),
            Vec2::new(right + 20.0, -18.0),
            1.0,
        );
        assert_eq!(*saved.borrow(), [(1000, 480, 660, 440)]);
    }

    #[test]
    fn title_buttons_never_clipped_by_long_name() {
        let title = "Wizard101 Chat Translator ".repeat(20);
        // 最小寬度下名稱整個讓出去，寬一點時被截短
        for (width, name_shown) in [(MIN_WIDTH as f32, false), (320.0, true)] {
            let (harness, _) = setup(&title, Vec2::new(width + 16.0, 120.0), 1.0);
            let area = harness.state().area;
            let status = harness.get_by_label(&t("status.listening", &[])).rect();
            for glyph in ["✕", "─", "⚙", "⛶"] {
                let r = harness.get_by_label(glyph).rect();
                assert!(
                    r.min.x >= area.min.x && r.max.x <= area.max.x,
                    "{glyph} {r:?} {area:?}"
                );
                assert!(r.min.x >= status.max.x, "{glyph} overlaps status");
            }
            let name = harness.query_by(|n| n.value().is_some_and(|v| v.starts_with("Wizard101")));
            assert_eq!(name.is_some(), name_shown, "width={width}");
            if let Some(name) = name {
                let name = name.rect();
                assert!(
                    name.max.x <= status.min.x,
                    "name {name:?} runs under status {status:?}"
                );
            }
        }
    }

    #[test]
    fn region_tooltip_shows_the_hotkey_after_a_pause() {
        let (mut harness, _) = setup("t", Vec2::new(640.0, 420.0), 1.0);
        let text = t("tooltip.region", &[("hotkey", "ctrl+shift+space")]);
        harness.get_by_label("⛶").hover();
        harness.step();
        assert!(
            harness.query_by_label(&text).is_none(),
            "shown before the delay"
        );
        for _ in 0..40 {
            harness.step();
        }
        harness.get_by_label(&text);
    }

    /// 懸停 ⛶ 滿延遲後提示框已出現。
    fn hovered_tooltip() -> (Harness<'static, State>, String) {
        let (mut harness, _) = setup("t", Vec2::new(640.0, 420.0), 1.0);
        let text = t("tooltip.region", &[("hotkey", "ctrl+shift+space")]);
        harness.get_by_label("⛶").hover();
        for _ in 0..40 {
            harness.step();
        }
        harness.get_by_label(&text);
        (harness, text)
    }

    fn move_away(harness: &mut Harness<State>) {
        let away = harness.state().area.center();
        harness.hover_at(away);
        for _ in 0..4 {
            harness.step();
        }
    }

    #[test]
    fn tooltip_text_is_bright() {
        let (harness, text) = hovered_tooltip();
        let color = harness.output().shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(s) if s.galley.text() == text => {
                Some(s.galley.job.sections[0].format.color)
            }
            _ => None,
        });
        assert_eq!(color, Some(FG_TRANSLATED));
    }

    /// 這一幀有沒有畫出帶模糊的陰影。
    pub(crate) fn has_shadow(output: &egui::FullOutput) -> bool {
        fn blurred(shape: &egui::Shape) -> bool {
            match shape {
                egui::Shape::Rect(r) => r.blur_width > 0.0,
                egui::Shape::Vec(shapes) => shapes.iter().any(blurred),
                _ => false,
            }
        }
        output.shapes.iter().any(|c| blurred(&c.shape))
    }

    #[test]
    fn the_tooltip_has_no_shadow() {
        let (harness, _) = hovered_tooltip();
        assert!(!has_shadow(harness.output()));
    }

    #[test]
    fn the_tooltip_stays_put_while_the_pointer_moves() {
        let (mut harness, text) = hovered_tooltip();
        let before = harness.get_by_label(&text).rect();
        let button = harness.get_by_label("⛶").rect();
        harness.hover_at(button.center() + Vec2::new(4.0, 3.0));
        harness.run_steps(2);
        assert_eq!(harness.get_by_label(&text).rect(), before);
    }

    #[test]
    fn leaving_hides_the_tooltip() {
        let (mut harness, text) = hovered_tooltip();
        move_away(&mut harness);
        assert!(harness.query_by_label(&text).is_none());
    }

    #[test]
    fn leaving_before_the_delay_elapses_cancels_the_show() {
        let (mut harness, _) = setup("t", Vec2::new(640.0, 420.0), 1.0);
        let text = t("tooltip.region", &[("hotkey", "ctrl+shift+space")]);
        harness.get_by_label("⛶").hover();
        harness.step();
        move_away(&mut harness);
        for _ in 0..40 {
            harness.step();
        }
        assert!(harness.query_by_label(&text).is_none());
    }

    #[test]
    fn button_press_hides_the_tooltip() {
        let (mut harness, text) = hovered_tooltip();
        let button = harness.get_by_label("⛶").rect().center();
        harness.drag_at(button);
        harness.run_steps(2);
        assert!(harness.query_by_label(&text).is_none());
    }

    fn banners_of<'a>(harness: &'a mut Harness<State>) -> &'a mut Banners {
        &mut harness.state_mut().content.as_mut().unwrap().0
    }

    #[test]
    fn error_banner_keeps_following_new_messages() {
        let (mut harness, _) = setup("t", Vec2::new(640.0, 420.0), 1.0);
        let mut model = OverlayModel::new(100, 0);
        for _ in 0..40 {
            let now = Instant::now();
            model.add_message(1, &[0], "original", "譯文", None, false, None, now);
        }
        harness.state_mut().content = Some((Banners::default(), MessageList::new(), model));
        harness.step();
        let at_bottom = |harness: &Harness<State>| {
            let (_, list, model) = harness.state().content.as_ref().unwrap();
            let last = list.rows().last().unwrap();
            let (origin, galley) = &last.lines[1];
            let bottom = origin.y + outlined::line_height(galley) + message_list::ROW_PAD;
            last.seq == model.current_view().unwrap().messages.last().unwrap().seq
                && (bottom - list.viewport().max.y).abs() < 0.01
        };
        assert!(at_bottom(&harness));

        banners_of(&mut harness).set_error("notice.offline", Vec::new());
        harness.step();
        assert!(at_bottom(&harness));
        banners_of(&mut harness).clear_error();
        harness.step();
        assert!(at_bottom(&harness));
    }

    #[test]
    fn release_after_drag_reports_geometry_once() {
        let (mut harness, saved) = setup("t", Vec2::new(640.0, 420.0), 1.0);
        drag(
            &mut harness,
            Vec2::new(300.0, 10.0),
            Vec2::new(350.0, 40.0),
            1.0,
        );
        assert_eq!(harness.state().shell.rect, (1050, 530, 640, 420));
        assert_eq!(*saved.borrow(), [(1050, 530, 640, 420)]);
    }

    #[test]
    fn resize_respects_min_size() {
        for (ppp, min) in [(1.0, (200, 90)), (1.5, (300, 135))] {
            let (mut harness, saved) = setup("t", Vec2::new(640.0, 420.0), ppp);
            let corner = harness.state().area.size() - Vec2::splat(3.0);
            drag(&mut harness, corner, Vec2::new(-2000.0, -2000.0), ppp);
            assert_eq!(
                harness.state().shell.rect,
                (1000, 500, min.0, min.1),
                "ppp={ppp}"
            );
            assert_eq!(*saved.borrow(), [(1000, 500, min.0, min.1)], "ppp={ppp}");
        }
    }

    #[test]
    fn left_edge_resizes_and_keeps_right_edge() {
        let (mut harness, saved) = setup("t", Vec2::new(640.0, 420.0), 1.0);
        drag(
            &mut harness,
            Vec2::new(2.0, 200.0),
            Vec2::new(-38.0, 230.0),
            1.0,
        );
        assert_eq!(*saved.borrow(), [(960, 500, 680, 420)]);
    }

    #[test]
    fn top_edge_band_resizes_not_drags() {
        let (mut harness, saved) = setup("t", Vec2::new(640.0, 420.0), 1.0);
        drag(
            &mut harness,
            Vec2::new(300.0, 2.0),
            Vec2::new(300.0, -28.0),
            1.0,
        );
        assert_eq!(*saved.borrow(), [(1000, 470, 640, 450)]);

        // 同一條標題列、邊帶以下：移動
        let (mut harness, saved) = setup("t", Vec2::new(640.0, 420.0), 1.0);
        drag(
            &mut harness,
            Vec2::new(300.0, 12.0),
            Vec2::new(300.0, -18.0),
            1.0,
        );
        assert_eq!(*saved.borrow(), [(1000, 470, 640, 420)]);
    }

    #[test]
    fn press_in_the_body_away_from_edges_does_nothing() {
        let (mut harness, saved) = setup("t", Vec2::new(640.0, 420.0), 1.0);
        drag(
            &mut harness,
            Vec2::new(300.0, 200.0),
            Vec2::new(350.0, 250.0),
            1.0,
        );
        assert_eq!(harness.state().shell.rect, START);
        assert!(saved.borrow().is_empty());
    }

    #[test]
    fn each_title_button_maps_to_its_action_without_moving() {
        for (glyph, action) in [
            ("✕", Clicked::Close),
            ("─", Clicked::Minimize),
            ("⚙", Clicked::Settings),
            ("⛶", Clicked::Region),
        ] {
            let (mut harness, saved) = setup("t", Vec2::new(640.0, 420.0), 1.0);
            harness.get_by_label(glyph).click();
            harness.step();
            assert_eq!(harness.state().clicked, Some(action), "{glyph}");
            assert_eq!(harness.state().shell.rect, START, "{glyph}");
            assert!(saved.borrow().is_empty(), "{glyph}");
        }
    }

    #[test]
    fn dragging_from_a_title_button_neither_moves_nor_clicks() {
        for glyph in ["✕", "─", "⚙", "⛶"] {
            let (mut harness, saved) = setup("t", Vec2::new(640.0, 420.0), 1.0);
            let from = harness.get_by_label(glyph).rect().center() - harness.state().area.min;
            drag(&mut harness, from, from + Vec2::new(-50.0, 30.0), 1.0);
            assert_eq!(harness.state().shell.rect, START, "{glyph}");
            assert!(saved.borrow().is_empty(), "{glyph}");
            assert_eq!(harness.state().clicked, None, "{glyph}");
        }
    }

    #[test]
    fn dragging_the_icon_name_or_status_moves_the_window() {
        let status = t("status.listening", &[]);
        for target in ["icon", "name", "status"] {
            let (mut harness, saved) = setup("Title", Vec2::new(640.0, 420.0), 1.0);
            let node = match target {
                "icon" => harness.get_by_role(egui::accesskit::Role::Image),
                "name" => harness.get_by(|n| n.value().as_deref() == Some("Title")),
                _ => harness.get_by(|n| n.value().as_deref() == Some(status.as_str())),
            };
            let from = node.rect().center() - harness.state().area.min;
            drag(&mut harness, from, from + Vec2::new(50.0, 30.0), 1.0);
            assert_eq!(*saved.borrow(), [(1050, 530, 640, 420)], "{target}");
        }
    }

    #[test]
    fn message_list_and_banners_keep_the_resize_bands_and_grip_reachable() {
        let size = Vec2::new(640.0, 420.0);
        let with_content = |banners: bool| {
            let (mut harness, saved) = setup("t", size, 1.0);
            let mut model = OverlayModel::new(100, 0);
            for _ in 0..100 {
                model.add_message(
                    1,
                    &[0],
                    "original",
                    "譯文",
                    None,
                    false,
                    None,
                    Instant::now(),
                );
            }
            let mut shown = Banners::default();
            if banners {
                shown.set_error(
                    "notice.offline_detail",
                    vec![("message", "see https://a.example/keys".into())],
                );
                shown.set_update(Release {
                    version: "0.2.0".into(),
                    url: "https://example.invalid/rel".into(),
                });
            }
            harness.state_mut().content = Some((shown, MessageList::new(), model));
            harness.step();
            (harness, saved)
        };
        let area = with_content(false).0.state().area.size();
        // 右緣帶（捲軸右側）、下緣帶（橫幅底下）、左緣帶（錯誤橫幅左端）與右下角抓取紋
        for banners in [false, true] {
            for (from, expected) in [
                (Vec2::new(area.x - 2.0, 200.0), (1000, 500, 660, 420)),
                (Vec2::new(300.0, area.y - 2.0), (1000, 500, 640, 440)),
                (area - Vec2::splat(3.0), (1000, 500, 660, 440)),
            ] {
                let (mut harness, saved) = with_content(banners);
                assert!(harness.state().content.as_ref().unwrap().1.rows().len() > 1);
                drag(&mut harness, from, from + Vec2::new(20.0, 20.0), 1.0);
                assert_eq!(*saved.borrow(), [expected], "{from:?} banners={banners}");
            }
        }
        let (mut harness, saved) = with_content(true);
        let link = harness.get_by_label("https://a.example/keys").rect();
        let from = Vec2::new(2.0, link.center().y);
        drag(&mut harness, from, from + Vec2::new(-20.0, 0.0), 1.0);
        assert_eq!(*saved.borrow(), [(980, 500, 660, 420)]);
    }

    fn run_frame(frame: &mut Frame, shell: &mut FakeShell) {
        let ctx = egui::Context::default();
        fonts_egui::install(&ctx, "en-US");
        let mut output = ctx.run_ui(Default::default(), |ui| {
            frame.show(ui, shell, "t", None, "");
        });
        output.textures_delta.clear();
    }

    #[test]
    fn initial_placement_waits_for_the_window_then_applies_once() {
        let saved = Geometry {
            x: Some(10),
            y: Some(20),
            width: 400,
            height: 250,
        };
        let mut frame = Frame::new(0.8, Some(saved), |_, _, _, _| {});
        let mut shell = FakeShell {
            cursor: (0, 0),
            rect: START,
            ready: false,
            focused: 0,
        };
        run_frame(&mut frame, &mut shell);
        assert_eq!(shell.rect, START);

        shell.ready = true;
        run_frame(&mut frame, &mut shell);
        assert_eq!(shell.rect, (10, 20, 400, 250));

        shell.rect = START;
        run_frame(&mut frame, &mut shell);
        assert_eq!(shell.rect, START, "placement must happen only once");
    }

    #[test]
    fn first_launch_is_centered_and_saved_geometry_is_kept() {
        let screen = [(0, 0, 1920, 1080)];
        let unset = Geometry {
            x: None,
            y: None,
            width: 640,
            height: 420,
        };
        assert_eq!(initial_rect(&unset, &screen, 1.0), (640, 330, 640, 420));
        let saved = Geometry {
            x: Some(5),
            y: Some(6),
            width: 100,
            height: 50,
        };
        assert_eq!(
            initial_rect(&saved, &screen, 1.0),
            (5, 6, MIN_WIDTH, MIN_HEIGHT)
        );
        // 最小尺寸以 DIP 計：150% 螢幕上 250×100 實體像素只有 167×67 DIP，要撐到 200×90 DIP
        let small = Geometry {
            x: Some(5),
            y: Some(6),
            width: 250,
            height: 100,
        };
        assert_eq!(initial_rect(&small, &screen, 1.5), (5, 6, 300, 135));
        let lost = Geometry {
            x: Some(2500),
            y: Some(300),
            width: 640,
            height: 420,
        };
        assert_eq!(initial_rect(&lost, &screen, 1.0), (1280, 300, 640, 420));
        // 主螢幕是含原點的那顆，不是清單第一顆
        let two = [(-1920, 0, 1920, 1200), (0, 0, 1920, 1080)];
        assert_eq!(initial_rect(&unset, &two, 1.0), (640, 330, 640, 420));
    }

    #[test]
    fn set_alpha_applies_immediately() {
        let mut frame = Frame::new(0.8, None, |_, _, _, _| {});
        frame.set_alpha(0.5);
        assert_eq!(frame.alpha(), 0.5);
    }

    #[test]
    fn every_status_has_a_colour() {
        // 狀態列出現的時機（掛不進遊戲）正是最不該少一個顏色的時候
        let catalog: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(include_str!("../../i18n/zh-TW.json")).unwrap();
        let mut labelled: Vec<_> = catalog
            .keys()
            .filter_map(|k| k.strip_prefix("status."))
            .collect();
        let mut coloured: Vec<_> = STATUS_COLORS.iter().map(|(k, _)| *k).collect();
        labelled.sort();
        coloured.sort();
        assert_eq!(labelled, coloured);
    }
}
