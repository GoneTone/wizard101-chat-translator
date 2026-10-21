//! 玻璃邊框：overlay、泡泡與框選卡片共用的邊緣反光（外圈淡白邊、頂邊高光、兩側上半反光、往內漸弱的內光暈）。

use eframe::egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};

/// Windows 11 圓角視窗的圓角半徑（DIP）。
pub const WINDOW_RADIUS: f32 = 8.0;
/// 邊框整體亮度。
const STRENGTH: f32 = 1.5;
/// 外圈各處都有的底亮度。
const RIM: f32 = 0.16;
/// 頂邊中央高光的峰值。
const TOP: f32 = 0.40;
/// 兩側上半反光的峰值。
const SIDE: f32 = 0.22;
/// 內光暈最內一圈的亮度；往內每 px 遞減。
const GLOW: f32 = 0.05;
/// 內光暈最多幾圈（px）；小視窗（泡泡）依邊長縮減。
const GLOW_RINGS: usize = 6;
/// 每個圓角切幾段。
const ARC_STEPS: usize = 8;
/// 每條直邊切幾段：高光沿邊漸變，段太少會看出色階。
const EDGE_STEPS: usize = 48;

/// 在 `rect` 外緣畫玻璃邊框；`radius` 是圓角半徑（等於半邊長時為圓），`opacity` 跟著視窗的不透明度淡出。
pub fn rim(painter: &Painter, rect: Rect, radius: f32, opacity: f32) {
    let strength = STRENGTH * opacity;
    let radius = radius.min(rect.width().min(rect.height()) / 2.0);
    let rings = glow_rings(rect);
    for i in 0..rings {
        let inset = 1.5 + i as f32;
        let alpha = GLOW * (1.0 - i as f32 / rings as f32) * strength;
        painter.rect_stroke(
            rect.shrink(inset),
            (radius - inset).max(0.0),
            Stroke::new(1.0, white(alpha)),
            StrokeKind::Inside,
        );
    }
    painter.add(outline_mesh(rect, radius, |p| {
        white(edge_alpha(rect, radius, p) * strength)
    }));
}

/// 內光暈圈數：短邊每 16px 一圈，最多 [`GLOW_RINGS`]。
fn glow_rings(rect: Rect) -> usize {
    GLOW_RINGS.min((rect.width().min(rect.height()) / 16.0) as usize)
}

/// 外圈某點的亮度：底亮度加上頂邊中央高光或兩側上半反光（取大者）。
fn edge_alpha(rect: Rect, radius: f32, p: Pos2) -> f32 {
    let centre = (1.0 - (p.x - rect.center().x).abs() / (rect.width() / 2.0)).clamp(0.0, 1.0);
    let near_top = (1.0 - (p.y - rect.min.y) / radius.max(1.0)).clamp(0.0, 1.0);
    let upper = (1.0 - (p.y - rect.min.y - radius) / (rect.height() / 2.0)).clamp(0.0, 1.0);
    RIM + (TOP * centre * near_top).max(SIDE * upper)
}

fn white(alpha: f32) -> Color32 {
    Color32::from_white_alpha((alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// 沿圓角矩形外緣、寬約 1px 的漸層帶：內外兩側淡到透明當作反鋸齒，中間取 `color(點)`。
fn outline_mesh(rect: Rect, radius: f32, color: impl Fn(Pos2) -> Color32) -> Shape {
    let mut mesh = Mesh::default();
    let points = outline(rect, radius);
    for &(p, inward) in &points {
        mesh.colored_vertex(p - inward * 0.5, Color32::TRANSPARENT);
        mesh.colored_vertex(p + inward * 0.5, color(p));
        mesh.colored_vertex(p + inward * 1.5, Color32::TRANSPARENT);
    }
    let n = points.len() as u32;
    for i in 0..n {
        let (a, b) = (i * 3, (i + 1) % n * 3);
        for k in 0..2 {
            mesh.add_triangle(a + k, b + k, b + k + 1);
            mesh.add_triangle(a + k, b + k + 1, a + k + 1);
        }
    }
    Shape::mesh(mesh)
}

/// 圓角矩形外緣上順時針的點與朝內的法向量，從左上角圓弧起算。
fn outline(rect: Rect, radius: f32) -> Vec<(Pos2, Vec2)> {
    let r = radius;
    let corners = [
        (Pos2::new(rect.min.x + r, rect.min.y + r), 180.0_f32),
        (Pos2::new(rect.max.x - r, rect.min.y + r), 270.0),
        (Pos2::new(rect.max.x - r, rect.max.y - r), 0.0),
        (Pos2::new(rect.min.x + r, rect.max.y - r), 90.0),
    ];
    let mut points = Vec::new();
    for (i, &(centre, start)) in corners.iter().enumerate() {
        for step in 0..ARC_STEPS {
            let angle = (start + 90.0 * step as f32 / ARC_STEPS as f32).to_radians();
            let out = Vec2::new(angle.cos(), angle.sin());
            points.push((centre + out * r, -out));
        }
        // 圓弧終點接到下一個圓角的起點：中間是直邊
        let end = (start + 90.0).to_radians();
        let out = Vec2::new(end.cos(), end.sin());
        let (from, to) = (centre + out * r, corners[(i + 1) % 4].0 + out * r);
        for step in 0..EDGE_STEPS {
            points.push((from.lerp(to, step as f32 / EDGE_STEPS as f32), -out));
        }
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT: Rect = Rect {
        min: Pos2::new(0.0, 0.0),
        max: Pos2::new(400.0, 300.0),
    };

    #[test]
    fn top_centre_is_the_brightest_and_the_bottom_only_has_the_base_rim() {
        let top = edge_alpha(RECT, 8.0, Pos2::new(200.0, 0.0));
        let side = edge_alpha(RECT, 8.0, Pos2::new(0.0, 40.0));
        let bottom = edge_alpha(RECT, 8.0, Pos2::new(200.0, 300.0));
        assert!(top > side && side > bottom, "{top} {side} {bottom}");
        assert_eq!(bottom, RIM);
    }

    #[test]
    fn outline_stays_on_the_rounded_rect_and_normals_point_inward() {
        for (p, inward) in outline(RECT, 8.0) {
            assert!(RECT.expand(0.01).contains(p), "{p:?}");
            assert!(RECT.contains(p + inward * 2.0), "{p:?} {inward:?}");
        }
    }

    #[test]
    fn a_circle_outline_keeps_every_point_on_the_circle() {
        let square = Rect::from_min_size(Pos2::ZERO, Vec2::splat(48.0));
        for (p, _) in outline(square, 24.0) {
            let off = (p - square.center()).length();
            // 直邊長度為 0：所有點都在圓上
            assert!((off - 24.0).abs() < 0.01, "{p:?} {off}");
        }
    }

    #[test]
    fn small_windows_get_fewer_glow_rings() {
        assert_eq!(glow_rings(RECT), GLOW_RINGS);
        assert_eq!(
            glow_rings(Rect::from_min_size(Pos2::ZERO, Vec2::splat(48.0))),
            3
        );
    }
}
