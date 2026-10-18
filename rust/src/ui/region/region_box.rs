//! 框選框：框選放開後留在畫面上的矩形，拖邊線改大小、拖框內移動，放開時回報新矩形。
//!
//! 視窗比矩形四邊各大 `MARGIN`（把手露在框外），只用一個 viewport：畫面只有框線與把手，
//! 其餘透明但整個視窗都收得到滑鼠。位移一律用螢幕座標算，視窗在拖曳中自己會動。

use eframe::egui::{self, CursorIcon, Stroke, StrokeKind, vec2};

use crate::logs::log;
use crate::ui::geometry::{
    Edge, Rect, edge_at_bands, is_click, moved_to, point_in_rect, resized_edge,
};
use crate::ui::palette::FG_UPDATE;

/// 矩形外側多留的寬度（px）：把手露在框外，也是邊帶在框外的那一半。
pub const MARGIN: i32 = 6;
/// 把手方塊邊長（px）。
pub const HANDLE: i32 = 10;
/// 縮放時的最小寬高（px）。
pub const MIN_SIZE: i32 = 20;
/// 邊帶在矩形內側的寬度（px）；與 `MARGIN` 合起來是整條邊帶。
pub const INNER_BAND: i32 = 4;
/// 亮色框線寬度（DIP），以矩形邊為中心線畫。
pub const LINE: f32 = 3.0;

/// 游標壓住框的哪裡。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// 框內：移動整個框。
    Move,
    /// 邊帶：縮放那條邊／角。
    Edge(Edge),
}

/// 視窗座標 (px, py) 壓在哪：邊帶回邊角、框內回 `Move`、視窗外回 `None`。
///
/// 邊帶厚度是框外留白加框線內側，角落區再寬一點才好抓到角落把手。
pub fn hit_at(px: i32, py: i32, w: i32, h: i32) -> Option<Hit> {
    let (outer_w, outer_h) = (w + 2 * MARGIN, h + 2 * MARGIN);
    if !point_in_rect(px, py, 0, 0, outer_w, outer_h) {
        return None;
    }
    let band = MARGIN + INNER_BAND;
    let edge = edge_at_bands(px, py, 0, 0, outer_w, outer_h, band, band + HANDLE / 2);
    Some(edge.map_or(Hit::Move, Hit::Edge))
}

/// 框選框視窗的外框（螢幕 px）：矩形四邊各擴 `MARGIN`。
pub fn window_rect((x, y, w, h): Rect) -> Rect {
    (x - MARGIN, y - MARGIN, w + 2 * MARGIN, h + 2 * MARGIN)
}

/// 游標壓在 `hit` 時的滑鼠游標：框內是移動，邊角是對應方向的縮放。
pub fn cursor(hit: Hit) -> CursorIcon {
    match hit {
        Hit::Move => CursorIcon::Move,
        Hit::Edge(Edge::N | Edge::S) => CursorIcon::ResizeVertical,
        Hit::Edge(Edge::E | Edge::W) => CursorIcon::ResizeHorizontal,
        Hit::Edge(Edge::Nw | Edge::Se) => CursorIcon::ResizeNwSe,
        Hit::Edge(Edge::Ne | Edge::Sw) => CursorIcon::ResizeNeSw,
    }
}

/// 八個把手在視窗座標的中心點（矩形本身從 `(MARGIN, MARGIN)` 起算）。
pub fn handles(w: i32, h: i32) -> [(Edge, (i32, i32)); 8] {
    let (left, right) = (MARGIN, MARGIN + w);
    let (top, bottom) = (MARGIN, MARGIN + h);
    let (mid_x, mid_y) = (MARGIN + w / 2, MARGIN + h / 2);
    [
        (Edge::Nw, (left, top)),
        (Edge::N, (mid_x, top)),
        (Edge::Ne, (right, top)),
        (Edge::W, (left, mid_y)),
        (Edge::E, (right, mid_y)),
        (Edge::Sw, (left, bottom)),
        (Edge::S, (mid_x, bottom)),
        (Edge::Se, (right, bottom)),
    ]
}

struct Drag {
    hit: Hit,
    start_screen: (i32, i32),
    start_rect: Rect,
}

/// 框的狀態；視窗與滑鼠事件由宿主托管，以螢幕座標餵進來。
pub struct RegionBox {
    rect: Rect,
    drag: Option<Drag>,
}

impl RegionBox {
    /// 以螢幕矩形 `rect` 開框。
    pub fn new(rect: Rect) -> Self {
        Self { rect, drag: None }
    }

    /// 目前的框選矩形（螢幕座標）。
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// 按下：`window_point` 是相對視窗左上角的位置，用來判定壓在哪；落在視窗外則不開始拖曳。
    pub fn press(&mut self, screen: (i32, i32), window_point: (i32, i32)) {
        let (_, _, w, h) = self.rect;
        self.drag = hit_at(window_point.0, window_point.1, w, h).map(|hit| Drag {
            hit,
            start_screen: screen,
            start_rect: self.rect,
        });
    }

    /// 拖曳中：回傳新矩形，供宿主搬移視窗；沒在拖曳回 `None`。
    pub fn drag(&mut self, screen: (i32, i32)) -> Option<Rect> {
        let drag = self.drag.as_ref()?;
        let (dx, dy) = (
            screen.0 - drag.start_screen.0,
            screen.1 - drag.start_screen.1,
        );
        let (x, y, w, h) = drag.start_rect;
        self.rect = match drag.hit {
            Hit::Move => {
                let (x, y) = moved_to(x, y, dx, dy);
                (x, y, w, h)
            }
            Hit::Edge(edge) => resized_edge(edge, x, y, w, h, dx, dy, MIN_SIZE, MIN_SIZE),
        };
        Some(self.rect)
    }

    /// 放開：矩形有改變就回報；位移在點擊門檻內視為誤觸，矩形還原、不回報。
    pub fn release(&mut self, screen: (i32, i32)) -> Option<Rect> {
        let drag = self.drag.take()?;
        let (dx, dy) = (
            screen.0 - drag.start_screen.0,
            screen.1 - drag.start_screen.1,
        );
        if is_click(dx, dy) {
            self.rect = drag.start_rect;
            return None;
        }
        if self.rect == drag.start_rect {
            return None;
        }
        log(format!("[region] box adjusted rect={:?}", self.rect));
        Some(self.rect)
    }

    /// 正在拖曳的是框內還是哪條邊；沒在拖曳回 `None`。
    pub fn dragging(&self) -> Option<Hit> {
        self.drag.as_ref().map(|d| d.hit)
    }

    /// 畫框線與八個把手；`ui` 的左上角對齊視窗左上角，px 依縮放換成 DIP。
    pub fn paint(&self, ui: &mut egui::Ui) {
        let (_, _, w, h) = self.rect;
        let origin = ui.max_rect().min;
        let ppp = ui.ctx().pixels_per_point();
        let at = |x: i32, y: i32| origin + vec2(x as f32, y as f32) / ppp;
        let painter = ui.painter();
        let line = egui::Rect::from_min_size(at(MARGIN, MARGIN), vec2(w as f32, h as f32) / ppp);
        painter.rect_stroke(line, 0.0, Stroke::new(LINE, FG_UPDATE), StrokeKind::Middle);
        let size = vec2(HANDLE as f32, HANDLE as f32) / ppp;
        for (_, (cx, cy)) in handles(w, h) {
            painter.rect_filled(
                egui::Rect::from_center_size(at(cx, cy), size),
                0.0,
                FG_UPDATE,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT: Rect = (100, 100, 300, 200);

    fn drag_through(
        b: &mut RegionBox,
        window: (i32, i32),
        by: (i32, i32),
    ) -> (Option<Rect>, Option<Rect>) {
        let start = (1000 + window.0, 1000 + window.1);
        let end = (start.0 + by.0, start.1 + by.1);
        b.press(start, window);
        let dragged = b.drag(end);
        (dragged, b.release(end))
    }

    #[test]
    fn hit_at_reports_edges_corners_and_move() {
        let cases = [
            ((MARGIN + 300, MARGIN + 200), Some(Hit::Edge(Edge::Se))),
            ((MARGIN, MARGIN), Some(Hit::Edge(Edge::Nw))),
            ((MARGIN + 300, MARGIN + 100), Some(Hit::Edge(Edge::E))),
            ((MARGIN + 150, MARGIN + 200), Some(Hit::Edge(Edge::S))),
            ((MARGIN + 60, MARGIN), Some(Hit::Edge(Edge::N))),
            (
                (MARGIN + 60, MARGIN + INNER_BAND - 1),
                Some(Hit::Edge(Edge::N)),
            ),
            ((MARGIN + 60, MARGIN + INNER_BAND), Some(Hit::Move)),
            ((MARGIN + 60, 0), Some(Hit::Edge(Edge::N))),
            ((MARGIN + 300 + 1, MARGIN + 60), Some(Hit::Edge(Edge::E))),
            ((MARGIN + 150, MARGIN + 100), Some(Hit::Move)),
            ((-1, MARGIN + 100), None),
        ];
        for ((px, py), expected) in cases {
            assert_eq!(hit_at(px, py, 300, 200), expected, "point ({px}, {py})");
        }
    }

    #[test]
    fn dragging_the_inside_moves_the_rect() {
        let mut b = RegionBox::new(RECT);
        let (dragged, released) = drag_through(&mut b, (50, 50), (20, 10));
        assert_eq!(dragged, Some((120, 110, 300, 200)));
        assert_eq!(released, Some((120, 110, 300, 200)));
        assert_eq!(b.rect(), (120, 110, 300, 200));
    }

    #[test]
    fn dragging_an_edge_resizes_only_that_edge() {
        let mut b = RegionBox::new(RECT);
        let (_, released) = drag_through(&mut b, (MARGIN + 60, MARGIN), (20, 10));
        assert_eq!(released, Some((100, 110, 300, 190)));

        let mut b = RegionBox::new(RECT);
        let (_, released) = drag_through(&mut b, (MARGIN + 300, MARGIN + 200), (40, 30));
        assert_eq!(released, Some((100, 100, 340, 230)));
    }

    #[test]
    fn resizing_stops_at_the_minimum_size() {
        let mut b = RegionBox::new(RECT);
        drag_through(&mut b, (MARGIN, MARGIN), (1000, 1000));
        assert_eq!(
            b.rect(),
            (
                100 + 300 - MIN_SIZE,
                100 + 200 - MIN_SIZE,
                MIN_SIZE,
                MIN_SIZE
            )
        );
    }

    #[test]
    fn a_click_restores_the_rect_and_reports_nothing() {
        let mut b = RegionBox::new(RECT);
        let (dragged, released) = drag_through(&mut b, (MARGIN + 60, MARGIN), (2, 1));
        assert_eq!(dragged, Some((100, 101, 300, 199)));
        assert_eq!(released, None);
        assert_eq!(b.rect(), RECT);
    }

    #[test]
    fn release_without_change_reports_nothing() {
        let mut b = RegionBox::new(RECT);
        let start = (1050, 1050);
        b.press(start, (50, 50));
        b.drag((1100, 1100));
        b.drag(start);
        assert_eq!(b.release((1100, 1100)), None);
        assert_eq!(b.rect(), RECT);
        assert_eq!(b.release(start), None);
        assert_eq!(b.drag(start), None);
    }

    #[test]
    fn handles_sit_on_the_corners_and_edge_midpoints() {
        let all = handles(300, 200);
        let at = |e: Edge| all.iter().find(|(k, _)| *k == e).unwrap().1;
        assert_eq!(all.len(), 8);
        assert_eq!(at(Edge::Nw), (MARGIN, MARGIN));
        assert_eq!(at(Edge::Se), (MARGIN + 300, MARGIN + 200));
        assert_eq!(at(Edge::N), (MARGIN + 150, MARGIN));
        assert_eq!(at(Edge::E), (MARGIN + 300, MARGIN + 100));
    }

    #[test]
    fn window_rect_extends_the_rect_by_the_margin_on_each_side() {
        assert_eq!(
            window_rect(RECT),
            (
                100 - MARGIN,
                100 - MARGIN,
                300 + 2 * MARGIN,
                200 + 2 * MARGIN
            )
        );
    }

    #[test]
    fn paint_converts_pixels_to_points_on_scaled_displays() {
        use eframe::egui::Shape;
        use egui_kittest::Harness;

        let b = RegionBox::new(RECT);
        let mut h = Harness::builder()
            .with_pixels_per_point(2.0)
            .build_ui(move |ui| b.paint(ui));
        h.step();
        let line = h
            .output()
            .shapes
            .iter()
            .find_map(|c| match &c.shape {
                Shape::Rect(r) if r.stroke.width == LINE => Some(r.rect),
                _ => None,
            })
            .expect("frame line drawn");
        let origin = line.min - vec2(MARGIN as f32, MARGIN as f32) / 2.0;
        assert_eq!(line.size(), vec2(150.0, 100.0));
        let handle = h
            .output()
            .shapes
            .iter()
            .find_map(|c| match &c.shape {
                Shape::Rect(r) if r.fill == FG_UPDATE => Some(r.rect),
                _ => None,
            })
            .expect("handle drawn");
        let nw = vec2(MARGIN as f32, MARGIN as f32) / 2.0;
        assert_eq!(handle.center(), origin + nw);
        assert_eq!(handle.size(), vec2(HANDLE as f32, HANDLE as f32) / 2.0);
    }

    #[test]
    fn only_the_line_and_the_handles_are_painted() {
        use eframe::egui::Shape;
        use egui_kittest::Harness;

        let b = RegionBox::new(RECT);
        let mut h = Harness::builder().build_ui(move |ui| b.paint(ui));
        h.step();
        let colours: Vec<_> = h
            .output()
            .shapes
            .iter()
            // 第一個是 harness 自己的面板底色
            .skip(1)
            .filter_map(|c| match &c.shape {
                Shape::Rect(r) if r.stroke.width > 0.0 => Some(r.stroke.color),
                Shape::Rect(r) => Some(r.fill),
                _ => None,
            })
            .collect();
        // 1 條框線 + 8 個把手，沒有暗色外圈、把手也沒有暗色邊
        assert_eq!(colours.len(), 9);
        assert!(colours.iter().all(|c| *c == FG_UPDATE));
    }
}
