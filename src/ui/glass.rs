//! 玻璃邊框：overlay、泡泡與框選卡片共用的邊緣（外圈淡白邊、往內漸弱的內光暈），以及自己畫圓角的視窗填色。

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};

/// Windows 11 圓角視窗的圓角半徑（DIP）。
pub const WINDOW_RADIUS: f32 = 8.0;
/// 外圈淡白邊的亮度。
const RIM: f32 = 0.16;
/// 內光暈最內一圈的亮度；往內每 px 遞減。
const GLOW: f32 = 0.05;
/// 內光暈最多幾圈（px）；小視窗（泡泡）依邊長縮減。
const GLOW_RINGS: usize = 6;

/// 在 `rect` 外緣畫玻璃邊框；`radius` 是圓角半徑（等於半邊長時為圓），`opacity` 跟著視窗的不透明度淡出。
pub fn rim(painter: &Painter, rect: Rect, radius: f32, opacity: f32) {
    let radius = radius.min(rect.width().min(rect.height()) / 2.0);
    let rings = glow_rings(rect);
    for i in 0..rings {
        let inset = 1.5 + i as f32;
        let alpha = GLOW * (1.0 - i as f32 / rings as f32) * opacity;
        painter.rect_stroke(
            rect.shrink(inset),
            (radius - inset).max(0.0),
            Stroke::new(1.0, white(alpha)),
            StrokeKind::Inside,
        );
    }
    let edge = Stroke::new(1.0, white(RIM * opacity));
    painter.rect_stroke(rect, radius, edge, StrokeKind::Inside);
}

/// `piece` 與圓角視窗 `window`（半徑 [`WINDOW_RADIUS`]）交集的填色：碰到視窗四角的部分沿圓角切掉，其餘與矩形相同。
///
/// 給不靠 DWM 圓角的視窗用（DWM 的圓角會附帶陰影）：凡是貼到視窗角落的底色都要經過這裡。
pub fn window_piece(piece: Rect, window: Rect, color: Color32) -> Shape {
    let mut points = window_outline(window);
    for (inside, cut) in [
        (
            Box::new(|p: Pos2| p.x >= piece.min.x) as Box<dyn Fn(Pos2) -> bool>,
            Cut::X(piece.min.x),
        ),
        (Box::new(|p: Pos2| p.x <= piece.max.x), Cut::X(piece.max.x)),
        (Box::new(|p: Pos2| p.y >= piece.min.y), Cut::Y(piece.min.y)),
        (Box::new(|p: Pos2| p.y <= piece.max.y), Cut::Y(piece.max.y)),
    ] {
        points = clip(&points, inside, cut);
    }
    if points.len() < 3 {
        return Shape::Noop;
    }
    Shape::convex_polygon(points, color, Stroke::NONE)
}

/// 裁切線：垂直線 x＝值或水平線 y＝值。
#[derive(Clone, Copy)]
enum Cut {
    X(f32),
    Y(f32),
}

/// Sutherland–Hodgman：把凸多邊形 `points` 裁到 `inside` 那一側。
fn clip(points: &[Pos2], inside: impl Fn(Pos2) -> bool, cut: Cut) -> Vec<Pos2> {
    let cross = |a: Pos2, b: Pos2| {
        let t = match cut {
            Cut::X(x) => (x - a.x) / (b.x - a.x),
            Cut::Y(y) => (y - a.y) / (b.y - a.y),
        };
        a.lerp(b, t)
    };
    let mut out = Vec::new();
    for (i, &b) in points.iter().enumerate() {
        let a = points[(i + points.len() - 1) % points.len()];
        match (inside(a), inside(b)) {
            (true, true) => out.push(b),
            (true, false) => out.push(cross(a, b)),
            (false, true) => out.extend([cross(a, b), b]),
            (false, false) => {}
        }
    }
    out
}

/// 圓角視窗外形的頂點（順時針、每個圓角切 8 段）。
fn window_outline(window: Rect) -> Vec<Pos2> {
    let r = WINDOW_RADIUS.min(window.width().min(window.height()) / 2.0);
    let corners = [
        (Pos2::new(window.min.x + r, window.min.y + r), 180.0_f32),
        (Pos2::new(window.max.x - r, window.min.y + r), 270.0),
        (Pos2::new(window.max.x - r, window.max.y - r), 0.0),
        (Pos2::new(window.min.x + r, window.max.y - r), 90.0),
    ];
    corners
        .into_iter()
        .flat_map(|(centre, start)| {
            (0..=8).map(move |step| {
                let angle = (start + 90.0 * step as f32 / 8.0).to_radians();
                centre + Vec2::new(angle.cos(), angle.sin()) * r
            })
        })
        .collect()
}

/// 內光暈圈數：短邊每 16px 一圈，最多 [`GLOW_RINGS`]。
fn glow_rings(rect: Rect) -> usize {
    GLOW_RINGS.min((rect.width().min(rect.height()) / 16.0) as usize)
}

fn white(alpha: f32) -> Color32 {
    Color32::from_white_alpha((alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Pos2, Vec2};

    use super::*;

    const WINDOW: Rect = Rect {
        min: Pos2::new(0.0, 0.0),
        max: Pos2::new(400.0, 300.0),
    };

    fn points(shape: Shape) -> Vec<Pos2> {
        match shape {
            Shape::Path(path) => path.points,
            other => panic!("not a polygon: {other:?}"),
        }
    }

    #[test]
    fn a_piece_away_from_the_corners_stays_a_plain_rectangle() {
        let piece = Rect::from_min_max(Pos2::new(0.0, 40.0), Pos2::new(400.0, 200.0));
        let got = points(window_piece(piece, WINDOW, Color32::RED));
        // 鞋帶公式：面積和矩形相同就沒被切到
        let area: f32 = (0..got.len())
            .map(|i| {
                let (a, b) = (got[i], got[(i + 1) % got.len()]);
                a.x * b.y - b.x * a.y
            })
            .sum::<f32>()
            .abs()
            / 2.0;
        assert!((area - piece.area()).abs() < 0.5, "{area}");
    }

    #[test]
    fn a_piece_in_a_corner_is_cut_along_the_rounded_corner() {
        let bar = Rect::from_min_max(Pos2::ZERO, Pos2::new(400.0, 28.0));
        let got = points(window_piece(bar, WINDOW, Color32::RED));
        let centre = Pos2::new(WINDOW_RADIUS, WINDOW_RADIUS);
        for p in got
            .iter()
            .filter(|p| p.x < WINDOW_RADIUS && p.y < WINDOW_RADIUS)
        {
            assert!((p.distance(centre) - WINDOW_RADIUS).abs() < 0.01, "{p:?}");
        }
        assert!(!got.contains(&Pos2::ZERO));
        assert!(got.iter().all(|p| bar.expand(0.01).contains(*p)));
    }

    #[test]
    fn a_piece_outside_the_window_draws_nothing() {
        let piece = Rect::from_min_max(Pos2::new(500.0, 0.0), Pos2::new(600.0, 10.0));
        assert!(matches!(
            window_piece(piece, WINDOW, Color32::RED),
            Shape::Noop
        ));
    }

    #[test]
    fn small_windows_get_fewer_glow_rings() {
        let big = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0));
        assert_eq!(glow_rings(big), GLOW_RINGS);
        assert_eq!(
            glow_rings(Rect::from_min_size(Pos2::ZERO, Vec2::splat(48.0))),
            3
        );
    }
}
