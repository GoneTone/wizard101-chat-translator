//! overlay 視窗外框：半透明底、標題列（圖示、名稱、狀態、⛶⚙─✕）、拖曳移動、四邊四角縮放與記憶位置。

use eframe::egui::{
    self, Align, Color32, CursorIcon, FontId, Image, Label, Layout, Pos2, RichText, Sense, Stroke,
    TextureHandle, TextureOptions, Ui, UiBuilder, Vec2,
};

use crate::config::Geometry;
use crate::i18n::t;
use crate::logs::log;
use crate::runtime::resources::icon_png;
use crate::ui::fonts_egui::tk_pt;
use crate::ui::geometry::{
    Edge, Rect, centered_position, edge_at, moved_to, pull_onscreen, resized_edge,
};
use crate::ui::palette::{BAR, BG, FG_BAR, FG_ERROR, OUTLINE};
use crate::ui::win32;

/// 視窗最小寬度（DIP）。
pub const MIN_WIDTH: i32 = 200;
/// 視窗最小高度（DIP）。
pub const MIN_HEIGHT: i32 = 90;

/// 狀態字的顏色（文字由 i18n 依 state key 取得）。
pub const STATUS_COLORS: [(&str, Color32); 6] = [
    ("locating", Color32::from_rgb(0xe0, 0xb0, 0x50)),
    ("listening", Color32::from_rgb(0x7d, 0xc8, 0x7d)),
    ("translating", Color32::from_rgb(0x6f, 0xa8, 0xdc)),
    ("waiting_game", Color32::from_rgb(0x9a, 0x9a, 0xa8)),
    // 要使用者動手（以管理員重開、更新）才解得掉，用錯誤色
    ("access_denied", FG_ERROR),
    ("version_mismatch", FG_ERROR),
];

const BAR_HEIGHT: f32 = 20.0;
const BAR_ICON: f32 = 16.0;
const GRIP_SIZE: f32 = 16.0;

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
    /// 移動／縮放視窗。
    fn set_rect(&mut self, rect: Rect);
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
    fn set_rect(&mut self, rect: Rect) {
        win32::set_window_rect(self.0, rect);
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

/// 啟動時的視窗矩形：沒存過位置就擺主螢幕正中央，存過但整個在畫面外就拉回來。
///
/// `screen` 是主螢幕尺寸，`monitors` 是所有螢幕矩形，皆為實體像素。
pub fn initial_rect(saved: &Geometry, screen: (i32, i32), monitors: &[Rect]) -> Rect {
    let (w, h) = (saved.width.max(MIN_WIDTH), saved.height.max(MIN_HEIGHT));
    let (cx, cy) = centered_position(screen.0, screen.1, w, h);
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
    initial: Option<Rect>,
    gesture: Option<Gesture>,
    icon: Option<TextureHandle>,
    on_geometry_change: Box<dyn FnMut(i32, i32, i32, i32)>,
}

impl Frame {
    /// `initial` 在第一次 `show` 時套到視窗（`None` 表示不動）；拖曳或縮放放開時呼叫 `on_geometry_change`。
    pub fn new(
        alpha: f32,
        initial: Option<Rect>,
        on_geometry_change: impl FnMut(i32, i32, i32, i32) + 'static,
    ) -> Self {
        Self {
            alpha,
            initial,
            gesture: None,
            icon: None,
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

    /// 畫出外框並處理拖曳與縮放；回傳這幀被按下的標題列按鈕。
    ///
    /// 要在內容之前呼叫：之後加入的元件疊在外框的拖曳區之上、優先接到滑鼠。
    pub fn show(
        &mut self,
        ui: &mut Ui,
        shell: &mut dyn Shell,
        title: &str,
        status: Option<&str>,
    ) -> Option<Clicked> {
        if let Some(rect) = self.initial.take() {
            shell.set_rect(rect);
        }
        let area = ui.max_rect();
        let bar = egui::Rect::from_min_size(area.min, Vec2::new(area.width(), BAR_HEIGHT));
        let grip =
            egui::Rect::from_min_size(area.max - Vec2::splat(GRIP_SIZE), Vec2::splat(GRIP_SIZE));
        let painter = ui.painter();
        painter.rect_filled(area, 0.0, BG.gamma_multiply(self.alpha));
        painter.rect_filled(bar, 0.0, BAR);
        paint_grip(painter, grip);

        let response = ui.interact(area, ui.scope_id().with("overlay_frame"), Sense::drag());
        let edge_under = |p: Pos2| {
            if grip.contains(p) {
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
        };
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
                        let ppp = ui.ctx().pixels_per_point();
                        let min = |dip: i32| (dip as f32 * ppp).round() as i32;
                        resized_edge(edge, x, y, w, h, dx, dy, min(MIN_WIDTH), min(MIN_HEIGHT))
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

        let icon = self.icon.get_or_insert_with(|| load_icon(ui.ctx())).clone();
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(bar)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| title_bar(ui, &icon, title, status),
        )
        .inner
    }
}

/// 標題列內容；按鈕與狀態先由右往左排，名稱只拿剩下的寬度，太長時被截的是名稱。
fn title_bar(
    ui: &mut Ui,
    icon: &TextureHandle,
    title: &str,
    status: Option<&str>,
) -> Option<Clicked> {
    ui.spacing_mut().item_spacing.x = 0.0;
    let small = FontId::proportional(tk_pt(8.0));
    let mut clicked = None;
    for (glyph, gap, action) in [
        ("✕", 6.0, Clicked::Close),
        ("─", 4.0, Clicked::Minimize),
        ("⚙", 4.0, Clicked::Settings),
        ("⛶", 4.0, Clicked::Region),
    ] {
        ui.add_space(gap);
        let text = RichText::new(glyph)
            .font(FontId::proportional(tk_pt(9.0)))
            .color(FG_BAR);
        // 也接拖曳：否則按鈕上的按壓會被底下的拖曳區當成移動視窗
        let button = ui.add(
            Label::new(text)
                .selectable(false)
                .sense(Sense::click_and_drag()),
        );
        if button.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            clicked = Some(action);
        }
    }
    ui.add_space(6.0);
    if let Some(state) = status {
        let text = RichText::new(t(&format!("status.{state}"), &[]))
            .font(small.clone())
            .color(status_color(state));
        ui.add(Label::new(text).selectable(false));
        ui.add_space(6.0);
    }
    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(6.0);
        ui.add(Image::new((icon.id(), Vec2::splat(BAR_ICON))));
        ui.add_space(4.0);
        let text = RichText::new(title).font(small).color(FG_BAR);
        ui.add(Label::new(text).selectable(false).truncate());
    });
    clicked
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

fn load_icon(ctx: &egui::Context) -> TextureHandle {
    let data =
        eframe::icon_data::from_png_bytes(icon_png(16)).expect("embedded icon is a valid PNG");
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [data.width as usize, data.height as usize],
        &data.rgba,
    );
    ctx.load_texture("overlay_bar_icon", image, TextureOptions::LINEAR)
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
fn describe((x, y, w, h): Rect) -> String {
    format!("{w}x{h}+{x}+{y}")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::ui::fonts_egui;

    const START: Rect = (1000, 500, 640, 420);

    struct FakeShell {
        cursor: (i32, i32),
        rect: Rect,
    }

    impl Shell for FakeShell {
        fn cursor(&self) -> (i32, i32) {
            self.cursor
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn set_rect(&mut self, rect: Rect) {
            self.rect = rect;
        }
    }

    struct State {
        frame: Frame,
        shell: FakeShell,
        title: String,
        area: egui::Rect,
        clicked: Option<Clicked>,
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
            },
            title: title.into(),
            area: egui::Rect::NOTHING,
            clicked: None,
        };
        let mut harness = Harness::builder()
            .with_size(size)
            .with_pixels_per_point(ppp)
            .with_context_setup(|ctx| fonts_egui::install(ctx, "en-US"))
            .build_ui_state(
                |ui, s: &mut State| {
                    s.area = ui.max_rect();
                    let clicked = s.frame.show(ui, &mut s.shell, &s.title, Some("listening"));
                    s.clicked = s.clicked.or(clicked);
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

    #[test]
    fn title_buttons_never_clipped_by_long_name() {
        let title = "Wizard101 Chat Translator ".repeat(20);
        let (harness, _) = setup(&title, Vec2::new(MIN_WIDTH as f32 + 16.0, 120.0), 1.0);
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
        let name = harness
            .get_by(|n| n.value().is_some_and(|v| v.starts_with("Wizard101")))
            .rect();
        assert!(
            name.max.x <= status.min.x,
            "name {name:?} runs under status {status:?}"
        );
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
    fn close_button_click_closes_without_moving() {
        let (mut harness, saved) = setup("t", Vec2::new(640.0, 420.0), 1.0);
        harness.get_by_label("✕").click();
        harness.step();
        assert_eq!(harness.state().clicked, Some(Clicked::Close));
        assert!(saved.borrow().is_empty());
    }

    #[test]
    fn initial_rect_is_applied_on_first_show() {
        let mut frame = Frame::new(0.8, Some((10, 20, 400, 250)), |_, _, _, _| {});
        let mut shell = FakeShell {
            cursor: (0, 0),
            rect: START,
        };
        let ctx = egui::Context::default();
        fonts_egui::install(&ctx, "en-US");
        let mut output = ctx.run_ui(Default::default(), |ui| {
            frame.show(ui, &mut shell, "t", None);
        });
        output.textures_delta.clear();
        assert_eq!(shell.rect, (10, 20, 400, 250));
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
        assert_eq!(
            initial_rect(&unset, (1920, 1080), &screen),
            (640, 330, 640, 420)
        );
        let saved = Geometry {
            x: Some(5),
            y: Some(6),
            width: 100,
            height: 50,
        };
        assert_eq!(
            initial_rect(&saved, (1920, 1080), &screen),
            (5, 6, MIN_WIDTH, MIN_HEIGHT)
        );
        let lost = Geometry {
            x: Some(2500),
            y: Some(300),
            width: 640,
            height: 420,
        };
        assert_eq!(
            initial_rect(&lost, (1920, 1080), &screen),
            (1280, 300, 640, 420)
        );
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
            serde_json::from_str(include_str!("../../../../src/i18n/zh-TW.json")).unwrap();
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
