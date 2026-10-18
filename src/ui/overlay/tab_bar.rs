//! overlay 標題列下方的分頁列：照抄遊戲聊天分頁按鈕的文字，高亮目前分頁，不可點擊。

use eframe::egui::{self, FontId, Label, RichText, TextWrapMode, Ui, Vec2};

use crate::ui::fonts_egui::tk_pt;
use crate::ui::palette::{TAB_ACCENT, TAB_BG, TAB_FG, TAB_FG_ACTIVE};

const ACCENT_HEIGHT: f32 = 2.0;
const CELL_GAP: f32 = 10.0;
const TEXT_PT: f32 = 9.0;
const PAD_TOP: i8 = 3;

/// 畫出分頁列；沒有分頁時什麼都不畫（整列隱藏）。文字是遊戲內容，不經 i18n。
pub fn show(ui: &mut Ui, tabs: &[(i32, String)], current: Option<i32>) {
    if tabs.is_empty() {
        return;
    }
    egui::Frame::new()
        .fill(TAB_BG)
        .inner_margin(egui::Margin {
            top: PAD_TOP,
            ..Default::default()
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for (index, text) in tabs {
                    ui.add_space(CELL_GAP);
                    let active = Some(*index) == current;
                    let color = if active { TAB_FG_ACTIVE } else { TAB_FG };
                    ui.vertical(|ui| {
                        let label = RichText::new(text)
                            .font(FontId::proportional(tk_pt(TEXT_PT)))
                            .color(color);
                        // 窄 overlay 也不換行：換行會把整列撐高、擠掉訊息清單
                        let label = Label::new(label)
                            .selectable(false)
                            .wrap_mode(TextWrapMode::Extend);
                        let width = ui.add(label).rect.width();
                        let (_, line) = ui.allocate_space(Vec2::new(width, ACCENT_HEIGHT));
                        if active {
                            ui.painter().rect_filled(line, 0.0, TAB_ACCENT);
                        }
                    });
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::ui::fonts_egui;

    fn harness(tabs: Vec<(i32, String)>, current: Option<i32>) -> Harness<'static> {
        harness_of_width(300.0, tabs, current)
    }

    fn harness_of_width(
        width: f32,
        tabs: Vec<(i32, String)>,
        current: Option<i32>,
    ) -> Harness<'static> {
        let mut harness = Harness::builder()
            .with_size(Vec2::new(width, 60.0))
            .with_context_setup(|ctx| fonts_egui::install(ctx, "en-US"))
            .build_ui(move |ui| show(ui, &tabs, current));
        harness.step();
        harness
    }

    fn tabs() -> Vec<(i32, String)> {
        vec![
            (0, "Main".into()),
            (1, "Group".into()),
            (2, "Room(1)".into()),
        ]
    }

    #[test]
    fn shows_every_game_label() {
        let bar = harness(tabs(), Some(1));
        for label in ["Main", "Group", "Room(1)"] {
            bar.get_by_label(label);
        }
    }

    #[test]
    fn labels_stay_on_one_line_in_a_narrow_overlay() {
        let long = vec![
            (0, "Main Channel".to_owned()),
            (1, "Group Channel".to_owned()),
        ];
        let wide = harness_of_width(400.0, long.clone(), Some(0));
        let narrow = harness_of_width(40.0, long, Some(0));
        for label in ["Main Channel", "Group Channel"] {
            assert_eq!(
                narrow.get_by_label(label).rect().height(),
                wide.get_by_label(label).rect().height(),
                "{label}"
            );
        }
    }

    #[test]
    fn hidden_without_tabs() {
        let harness = harness(Vec::new(), None);
        assert!(harness.query_by_label("Main").is_none());
    }

    fn accent_rects(harness: &Harness<'_>) -> Vec<egui::Rect> {
        harness
            .output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.fill == TAB_ACCENT => Some(rect.rect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn only_the_current_tab_gets_a_two_pixel_accent_underline() {
        let bar = harness(tabs(), Some(1));
        let group = bar.get_by_label("Group").rect();
        let accents = accent_rects(&bar);
        assert_eq!(accents.len(), 1);
        assert_eq!(accents[0].height(), ACCENT_HEIGHT);
        assert!((accents[0].center().x - group.center().x).abs() < 1.0);
        assert!(accents[0].min.y >= group.max.y - 1.0);
        assert!(accent_rects(&harness(tabs(), None)).is_empty());
    }

    /// 以 label 字串找出畫出來的文字顏色。
    fn text_color(harness: &Harness<'_>, label: &str) -> egui::Color32 {
        fn find(shape: &egui::Shape, label: &str) -> Option<egui::Color32> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.galley.job.sections[0].format.color)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
                _ => None,
            }
        }
        harness
            .output()
            .shapes
            .iter()
            .find_map(|clipped| find(&clipped.shape, label))
            .unwrap_or_else(|| panic!("no text shape for {label}"))
    }

    #[test]
    fn current_tab_text_is_bright_and_the_others_are_dim() {
        let bar = harness(tabs(), Some(1));
        assert_eq!(text_color(&bar, "Group"), TAB_FG_ACTIVE);
        assert_eq!(text_color(&bar, "Main"), TAB_FG);
        assert_eq!(text_color(&bar, "Room(1)"), TAB_FG);
        let none = harness(tabs(), None);
        assert_eq!(text_color(&none, "Group"), TAB_FG);
    }
}
