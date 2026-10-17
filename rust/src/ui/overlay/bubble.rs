//! overlay 縮小後的浮動泡泡：圓形 app 圖示加未讀數徽章，點一下展開、拖曳移動，並輪詢前景決定自動展開。

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, FontId, Image, Pos2, Sense, Shape, Stroke, TextureHandle, Vec2};

use crate::logs::log;
use crate::ui::fonts_egui::tk_pt;
use crate::ui::geometry::{Rect, is_click, moved_to, point_in_rect, pull_onscreen};
use crate::ui::overlay::frame::{Shell, load_icon};
use crate::ui::palette::{BAR, GRIP};

/// 泡泡邊長（DIP）。
pub const BUBBLE_SIZE: i32 = 48;
/// 泡泡狀態下檢查前景視窗的間隔。
pub const POLL_INTERVAL: Duration = Duration::from_millis(300);
// 泡泡比主視窗再透一些，但留下限：overlay_alpha 調到最低時才不會幾乎看不見、找不回來
const ALPHA_FACTOR: f32 = 0.8;
const ALPHA_FLOOR: f32 = 0.30;
const BADGE_FONT_PT: f32 = 7.0;
const BADGE_FG: Color32 = Color32::from_rgb(0xff, 0x90, 0x90);
// 圖示右上是「文A」、右下是 101，只剩左下留白；高度對齊 101 的中心線（量圖得來的比例）
const BADGE_HOME: (f32, f32) = (0.18, 0.82);

/// 泡泡的不透明度：跟著主視窗的設定走，但再透一些（不低於下限）。
pub fn bubble_alpha(alpha: f32) -> f32 {
    ALPHA_FLOOR.max(alpha * ALPHA_FACTOR)
}

/// 泡泡被切成前景（點工作列按鈕／Alt+Tab）時是否該自動展開回 overlay。
///
/// 只認「這一輪才變成前景」；游標壓在泡泡上代表使用者正直接按它，交給點擊／拖曳處理，
/// 否則按下的瞬間就展開、泡泡再也拖不動。`bubble_hwnd` 為 0（還沒找到）時一律不觸發。
pub fn should_auto_expand(
    foreground: isize,
    previous: isize,
    bubble_hwnd: isize,
    pointer_over: bool,
) -> bool {
    bubble_hwnd != 0 && foreground == bubble_hwnd && previous != bubble_hwnd && !pointer_over
}

/// 未讀數徽章的文字：0 不顯示，超過 99 顯示 `99+`。
pub fn badge_text(count: u32) -> Option<String> {
    match count {
        0 => None,
        1..=99 => Some(count.to_string()),
        _ => Some("99+".into()),
    }
}

/// 徽章底圓的圓心與半徑（泡泡內 DIP 座標）；`text` 是數字的大小。
///
/// 單一數字也至少是正圓；位數多了往左右長成橫橢圓，碰到左／下緣就把圓心推回來不被裁掉。
pub fn badge_ellipse(text: Vec2) -> (Pos2, Vec2) {
    let size = BUBBLE_SIZE as f32;
    // 與 Python 的 round 相同（銀行家捨入），半格的差會讓數字與底圓不同心
    let round = f32::round_ties_even;
    let pad = round(size * 0.065).max(2.0);
    let half_h = round(text.y / 2.0 + pad);
    let half_w = round(text.x / 2.0 + pad).max(half_h);
    let cx = round(size * BADGE_HOME.0).max(half_w + 1.0);
    let cy = round(size * BADGE_HOME.1).min(size - half_h - 1.0);
    (Pos2::new(cx, cy), Vec2::new(half_w, half_h))
}

/// 泡泡的起始位置（實體像素）：存過就用（整個在畫面外則拉回），否則貼在 overlay 右上角。
pub fn initial_position(
    saved: (Option<i32>, Option<i32>),
    overlay: Rect,
    size: i32,
    monitors: &[Rect],
) -> (i32, i32) {
    let (ox, oy, ow, _) = overlay;
    let (Some(x), Some(y)) = saved else {
        return (ox + ow - size, oy);
    };
    let pulled = pull_onscreen(x, y, size, size, monitors);
    if pulled != (x, y) {
        log(format!(
            "[ui] bubble was offscreen, moved back: +{x}+{y} -> +{}+{}",
            pulled.0, pulled.1
        ));
    }
    pulled
}

/// 泡泡這一幀要 overlay 做的事。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// 點擊：展開回 overlay。
    Expand,
    /// 拖曳放開：新的左上角（實體像素）。
    Moved(i32, i32),
    /// Alt+F4／工作列關閉：走與 ✕ 相同的關閉路徑。
    Close,
}

/// 一次縮小期間的泡泡；展開時整個丟掉。
pub struct Bubble {
    /// 泡泡視窗的 HWND；視窗建好、找到之前是 0。
    hwnd: isize,
    /// 還沒套用到視窗的起始位置。
    pending: Option<(i32, i32)>,
    /// 按下時的游標與視窗矩形；`None`＝沒有對應的按下。
    press: Option<((i32, i32), Rect)>,
    prev_foreground: isize,
    next_poll: Instant,
    lookup_logged: bool,
    icon: Option<TextureHandle>,
}

impl Bubble {
    /// `position` 是起始左上角（實體像素），等視窗建好後套用。
    pub fn new(position: (i32, i32), now: Instant) -> Self {
        Self {
            hwnd: 0,
            pending: Some(position),
            press: None,
            prev_foreground: 0,
            next_poll: now + POLL_INTERVAL,
            lookup_logged: false,
            icon: None,
        }
    }

    /// 泡泡視窗的 HWND；還沒找到是 0。
    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    /// 還沒套用到視窗的起始位置（實體像素）。
    pub fn pending(&self) -> Option<(i32, i32)> {
        self.pending
    }

    /// 記下找到的 HWND（0＝還沒找到）；以當下的 `foreground` 為前景基準 —— 泡泡建立時可能已被啟用，
    /// 否則第一輪就會誤判成「使用者切回本工具」。
    pub fn attach(&mut self, hwnd: isize, foreground: isize) {
        self.hwnd = hwnd;
        self.prev_foreground = foreground;
        if hwnd == 0 && !self.lookup_logged {
            self.lookup_logged = true;
            log("[ui] bubble hwnd lookup failed: no visible bubble window on the UI thread");
        }
    }

    /// 輪詢時間到才檢查前景；泡泡這一輪才變成前景、游標又不在它上面就回 `true`。
    pub fn watch(&mut self, now: Instant, foreground: isize, shell: &dyn Shell) -> bool {
        if now < self.next_poll {
            return false;
        }
        self.next_poll = now + POLL_INTERVAL;
        let (px, py) = shell.cursor();
        let (x, y, w, h) = shell.rect();
        let pointer_over = point_in_rect(px, py, x, y, w, h);
        let expand = should_auto_expand(foreground, self.prev_foreground, self.hwnd, pointer_over);
        self.prev_foreground = foreground;
        if expand {
            log(format!(
                "[ui] auto-expand: bubble brought to foreground, fg=0x{foreground:x}"
            ));
        }
        expand
    }

    /// 畫出泡泡並處理按下／拖曳／放開與關閉要求；`alpha` 是泡泡自己的不透明度。
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        shell: &mut dyn Shell,
        unread: u32,
        alpha: f32,
    ) -> Option<Action> {
        if let Some((x, y)) = self.pending {
            let (_, _, w, h) = shell.rect();
            if shell.set_rect((x, y, w, h)) {
                self.pending = None;
            }
        }
        if ui.input(|i| i.viewport().close_requested()) {
            return Some(Action::Close);
        }
        let rect = ui.max_rect();
        let icon = self
            .icon
            .get_or_insert_with(|| load_icon(ui.ctx(), BUBBLE_SIZE as u32))
            .clone();
        Image::new((icon.id(), rect.size()))
            .tint(Color32::WHITE.gamma_multiply(alpha))
            .paint_at(ui, rect);
        if let Some(text) = badge_text(unread) {
            let painter = ui.painter();
            let galley = painter.layout_no_wrap(
                text,
                FontId::proportional(tk_pt(BADGE_FONT_PT)),
                BADGE_FG.gamma_multiply(alpha),
            );
            let (center, radius) = badge_ellipse(galley.size());
            let center = rect.min + center.to_vec2();
            painter.add(Shape::ellipse_filled(
                center,
                radius,
                BAR.gamma_multiply(alpha),
            ));
            painter.add(Shape::ellipse_stroke(
                center,
                radius,
                Stroke::new(1.0, GRIP.gamma_multiply(alpha)),
            ));
            painter.galley(center - galley.size() / 2.0, galley, BADGE_FG);
        }
        self.pointer(ui, rect, shell)
    }

    /// 移動不到 5px 的按下＋放開算點擊，否則跟著游標移動、放開時回報新位置。
    fn pointer(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        shell: &mut dyn Shell,
    ) -> Option<Action> {
        let response = ui.interact(rect, ui.scope_id().with("bubble"), Sense::click_and_drag());
        let primary_down = ui.input(|i| i.pointer.primary_down());
        if self.press.is_none() && primary_down && response.is_pointer_button_down_on() {
            self.press = Some((shell.cursor(), shell.rect()));
        }
        let ((sx, sy), (x, y, w, h)) = self.press?;
        let (cx, cy) = shell.cursor();
        let (dx, dy) = (cx - sx, cy - sy);
        if primary_down {
            let (nx, ny) = moved_to(x, y, dx, dy);
            if shell.rect() != (nx, ny, w, h) {
                shell.set_rect((nx, ny, w, h));
            }
            return None;
        }
        self.press = None;
        if is_click(dx, dy) {
            return Some(Action::Expand);
        }
        let (x, y, _, _) = shell.rect();
        Some(Action::Moved(x, y))
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;

    use super::*;
    use crate::ui::overlay::frame::tests::FakeShell;

    const OVERLAY: Rect = (100, 200, 640, 420);

    #[test]
    fn bubble_alpha_floor() {
        assert!((bubble_alpha(1.0) - 0.8).abs() < 1e-6);
        assert!((bubble_alpha(0.5) - 0.4).abs() < 1e-6);
        assert!((bubble_alpha(0.3) - 0.30).abs() < 1e-6);
    }

    #[test]
    fn auto_expand_only_on_becoming_foreground_without_pointer() {
        // 點工作列按鈕／Alt+Tab：這一輪才變前景、游標不在泡泡上
        assert!(should_auto_expand(101, 202, 101, false));
        // 直接按泡泡也會讓它變前景；交給點擊／拖曳，否則拖不動
        assert!(!should_auto_expand(101, 202, 101, true));
        // 已經是前景不算切換，不重複觸發
        assert!(!should_auto_expand(101, 101, 101, false));
        assert!(!should_auto_expand(303, 202, 101, false));
        // 還沒找到泡泡：不可與 GetForegroundWindow 的 0 誤判成相等
        assert!(!should_auto_expand(0, 202, 0, false));
    }

    #[test]
    fn watch_polls_every_300ms_from_the_attach_baseline() {
        let t0 = Instant::now();
        let shell = FakeShell {
            cursor: (0, 0),
            rect: (500, 500, 48, 48),
            ready: true,
        };
        let mut bubble = Bubble::new((500, 500), t0);
        // 建立時被啟用、找到的當下已經是前景：拿它當基準，不算切回
        bubble.attach(101, 101);
        assert!(!bubble.watch(t0 + POLL_INTERVAL, 101, &shell));
        assert!(!bubble.watch(t0 + POLL_INTERVAL * 2, 202, &shell));
        assert!(
            !bubble.watch(t0 + POLL_INTERVAL * 2, 101, &shell),
            "not yet"
        );
        assert!(bubble.watch(t0 + POLL_INTERVAL * 3, 101, &shell));
    }

    #[test]
    fn badge_caps_at_99_plus() {
        assert_eq!(badge_text(0), None);
        assert_eq!(badge_text(7).as_deref(), Some("7"));
        assert_eq!(badge_text(99).as_deref(), Some("99"));
        assert_eq!(badge_text(100).as_deref(), Some("99+"));
        assert_eq!(badge_text(12345).as_deref(), Some("99+"));
    }

    #[test]
    fn badge_is_round_and_stays_inside_the_bubble() {
        let (center, radius) = badge_ellipse(Vec2::new(4.0, 7.0));
        assert_eq!(radius, Vec2::splat(6.0), "single digit is a circle");
        assert_eq!(center, Pos2::new(9.0, 39.0));
        let (center, radius) = badge_ellipse(Vec2::new(20.0, 11.0));
        assert!(radius.x > radius.y);
        assert!(center.x - radius.x >= 1.0);
        assert!(center.y + radius.y <= BUBBLE_SIZE as f32 - 1.0);
    }

    struct State {
        bubble: Bubble,
        shell: FakeShell,
        actions: Vec<Action>,
    }

    fn harness() -> Harness<'static, State> {
        let state = State {
            bubble: Bubble::new((500, 300), Instant::now()),
            shell: FakeShell {
                cursor: (520, 320),
                rect: (0, 0, 48, 48),
                ready: true,
            },
            actions: Vec::new(),
        };
        let mut harness = Harness::builder()
            .with_size(Vec2::splat(BUBBLE_SIZE as f32))
            .build_ui_state(
                |ui, s: &mut State| {
                    if let Some(action) = s.bubble.show(ui, &mut s.shell, 3, 0.64) {
                        s.actions.push(action);
                    }
                },
                state,
            );
        harness.step();
        harness
    }

    /// 在泡泡中央按下、游標（螢幕實體像素）移動 `by` 後放開。
    fn press_move_release(harness: &mut Harness<State>, by: (i32, i32)) {
        let at = egui::pos2(24.0, 24.0);
        harness.hover_at(at);
        harness.drag_at(at);
        harness.step();
        let (x, y) = harness.state().shell.cursor;
        harness.state_mut().shell.cursor = (x + by.0, y + by.1);
        harness.step();
        harness.drop_at(at);
        harness.step();
    }

    #[test]
    fn pending_position_applies_once_the_window_exists() {
        let harness = harness();
        assert_eq!(harness.state().shell.rect, (500, 300, 48, 48));
    }

    #[test]
    fn click_vs_drag_threshold_5px() {
        let mut harness = harness();
        press_move_release(&mut harness, (4, -4));
        assert_eq!(harness.state().actions, [Action::Expand]);
        assert_eq!(harness.state().shell.rect, (504, 296, 48, 48));

        let mut harness = self::harness();
        press_move_release(&mut harness, (5, 0));
        assert_eq!(harness.state().actions, [Action::Moved(505, 300)]);
        assert_eq!(harness.state().shell.rect, (505, 300, 48, 48));
    }

    #[test]
    fn release_without_press_is_ignored() {
        // 點 ─ 縮小時放開的事件可能落到剛出現在游標下的泡泡上，沒有對應的按下就當作沒發生
        let mut harness = harness();
        harness.hover_at(egui::pos2(24.0, 24.0));
        harness.drop_at(egui::pos2(24.0, 24.0));
        harness.step();
        assert!(harness.state().actions.is_empty());
    }

    #[test]
    fn close_request_takes_the_shutdown_path() {
        let mut harness = harness();
        harness
            .input_mut()
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        harness.step();
        assert_eq!(harness.state().actions, [Action::Close]);
    }

    #[test]
    fn default_position_top_right_of_overlay() {
        let monitors = [(0, 0, 1920, 1080)];
        assert_eq!(
            initial_position((None, None), OVERLAY, 60, &monitors),
            (100 + 640 - 60, 200)
        );
        assert_eq!(
            initial_position((Some(30), Some(40)), OVERLAY, 60, &monitors),
            (30, 40)
        );
        // 存的位置整個在畫面外（舊版邏輯座標）就拉回最近的螢幕
        assert_eq!(
            initial_position((Some(5000), Some(40)), OVERLAY, 60, &monitors),
            (1920 - 60, 40)
        );
    }
}
