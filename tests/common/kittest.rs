//! egui_kittest 測試共用的小工具：查詢、點擊、讀畫面輸出，以及全域介面語言的排隊鎖。
#![allow(dead_code)]

use eframe::egui::accesskit::Role;
use eframe::egui::{Event, Modifiers, PointerButton, Pos2, Shape, Ui, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, HarnessBuilder};
use wizard101_chat_translator::i18n::t;
use wizard101_chat_translator::ui::fonts_egui;
use wizard101_chat_translator::ui::settings::theme;

pub fn tr(key: &str) -> String {
    t(key, &[])
}

pub fn run<S>(h: &mut Harness<S>) {
    h.run_steps(3);
}

/// 套設定主題、載入 `language` 的字型並跑三輪的 harness；樣式只套在根 Ui 上。
pub fn themed_harness<S: 'static>(
    size: Vec2,
    language: &'static str,
    ui: impl FnMut(&mut Ui, &mut S) + 'static,
    state: S,
) -> Harness<'static, S> {
    themed_harness_with(Harness::builder(), size, language, ui, state)
}

/// 同 [`themed_harness`]，builder 由呼叫端給（例如要 `.wgpu()` 離屏渲染）。
pub fn themed_harness_with<S: 'static>(
    builder: HarnessBuilder<S>,
    size: Vec2,
    language: &'static str,
    mut ui: impl FnMut(&mut Ui, &mut S) + 'static,
    state: S,
) -> Harness<'static, S> {
    let mut h = builder
        .with_size(size)
        .with_context_setup(move |ctx| fonts_egui::install(ctx, language))
        .build_ui_state(
            move |root, s| {
                theme::apply(root.style_mut());
                ui(root, s);
            },
            state,
        );
    run(&mut h);
    h
}

/// 先捲到元件再點：小視窗裡卡片可能在捲動區外。
pub fn click<S>(h: &mut Harness<S>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).scroll_to_me();
    run(h);
    h.get_by_role_and_label(role, label).click();
    run(h);
}

/// 點由上往下數第 `index` 顆「變更」按鈕（錄熱鍵用；wizard 中 0 是輸入框熱鍵、1 是框選熱鍵）。
pub fn click_change<S>(h: &mut Harness<S>, index: usize) {
    let label = tr("button.change");
    let mut nodes: Vec<_> = h
        .query_all_by_role_and_label(Role::Button, &label)
        .collect();
    nodes.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    nodes[index].scroll_to_me();
    nodes[index].click();
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
