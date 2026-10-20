//! egui_kittest 測試共用的小工具：查詢、點擊、讀畫面輸出，以及全域介面語言的排隊鎖。
#![allow(dead_code)]

use eframe::egui::accesskit::Role;
use eframe::egui::{Event, Modifiers, PointerButton, Pos2, Shape};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use wizard101_chat_translator::i18n::t;

pub fn tr(key: &str) -> String {
    t(key, &[])
}

pub fn run<S>(h: &mut Harness<S>) {
    h.run_steps(3);
}

/// 先捲到元件再點：小視窗裡卡片可能在捲動區外。
pub fn click<S>(h: &mut Harness<S>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).scroll_to_me();
    run(h);
    h.get_by_role_and_label(role, label).click();
    run(h);
}

/// 在 `pos` 按下再放開滑鼠左鍵。
pub fn click_at<S>(h: &mut Harness<S>, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    run(h);
}

pub fn has_label<S>(h: &Harness<S>, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some()
}

/// 畫面上所有文字字形。
pub fn painted_texts<S>(h: &Harness<S>) -> Vec<String> {
    h.output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Text(shape) => Some(shape.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}
