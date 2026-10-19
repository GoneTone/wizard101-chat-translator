//! 自繪文字選取的共用零件：字元區間取字、反白矩形、拖出範圍的自動捲動、複製鍵與滾輪換算。
//!
//! overlay 的訊息框選與框選結果卡都在自己排版的 galley 上做選取，選取端點一律是字元索引。

use std::time::Duration;

use eframe::egui::text::CharIndex;
use eframe::egui::{Context, Event, Galley, Key, MouseWheelUnit, Rect, Ui};

/// 拖出範圍時的自動捲動節奏（秒）。
pub const AUTOSCROLL_INTERVAL: f64 = 0.040;
/// 每輪最少捲幾像素：剛越界時要慢，才停得準。
const AUTOSCROLL_MIN: i32 = 2;
const AUTOSCROLL_MAX: i32 = 24;

/// `text` 第 `[lo, hi)` 個字元（`hi` 可超過字數，代表到結尾）。
pub fn char_range_text(text: &str, lo: usize, hi: usize) -> String {
    text.chars().skip(lo).take(hi.saturating_sub(lo)).collect()
}

/// 拖到範圍之外時每一輪要捲動的像素（負值往上，落在區內回 0）；離邊界越遠越快，但設上限才選得準。
pub fn autoscroll_pixels(y: i32, top: i32, bottom: i32) -> i32 {
    if y < top {
        -AUTOSCROLL_MAX.min(AUTOSCROLL_MIN + (top - y) / 4)
    } else if y > bottom {
        AUTOSCROLL_MAX.min(AUTOSCROLL_MIN + (y - bottom) / 4)
    } else {
        0
    }
}

/// 拖曳選字中每幀呼叫：到了 `next_tick` 就排下一輪並回傳指標 `y` 對 `view` 的捲動量（正值＝往下捲），否則回 0；並排好下一次重繪。
pub fn autoscroll_tick(ctx: &Context, next_tick: &mut f64, time: f64, y: f32, view: Rect) -> f32 {
    ctx.request_repaint_after(Duration::from_secs_f64(AUTOSCROLL_INTERVAL));
    if time < *next_tick {
        return 0.0;
    }
    *next_tick = time + AUTOSCROLL_INTERVAL;
    autoscroll_pixels(y.floor() as i32, view.min.y as i32, view.max.y as i32 - 1) as f32
}

/// 排版 `galley` 中 `[lo, hi)` 要反白的矩形（相對 galley 原點，每個視覺行一個）。
pub fn highlight_rects(galley: &Galley, lo: usize, hi: usize) -> Vec<Rect> {
    let mut rects = Vec::new();
    let mut first = 0;
    for row in &galley.rows {
        let (a, b) = (
            lo.max(first),
            hi.min(first + row.char_count_excluding_newline().0),
        );
        if a < b {
            let x = |i: usize| row.pos.x + row.x_offset(CharIndex(i - first));
            rects.push(Rect::from_x_y_ranges(
                x(a)..=x(b),
                row.min_y()..=row.max_y(),
            ));
        }
        first += row.char_count_including_newline().0;
    }
    rects
}

/// 這個輸入事件是不是複製（Ctrl+C，含 Ctrl+Shift+C）。
pub fn is_copy(event: &Event) -> bool {
    // egui-winit 只把不帶 Shift 的 Ctrl+C 轉成 Copy；Ctrl+Shift+C（Tk 的 <Control-C>）仍是 Key
    match event {
        Event::Copy => true,
        Event::Key {
            key: Key::C,
            pressed: true,
            modifiers,
            ..
        } => modifiers.ctrl || modifiers.command,
        _ => false,
    }
}

/// 本幀的滾輪量換成內容位移（DIP）：一格捲視口高度的十分之一，與 Tk 的 `yview_scroll(units)` 相同。
pub fn wheel_delta(ui: &Ui, viewport_h: f32) -> f32 {
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

/// 本幀 egui 排入的複製文字（測試用：看 egui 收到什麼，不碰系統剪貼簿）。
#[cfg(test)]
pub fn copied_texts(ctx: &Context) -> Vec<String> {
    use eframe::egui::OutputCommand;
    ctx.output(|o| {
        o.commands
            .iter()
            .filter_map(|c| match c {
                OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use eframe::egui::{Color32, FontId, Vec2, pos2};

    use super::*;
    use crate::ui::fonts_egui::{self, tk_pt};

    const WRAPPED: &str = "Hello wizard friends 這是一段中英混排的訊息內容用來測試換行";

    fn layout(text: &str, wrap: f32) -> Arc<Galley> {
        let ctx = Context::default();
        fonts_egui::install(&ctx, "zh-TW");
        let mut galley = None;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            galley = Some(ui.painter().layout(
                text.to_owned(),
                FontId::proportional(tk_pt(11.0)),
                Color32::PLACEHOLDER,
                wrap,
            ));
        });
        output.textures_delta.clear();
        galley.unwrap()
    }

    #[test]
    fn char_range_text_counts_characters() {
        assert_eq!(char_range_text("原文abc", 1, 4), "文ab");
        assert_eq!(char_range_text("abc", 1, usize::MAX), "bc");
        assert_eq!(char_range_text("abc", 2, 2), "");
    }

    #[test]
    fn autoscroll_pixels_matches_python() {
        assert_eq!(autoscroll_pixels(500, 400, 600), 0);
        assert_eq!(autoscroll_pixels(400, 400, 600), 0);
        assert_eq!(autoscroll_pixels(600, 400, 600), 0);
        assert!(autoscroll_pixels(392, 400, 600) < 0);
        assert!(autoscroll_pixels(608, 400, 600) > 0);
        let (near, far) = (
            autoscroll_pixels(610, 400, 600),
            autoscroll_pixels(650, 400, 600),
        );
        assert!(0 < near && near < far && far < 24);
        assert_eq!(autoscroll_pixels(9000, 400, 600), 24);
        assert_eq!(autoscroll_pixels(-9000, 400, 600), -24);
    }

    #[test]
    fn autoscroll_tick_scrolls_once_per_interval() {
        let ctx = Context::default();
        let view = Rect::from_min_max(pos2(0.0, 400.0), pos2(100.0, 601.0));
        let mut next = 1.0;
        assert_eq!(autoscroll_tick(&ctx, &mut next, 0.99, 9000.0, view), 0.0);
        assert_eq!(autoscroll_tick(&ctx, &mut next, 1.0, 9000.0, view), 24.0);
        assert_eq!(next, 1.0 + AUTOSCROLL_INTERVAL);
        assert_eq!(autoscroll_tick(&ctx, &mut next, 1.01, 9000.0, view), 0.0);
        assert_eq!(autoscroll_tick(&ctx, &mut next, 2.0, -9000.0, view), -24.0);
    }

    #[test]
    fn highlight_x_agrees_with_hit_test() {
        // 反白的 x 與 hit-test 必須認同同一個字元位置，否則反白會整片偏掉
        let galley = layout(WRAPPED, 120.0);
        assert!(galley.rows.len() > 1, "test text must wrap");
        let first = galley.rows[0].char_count_excluding_newline().0;
        let middle = first / 2;
        let rect = highlight_rects(&galley, middle, first)[0];
        assert_eq!(rect.min.y, galley.rows[0].min_y());
        let probe = Vec2::new(rect.min.x + 0.5, rect.center().y);
        assert_eq!(galley.cursor_from_pos(probe).index.0, middle);
    }

    #[test]
    fn highlight_spans_every_visual_line_and_empty_ranges_have_none() {
        let galley = layout(WRAPPED, 120.0);
        let count = WRAPPED.chars().count();
        assert_eq!(highlight_rects(&galley, 0, count).len(), galley.rows.len());
        assert_eq!(
            highlight_rects(&galley, 0, usize::MAX).len(),
            galley.rows.len()
        );
        assert!(highlight_rects(&galley, 5, 5).is_empty());
    }
}
