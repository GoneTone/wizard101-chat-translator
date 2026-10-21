//! 字幕式描邊文字：八方向各偏移 1px 畫描邊色，再疊本色，亮色遊戲畫面上也看得清。

use std::sync::Arc;

use eframe::egui::{Color32, Galley, Painter, Pos2, Vec2};

use crate::ui::palette::OUTLINE;

/// 文字相對行原點的偏移，留給描邊往左上多出的 1px。
pub const TEXT_ORIGIN: f32 = 2.0;
/// 描邊副本的八個方向。
pub const OUTLINE_OFFSETS: [(f32, f32); 8] = [
    (-1.0, -1.0),
    (-1.0, 0.0),
    (-1.0, 1.0),
    (0.0, -1.0),
    (0.0, 1.0),
    (1.0, -1.0),
    (1.0, 0.0),
    (1.0, 1.0),
];

/// 以 `pos` 為行原點畫一行描邊文字；`galley` 用 `Color32::PLACEHOLDER` 排版，顏色在這裡給。
pub fn paint(painter: &Painter, pos: Pos2, galley: &Arc<Galley>, color: Color32) {
    let text = pos + Vec2::splat(TEXT_ORIGIN);
    for (dx, dy) in OUTLINE_OFFSETS {
        painter.galley(text + Vec2::new(dx, dy), Arc::clone(galley), OUTLINE);
    }
    painter.galley(text, Arc::clone(galley), color);
}

/// 一行（換行後）佔的高度：文字高加上原點偏移、描邊往下的 1px 與底部 2px 餘裕。
pub fn line_height(galley: &Galley) -> f32 {
    TEXT_ORIGIN + galley.size().y + 1.0 + 2.0
}

#[cfg(test)]
mod tests {
    use eframe::egui::{FontId, Shape};
    use egui_kittest::Harness;

    use super::*;
    use crate::ui::fonts_egui::{self, tk_pt};

    fn shapes(text: &'static str, origin: Pos2, color: Color32) -> Vec<(Pos2, Color32, String)> {
        let mut harness = Harness::builder()
            .with_context_setup(|ctx| fonts_egui::install(ctx, "zh-TW"))
            .build_ui(move |ui| {
                let galley = ui.painter().layout(
                    text.to_owned(),
                    FontId::proportional(tk_pt(11.0)),
                    Color32::PLACEHOLDER,
                    250.0,
                );
                paint(ui.painter(), origin, &galley, color);
            });
        harness.step();
        harness
            .output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Text(t) => Some((t.pos, t.fallback_color, t.galley.text().to_owned())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn outline_copies_first_then_the_fill_on_top() {
        let color = Color32::from_hex("#f2f2f7").unwrap();
        let drawn = shapes("hello", Pos2::new(0.0, 50.0), color);
        // 對照 Python draw_outlined 在 (0, 50) 畫出的九個文字 item 座標
        let expected = [
            (1.0, 51.0),
            (1.0, 52.0),
            (1.0, 53.0),
            (2.0, 51.0),
            (2.0, 53.0),
            (3.0, 51.0),
            (3.0, 52.0),
            (3.0, 53.0),
        ];
        assert_eq!(drawn.len(), 9);
        for ((pos, fill, text), (x, y)) in drawn.iter().zip(expected) {
            assert_eq!(
                (*pos, *fill, text.as_str()),
                (Pos2::new(x, y), OUTLINE, "hello")
            );
        }
        assert_eq!(drawn[8], (Pos2::new(2.0, 52.0), color, "hello".to_owned()));
    }

    #[test]
    fn line_height_adds_origin_outline_and_margin() {
        let ctx = eframe::egui::Context::default();
        fonts_egui::install(&ctx, "zh-TW");
        let long = "Hello wizard friends 這是一段中英混排的訊息內容用來測試換行";
        let mut heights = Vec::new();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            for wrap in [1000.0, 120.0] {
                let galley = ui.painter().layout(
                    long.to_owned(),
                    FontId::proportional(tk_pt(11.0)),
                    Color32::PLACEHOLDER,
                    wrap,
                );
                heights.push((galley.size().y, line_height(&galley)));
            }
        });
        output.textures_delta.clear();
        for (text, line) in &heights {
            assert_eq!(*line, text + 5.0);
        }
        assert!(heights[1].1 > heights[0].1, "narrow wrap must add lines");
    }
}
