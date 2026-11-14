//! 單一 release 的卡片：版本列加 Markdown 內文，更新視窗與目前版本說明視窗共用。

use eframe::egui::{Align, FontId, Frame, Label, Layout, Margin, RichText, TextStyle, Ui, vec2};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use super::release_links::linkify_github;
use super::release_table::{self, Block, split_tables};
use super::theme;
use super::widgets::{card, hint, pill, skeleton};
use crate::i18n::t;
use crate::runtime::updater::Release;

/// 內文標題字級（粗體）：比版本號小，略大於內文。
const MARKDOWN_HEADING_SIZE: f32 = 15.0;
/// 載入中佔位條：標題列寬度（px）。
const SKELETON_TITLE_WIDTH: f32 = 120.0;
/// 載入中佔位條：日期寬度（px）。
const SKELETON_DATE_WIDTH: f32 = 96.0;
/// 載入中佔位條：兩段內文之間的間距（px）。
const SKELETON_SECTION_GAP: f32 = 8.0;
/// 載入中佔位條：每段內文各列佔可用寬度的比例。
const SKELETON_SECTIONS: [&[f32]; 2] = [&[1.0, 0.92, 0.64], &[0.96, 0.58]];
/// 標題區內距：字型行高在字形上方多留約 5 px、下方約 2 px，上方少給，字形上下到邊界看起來才一樣寬。
pub(super) const HEADER_MARGIN: Margin = Margin {
    left: 20,
    right: 20,
    top: 13,
    bottom: 16,
};
/// 卡片清單頭尾的留白：第一張卡片離標題區的分隔線、最後一張卡片離底部列的距離。
pub(super) const LIST_PADDING: i8 = 16;
/// 底部列內距。
pub(super) const FOOTER_MARGIN: Margin = Margin::symmetric(20, 14);

/// 畫一張 release 卡片；`latest` 決定要不要畫「最新」標籤。
pub(super) fn show(ui: &mut Ui, cache: &mut CommonMarkCache, release: &Release, latest: bool) {
    let meta = |ui: &mut Ui| {
        let date = t("update.published", &[("date", &local_date(release))]);
        hint(ui, &date, theme::TEXT_MUTED);
        if latest {
            pill(ui, &t("update.latest_tag", &[]));
        }
    };
    shell(ui, &release.version, meta, |ui| {
        ui.style_mut()
            .text_styles
            .insert(TextStyle::Heading, theme::bold(MARKDOWN_HEADING_SIZE));
        // 主題的 override_text_color 會把連結也染成內文色
        let visuals = ui.visuals_mut();
        visuals.override_text_color = None;
        visuals.widgets.noninteractive.fg_stroke.color = theme::TEXT;
        // 清單項目用 horizontal_wrapped，首列高取 interact_size.y：控制項高會讓連結與項目符號浮在內文上方
        ui.spacing_mut().interact_size.y = ui.text_style_height(&TextStyle::Body);
        let body = linkify_github(&flatten_headings(&release.body));
        for block in split_tables(&body) {
            match block {
                Block::Markdown(text) => {
                    CommonMarkViewer::new().show(ui, cache, text);
                }
                Block::Table(table) => release_table::show(ui, &table, cache),
            }
        }
    });
}

/// 載入中的卡片：日期與內文都用佔位條。
pub(super) fn loading(ui: &mut Ui, version: &str) {
    let date_height = ui.fonts_mut(|f| f.row_height(&FontId::proportional(theme::HINT_SIZE)));
    let meta = |ui: &mut Ui| {
        skeleton(ui, SKELETON_DATE_WIDTH, date_height);
    };
    shell(ui, version, meta, |ui| {
        let line = ui.text_style_height(&TextStyle::Body);
        let width = ui.available_width();
        for (i, lines) in SKELETON_SECTIONS.iter().enumerate() {
            if i > 0 {
                ui.add_space(SKELETON_SECTION_GAP);
            }
            skeleton(ui, SKELETON_TITLE_WIDTH, line);
            for share in *lines {
                skeleton(ui, width * share, line);
            }
        }
    });
}

/// 卡片外框與版本列（版本號加 `meta`），分隔線下交給 `body`。
pub(super) fn shell(
    ui: &mut Ui,
    version: &str,
    meta: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui),
) {
    card(ui, |ui| {
        Frame::new()
            .inner_margin(Margin::symmetric(16, 14))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                let font = theme::bold(17.0);
                // 列高取版本號字高：用預設的控制項高時「最新」標籤會被撐高
                let height = ui.fonts_mut(|f| f.row_height(&font));
                let row = vec2(ui.available_width(), height);
                ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let version = RichText::new(format!("v{version}"))
                        .font(font)
                        .color(theme::ACCENT);
                    ui.add(Label::new(version));
                    meta(ui);
                });
                ui.separator();
                body(ui);
            });
    });
}

/// 各級 ATX 標題都改成一級：egui_commonmark 只有一級標題用 `TextStyle::Heading`（粗體），其餘只放大字級。
fn flatten_headings(body: &str) -> String {
    let mut fenced = false;
    body.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                fenced = !fenced;
            }
            let hashes = trimmed.len() - trimmed.trim_start_matches('#').len();
            let rest = &trimmed[hashes..];
            // 縮排四格以上是程式碼區塊；`#` 超過六個或後面沒接空白就不是標題
            let heading = line.len() - trimmed.len() < 4
                && (2..=6).contains(&hashes)
                && (rest.is_empty() || rest.starts_with([' ', '\t']));
            if heading && !fenced {
                format!("#{rest}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 發布日期換成本機時區的 `YYYY-MM-DD`；解析不了就原樣顯示。
fn local_date(release: &Release) -> String {
    chrono::DateTime::parse_from_rfc3339(&release.published_at).map_or_else(
        |_| release.published_at.clone(),
        |at| {
            at.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string()
        },
    )
}
