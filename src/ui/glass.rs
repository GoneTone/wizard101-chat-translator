//! 玻璃邊框：overlay、泡泡與框選卡片共用的邊緣（外圈淡白邊、往內漸弱的內光暈）。

use eframe::egui::{Color32, Painter, Rect, Stroke, StrokeKind};

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
