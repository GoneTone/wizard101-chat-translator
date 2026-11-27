//! overlay 等待遊戲時內容區中央的「啟動遊戲」大按鈕。

use eframe::egui::{Align2, Area, Id, Order, Pos2, Rect, Ui};

use crate::i18n::t;
use crate::ui::overlay::frame::GRIP_SIZE;
use crate::ui::settings::theme::{self, icon};
use crate::ui::settings::widgets::primary_leading_icon_button;

/// 按鈕與上方狀態大字的距離。
const BUTTON_GAP: f32 = 8.0;

/// 畫出掛在 `at` 下方的中央按鈕，不超出 `body`（標題列以下的內容區）、底部讓出抓取紋；回傳這一幀是否按下。
pub fn center_button(ui: &Ui, at: Pos2, body: Rect) -> bool {
    let top = (at.y + BUTTON_GAP)
        .min(body.max.y - GRIP_SIZE - theme::CONTROL_HEIGHT)
        .max(body.min.y);
    Area::new(Id::unique("launch_button"))
        .order(Order::Middle)
        .constrain(false)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(Pos2::new(at.x, top))
        .show(ui.ctx(), |ui| {
            theme::apply(ui.style_mut());
            primary_leading_icon_button(ui, icon::PLAY, &t("launch.open_game", &[]))
        })
        .inner
        .clicked()
}
