//! 只有「複製」一項的右鍵選單：浮在視窗最上層、夾在視窗內，overlay 與框選結果卡共用。

use std::sync::Arc;

use eframe::egui::{
    Area, Color32, FontId, Galley, Id, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
};

use crate::i18n::t;
use crate::logs::log;
use crate::ui::fonts_egui::tk_pt;
use crate::ui::geometry::clamped_position;
use crate::ui::palette::{BAR, FG_BAR, FG_TRANSLATED, GRIP, SELECT_BG, TAB_ACCENT};

const PAD_X: f32 = 10.0;
const PAD_Y: f32 = 5.0;
const GAP: f32 = 6.0;
const ICON: f32 = 13.0;
const SHEET_OFFSET: f32 = 4.0;

/// 選單這一幀的結果。
enum MenuOutcome {
    /// 點了「複製」。
    Copy,
    /// 在選單外按下滑鼠左鍵。
    Dismissed,
    /// 仍開著；帶著這一幀的選單範圍。
    Open(Rect),
}

fn label(ui: &Ui) -> Arc<Galley> {
    ui.painter().layout_no_wrap(
        t("menu.copy", &[]),
        FontId::proportional(tk_pt(9.0)),
        Color32::PLACEHOLDER,
    )
}

/// 回傳選單範圍與它相對 `ui` 範圍左上角的位置（供 log）。
fn placed(ui: &Ui, at: Pos2, galley: &Galley) -> (Rect, (i32, i32)) {
    let inner = Vec2::new(
        PAD_X + ICON + GAP + galley.size().x + PAD_X,
        PAD_Y + ICON.max(galley.size().y) + PAD_Y,
    );
    let size = inner + Vec2::splat(2.0);
    let area = ui.max_rect();
    let (x, y) = clamped_position(
        (at.x - area.min.x) as i32,
        (at.y - area.min.y) as i32,
        size.x.ceil() as i32,
        size.y.ceil() as i32,
        area.width() as i32,
        area.height() as i32,
    );
    let min = area.min + Vec2::new(x as f32, y as f32);
    (Rect::from_min_size(min, size), (x, y))
}

/// 在 `at`（視窗內 DIP）開啟時的選單範圍，已夾在 `ui` 的範圍內；給還沒畫出選單的那一幀判斷點擊用。
fn rect(ui: &Ui, at: Pos2) -> Rect {
    placed(ui, at, &label(ui)).0
}

/// 在 `at` 畫出選單並回報這一幀的結果；`ui` 要是整個視窗的 Ui。
///
/// 上一幀沒畫過、或換了位置才記「shown」log；「hidden」由 `CopyMenu::close` 記。
fn draw(ui: &Ui, id: Id, at: Pos2) -> MenuOutcome {
    let galley = label(ui);
    let (menu, (x, y)) = placed(ui, at, &galley);
    let pass = ui.ctx().cumulative_pass_nr();
    let last = ui.data_mut(|d| {
        let last = d.get_temp::<(Pos2, u64)>(id.with("shown"));
        d.insert_temp(id.with("shown"), (at, pass));
        last
    });
    let fresh = !matches!(last, Some((p, n)) if p == at && n + 1 >= pass);
    if fresh {
        log(format!("[ui] copy popup shown at {x},{y}"));
    }
    let pressed = ui.input(|i| {
        let p = &i.pointer;
        p.primary_pressed().then(|| p.press_origin()).flatten()
    });
    if pressed.is_some_and(|p| !menu.contains(p)) {
        return MenuOutcome::Dismissed;
    }
    let clicked = Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(menu.min)
        .show(ui.ctx(), |ui| {
            let (rect, response) = ui.allocate_exact_size(menu.size(), Sense::click());
            // hover 要一眼看得出來：整列換成反白藍、外框換強調色
            let (border, bg, fg) = if response.hovered() {
                (TAB_ACCENT, SELECT_BG, FG_TRANSLATED)
            } else {
                (GRIP, BAR, FG_BAR)
            };
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, border);
            let row = rect.shrink(1.0);
            painter.rect_filled(row, 0.0, bg);
            // 「兩張疊起來的紙」：後面那張只露出左上角
            let icon = Pos2::new(row.min.x + PAD_X, row.center().y - ICON / 2.0);
            let back = Rect::from_min_size(icon, Vec2::splat(ICON - SHEET_OFFSET));
            let front = back.translate(Vec2::splat(SHEET_OFFSET));
            painter.rect_stroke(back, 0.0, Stroke::new(1.0, fg), StrokeKind::Inside);
            painter.rect_filled(front, 0.0, bg);
            painter.rect_stroke(front, 0.0, Stroke::new(1.0, fg), StrokeKind::Inside);
            let text = Pos2::new(icon.x + ICON + GAP, row.center().y - galley.size().y / 2.0);
            painter.galley(text, galley, fg);
            response.clicked()
        })
        .inner;
    if clicked {
        MenuOutcome::Copy
    } else {
        MenuOutcome::Open(menu)
    }
}

/// 右鍵「複製」選單的開關狀態：位置與範圍；overlay 與框選結果卡各持一份。
#[derive(Default)]
pub struct CopyMenu {
    at: Option<Pos2>,
    rect: Option<Rect>,
}

impl CopyMenu {
    /// 在 `at`（視窗內 DIP）開啟；範圍先算好，下一幀的點擊比選單先處理，不能等 `show` 回報。
    pub fn open(&mut self, ui: &Ui, at: Pos2) {
        self.at = Some(at);
        self.rect = Some(rect(ui, at));
    }

    /// 收掉選單；原本開著才記 log。
    pub fn close(&mut self) {
        self.rect = None;
        if self.at.take().is_some() {
            log("[ui] copy popup hidden");
        }
    }

    /// `p` 是否落在選單上（選單上的按放不算點到底下的內容）。
    pub fn contains(&self, p: Pos2) -> bool {
        self.rect.is_some_and(|r| r.contains(p))
    }

    /// 選單目前的範圍；沒開是 `None`。
    #[cfg(test)]
    pub fn rect(&self) -> Option<Rect> {
        self.rect
    }

    /// 開著就畫出來；點了「複製」或在選單外按下都會收掉，只有點「複製」回 `true`。
    pub fn show(&mut self, ui: &Ui, id: Id) -> bool {
        let Some(at) = self.at else {
            return false;
        };
        match draw(ui, id, at) {
            MenuOutcome::Open(rect) => self.rect = Some(rect),
            MenuOutcome::Copy => {
                self.close();
                return true;
            }
            MenuOutcome::Dismissed => self.close(),
        }
        false
    }
}
