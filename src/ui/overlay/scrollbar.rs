//! 自繪細捲軸：圓角滑塊加深色描邊，滑鼠移上去變亮；內容塞得下時整條隱藏。

use eframe::egui::{Color32, Rect, Ui, Vec2};

use crate::ui::palette::OUTLINE;

/// 捲軸寬度（DIP）。
pub const WIDTH: f32 = 8.0;
/// 滑塊最短長度（DIP）：訊息很多時仍抓得住。
pub const MIN_THUMB: i32 = 20;
/// 滑塊顏色。
pub const THUMB: Color32 = Color32::from_rgb(0x8a, 0x8a, 0xb0);
/// 滑鼠在捲軸上時的滑塊顏色。
pub const THUMB_HOVER: Color32 = Color32::from_rgb(0xc0, 0xc0, 0xe0);
const PAD: f32 = 2.0;

/// 滑塊在軌道上的 (top, bottom) 像素範圍；不需捲動時為 None，比例算出太短就撐到 `min_thumb`。
pub fn thumb_span(first: f64, last: f64, track_height: i32, min_thumb: i32) -> Option<(i32, i32)> {
    if track_height <= 0 || last - first >= 1.0 {
        return None;
    }
    let px = |f: f64| (f * f64::from(track_height)).round_ties_even() as i32;
    let (mut top, mut bottom) = (px(first), px(last));
    if bottom - top < min_thumb {
        bottom = track_height.min(top + min_thumb);
        top = 0.max(bottom - min_thumb);
    }
    Some((top, bottom))
}

/// 拖曳滑塊時要捲到的比例：游標位置扣掉抓取點偏移，夾在 0–1。
pub fn scroll_fraction(pointer_y: f64, grab_offset: f64, track_height: f64) -> f64 {
    if track_height <= 0.0 {
        return 0.0;
    }
    (pointer_y / track_height - grab_offset).clamp(0.0, 1.0)
}

/// 捲軸的拖曳狀態與上一幀畫出的可見範圍。
pub struct Scrollbar {
    /// 拖曳中時是抓取點相對滑塊頂端的比例偏移。
    grab: Option<f64>,
    view: (f64, f64),
}

impl Default for Scrollbar {
    fn default() -> Self {
        Self {
            grab: None,
            view: (0.0, 1.0),
        }
    }
}

impl Scrollbar {
    /// 處理軌道 `track` 上的拖曳（以上一幀畫出的滑塊為準），回傳要捲到的頂端比例；滑塊隱藏時不接滑鼠。
    ///
    /// 直接讀指標而不註冊 egui 元件：元件的命中半徑（5pt）會吃掉右側 6px 縮放帶的大半。
    pub fn drag(&mut self, ui: &Ui, track: Rect) -> Option<f64> {
        let height = f64::from(track.height());
        let (pressed, down, pos) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.interact_pos(),
            )
        });
        if !down {
            self.grab = None;
        }
        let pos = pos?;
        let y = f64::from(pos.y - track.min.y);
        let shown = thumb_span(self.view.0, self.view.1, height as i32, MIN_THUMB).is_some();
        if pressed && shown && track.contains(pos) {
            self.grab = Some(y / height.max(1.0) - self.view.0);
        }
        self.grab.map(|grab| scroll_fraction(y, grab, height))
    }

    /// 畫出可見範圍為 `first`–`last` 比例的滑塊；不需捲動時不畫。
    pub fn paint(&mut self, ui: &Ui, track: Rect, first: f64, last: f64) {
        self.view = (first, last);
        let Some((top, bottom)) = thumb_span(first, last, track.height() as i32, MIN_THUMB) else {
            return;
        };
        let active = self.grab.is_some() || ui.rect_contains_pointer(track);
        let color = if active { THUMB_HOVER } else { THUMB };
        let thumb = Rect::from_min_max(
            track.min + Vec2::new(PAD, top as f32),
            track.min + Vec2::new(WIDTH - PAD, bottom as f32),
        );
        // 底板半透明，壓在亮色畫面上沒描邊的滑塊會融進背景
        let painter = ui.painter();
        painter.rect_filled(thumb.expand(1.0), thumb.width() / 2.0 + 1.0, OUTLINE);
        painter.rect_filled(thumb, thumb.width() / 2.0, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_span_matches_python() {
        // 對照 Python 版 overlay 測試的字面值
        assert_eq!(thumb_span(0.0, 1.0, 200, MIN_THUMB), None);
        assert_eq!(thumb_span(0.0, 0.5, 0, MIN_THUMB), None);
        assert_eq!(thumb_span(0.0, 0.5, 200, MIN_THUMB), Some((0, 100)));
        assert_eq!(thumb_span(0.5, 1.0, 200, MIN_THUMB), Some((100, 200)));
        assert_eq!(thumb_span(0.0, 0.02, 200, 20), Some((0, 20)));
        assert_eq!(thumb_span(0.98, 1.0, 200, 20), Some((180, 200)));
        // Python round 是銀行家進位：0.0125 × 200 = 2.5 → 2
        assert_eq!(thumb_span(0.0125, 0.5, 200, 20), Some((2, 100)));
    }

    #[test]
    fn scroll_fraction_matches_python() {
        assert_eq!(scroll_fraction(100.0, 0.0, 200.0), 0.5);
        assert_eq!(scroll_fraction(100.0, 0.25, 200.0), 0.25);
        assert_eq!(scroll_fraction(-50.0, 0.0, 200.0), 0.0);
        assert_eq!(scroll_fraction(400.0, 0.0, 200.0), 1.0);
        assert_eq!(scroll_fraction(10.0, 0.0, 0.0), 0.0);
    }

    #[test]
    fn colours_match_python() {
        assert_eq!(THUMB, Color32::from_hex("#8a8ab0").unwrap());
        assert_eq!(THUMB_HOVER, Color32::from_hex("#c0c0e0").unwrap());
    }
}
