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

/// 畫面上文字 `text` 的各段文字與顏色；同樣的字畫了好幾次時取最後畫的（彈出層在最上層）。找不到就 panic。
pub fn text_sections<S>(h: &Harness<S>, text: &str) -> Vec<(String, eframe::egui::Color32)> {
    h.output()
        .shapes
        .iter()
        .rev()
        .find_map(|c| match &c.shape {
            Shape::Text(t) if t.galley.text() == text => Some(
                t.galley
                    .job
                    .sections
                    .iter()
                    .map(|s| {
                        let range = s.byte_range.start.0..s.byte_range.end.0;
                        (text[range].to_owned(), s.format.color)
                    })
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{text:?} not painted"))
}

/// 跑過服務表單開啟時載入佔位的時間上限（每步 0.25 秒），之後才能操作剛打開的表單。
pub fn settle<S>(h: &mut Harness<S>) {
    h.run_steps(8);
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
    click_nth(h, &tr("button.change"), index);
}

/// 點由上往下數第 `index` 顆名稱是 `label` 的按鈕。
pub fn click_nth<S>(h: &mut Harness<S>, label: &str, index: usize) {
    let mut nodes: Vec<_> = h.query_all_by_role_and_label(Role::Button, label).collect();
    nodes.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    nodes[index].scroll_to_me();
    nodes[index].click();
    run(h);
}

/// 點對話框裡的按鈕（背後的卡片也有同名按鈕）。
pub fn click_in_dialog<S>(h: &mut Harness<S>, title: &str, button: &str) {
    h.get_by_role_and_label(Role::Dialog, title)
        .get_by_role_and_label(Role::Button, button)
        .click();
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
