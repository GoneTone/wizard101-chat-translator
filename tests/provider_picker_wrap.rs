//! 服務商卡片在每個介面語言下，標題與說明都排在卡片內、格狀同列等高。
//!
//! 會切換全域介面語言，獨立成一個測試執行檔，檔內以鎖排隊。

mod common;

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Rect, Shape, Ui};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use common::kittest::{themed_harness, tr};
use common::language_lock;
use wizard101_chat_translator::config::services::providers;
use wizard101_chat_translator::i18n::{self, available_languages};
use wizard101_chat_translator::ui::settings::provider_picker::{CardLayout, provider_cards};
use wizard101_chat_translator::ui::settings::theme;

/// 選擇對話框兩側內距各 20。
const PICKER_MARGIN: f32 = 20.0;
/// 精靈內容區左右內距各 36；精靈最小寬 600。
const WIZARD_BODY_MARGIN: f32 = 36.0;

fn cards_harness(layout: CardLayout, width: f32, language: &'static str) -> Harness<'static, ()> {
    i18n::set_language(language);
    themed_harness(
        egui::vec2(width + 40.0, 2000.0),
        language,
        move |ui: &mut Ui, _: &mut ()| {
            ui.scope(|ui| {
                ui.set_width(width);
                provider_cards(ui, layout);
            });
        },
        (),
    )
}

/// 畫面上文字恰為 `text` 的字形範圍。
fn galley_rects(h: &Harness<()>, text: &str) -> Vec<Rect> {
    h.output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Text(shape) if shape.galley.text() == text => {
                Some(shape.galley.rect.translate(shape.pos.to_vec2()))
            }
            _ => None,
        })
        .collect()
}

fn assert_text_inside_cards(layout: CardLayout, width: f32, language: &'static str) {
    let h = cards_harness(layout, width, language);
    let mut rects = Vec::new();
    for provider in providers() {
        let card = h
            .get_by_role_and_label(Role::Button, &tr(provider.label_key))
            .rect();
        for key in [provider.label_key, provider.desc_key] {
            let galleys = galley_rects(&h, &tr(key));
            assert_eq!(galleys.len(), 1, "{language} {layout:?} {key}");
            assert!(
                card.contains_rect(galleys[0]),
                "{language} {layout:?} w={width} {key}: text {:?} outside card {card:?}",
                galleys[0]
            );
        }
        rects.push(card);
    }
    if layout == CardLayout::Grid {
        for row in rects.chunks(2) {
            assert!(
                row.windows(2).all(|p| p[0].height() == p[1].height()),
                "{language} grid row heights differ: {row:?}"
            );
        }
    }
}

#[test]
fn provider_card_text_stays_inside_the_card_in_every_language() {
    let _lock = language_lock();
    for (code, _) in available_languages() {
        let code: &'static str = Box::leak(code.into_boxed_str());
        assert_text_inside_cards(
            CardLayout::List,
            theme::PICKER_DIALOG_WIDTH - 2.0 * PICKER_MARGIN,
            code,
        );
        assert_text_inside_cards(
            CardLayout::Grid,
            theme::WIZARD_MIN_SIZE.x - 2.0 * WIZARD_BODY_MARGIN,
            code,
        );
    }
}
