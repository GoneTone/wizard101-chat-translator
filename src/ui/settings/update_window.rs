//! 更新內容視窗的畫面：由新到舊的 release 卡片（Markdown 內文），底部列依下載階段變化。

use std::collections::BTreeMap;
use std::time::Instant;

use eframe::egui::{Align, Frame, Id, Label, Layout, Margin, Panel, RichText, TextStyle, Ui, vec2};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use super::local_rate::{MB, Rates, Speed, format_speed};
use super::quant_table::fraction;
use super::release_links::linkify_github;
use super::release_table::{self, Block, split_tables};
use super::theme;
use super::widgets::{
    card, danger_button, hint, pill, primary_button, progress_bar, right_aligned,
    right_aligned_width, scroll_page, secondary_button,
};
use crate::i18n::t;
use crate::runtime::update_job::Phase;
use crate::runtime::updater::Release;

/// 內文標題字級（粗體）：比版本號小，略大於內文。
const MARKDOWN_HEADING_SIZE: f32 = 15.0;
/// 更新下載在 `Rates` 裡的鍵。
const RATE_KEY: &str = "update";
/// 標題區內距：字型行高在字形上方多留約 5 px、下方約 2 px，上方少給，字形上下到邊界看起來才一樣寬。
const HEADER_MARGIN: Margin = Margin {
    left: 20,
    right: 20,
    top: 13,
    bottom: 16,
};
/// 卡片清單頭尾的留白：第一張卡片離標題區的分隔線、最後一張卡片離底部列的距離。
const LIST_PADDING: i8 = 16;

/// 使用者在更新視窗按下的動作，由 host 轉給 `UpdateJob`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    Start,
    Cancel,
    Retry,
    OpenReleasePage,
    Close,
    Restart,
}

/// 更新視窗的畫面狀態：Markdown 快取與下載速度。
#[derive(Default)]
pub struct UpdateView {
    markdown: CommonMarkCache,
    rates: Rates,
}

/// 視窗能不能關：下載中（含驗證）與重新啟動中不能。
pub fn can_close(phase: &Phase) -> bool {
    !matches!(phase, Phase::Downloading | Phase::Restarting)
}

impl UpdateView {
    pub fn new() -> Self {
        Self::default()
    }

    /// 畫一幀；`unsaved` 是設定視窗有沒有沒儲存的變更（決定 ⑧-1／⑧-2）。
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        releases: &[Release],
        phase: &Phase,
        progress: (u64, u64),
        unsaved: bool,
    ) -> Vec<UpdateAction> {
        let running = match phase {
            Phase::Downloading => BTreeMap::from([(RATE_KEY.to_owned(), progress.0)]),
            _ => BTreeMap::new(),
        };
        self.rates.observe_running(Instant::now(), running);
        let speed = self.rates.speed(RATE_KEY);
        let mut actions = Vec::new();
        Panel::bottom("update_footer")
            .frame(
                Frame::new()
                    .fill(theme::MODAL_FOOT)
                    .inner_margin(Margin::symmetric(20, 14)),
            )
            .show(ui, |ui| {
                footer(ui, phase, progress, speed, unsaved, &mut actions);
            });
        if let Some(latest) = releases.first() {
            Panel::top("update_header")
                .frame(Frame::new().inner_margin(HEADER_MARGIN))
                .show(ui, |ui| header(ui, latest, releases.len()));
        }
        let margin = Margin::symmetric(20, LIST_PADDING);
        scroll_page(ui, "update_releases", margin, theme::CARD_GAP, |ui| {
            self.releases(ui, releases);
        });
        actions
    }

    /// 每一版的卡片。
    fn releases(&mut self, ui: &mut Ui, releases: &[Release]) {
        for (i, release) in releases.iter().enumerate() {
            ui.push_id(i, |ui| self.release_card(ui, release, i == 0));
        }
    }

    fn release_card(&mut self, ui: &mut Ui, release: &Release, latest: bool) {
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
                        let version = RichText::new(format!("v{}", release.version))
                            .font(font)
                            .color(theme::ACCENT);
                        ui.add(Label::new(version));
                        let date = t("update.published", &[("date", &local_date(release))]);
                        hint(ui, &date, theme::TEXT_MUTED);
                        if latest {
                            pill(ui, &t("update.latest_tag", &[]));
                        }
                    });
                    ui.separator();
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
                                CommonMarkViewer::new().show(ui, &mut self.markdown, text);
                            }
                            Block::Table(table) => {
                                release_table::show(ui, &table, &mut self.markdown)
                            }
                        }
                    }
                });
        });
    }
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

/// 固定在上方的標題與副標：最新版號、目前版號與新版本數。
fn header(ui: &mut Ui, latest: &Release, count: usize) {
    ui.spacing_mut().item_spacing.y = 4.0;
    let heading = t("update.heading", &[("version", &latest.version)]);
    ui.add(Label::new(RichText::new(heading).font(theme::bold(20.0))));
    let count = count.to_string();
    let sub = t(
        "update.subheading",
        &[("current", env!("CARGO_PKG_VERSION")), ("count", &count)],
    );
    hint(ui, &sub, theme::TEXT_SECONDARY);
}

/// 底部列：左側狀態（進度條、文字），右側按鈕。
fn footer(
    ui: &mut Ui,
    phase: &Phase,
    (done, total): (u64, u64),
    speed: Option<Speed>,
    unsaved: bool,
    actions: &mut Vec<UpdateAction>,
) {
    let verifying = matches!(phase, Phase::Downloading) && total > 0 && done == total;
    let mut press = |clicked: bool, action| {
        if clicked {
            actions.push(action);
        }
    };
    let buttons = Id::unique("update_buttons");
    let status = Id::unique("update_status");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let room = (ui.available_width() - right_aligned_width(ui, buttons) - 16.0).max(120.0);
        // 零高區塊會被擺在列中線下方，所以用上一幀量到的內容高度預留空間，再把內容置中。
        let height = ui.data(|d| d.get_temp(status)).unwrap_or(0.0_f32);
        let column = Layout::top_down(Align::Min).with_main_align(Align::Center);
        let shown = ui.allocate_ui_with_layout(vec2(room, height), column, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.set_width(room);
            match phase {
                Phase::Idle => {}
                Phase::Downloading if !verifying => {
                    progress_bar(ui, fraction(done, total), room);
                    hint(
                        ui,
                        &transfer_text(done, total, speed),
                        theme::TEXT_SECONDARY,
                    );
                }
                Phase::Downloading => {
                    progress_bar(ui, 1.0, room);
                    hint(ui, &t("update.verifying", &[]), theme::TEXT_SECONDARY);
                }
                Phase::Restarting => {
                    progress_bar(ui, 1.0, room);
                    hint(ui, &t("update.restarting", &[]), theme::OK);
                }
                Phase::Failed { error, .. } => {
                    let text = t("update.failed_download", &[("error", error)]);
                    hint(ui, &text, theme::ERROR);
                }
                Phase::Cancelled => hint(ui, &t("update.cancelled", &[]), theme::TEXT_SECONDARY),
                Phase::Ready if unsaved => hint(ui, &t("update.unsaved", &[]), theme::WARN_TEXT),
                Phase::Ready => hint(ui, &t("update.ready", &[]), theme::OK),
            }
        });
        let used = shown.response.rect.height();
        if (used - height).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(status, used));
            ui.ctx().request_discard("footer status resized");
        }
        right_aligned(ui, buttons, |ui| {
            let label = |key| t(key, &[]);
            match phase {
                Phase::Idle => {
                    press(
                        secondary_button(ui, &label("update.later")).clicked(),
                        UpdateAction::Close,
                    );
                    press(
                        primary_button(ui, &label("update.now")).clicked(),
                        UpdateAction::Start,
                    );
                }
                Phase::Downloading | Phase::Restarting => {
                    let cancellable = matches!(phase, Phase::Downloading) && !verifying;
                    let cancel = ui
                        .add_enabled_ui(cancellable, |ui| {
                            secondary_button(ui, &label("update.cancel"))
                        })
                        .inner;
                    press(cancel.clicked(), UpdateAction::Cancel);
                }
                Phase::Failed { .. } => {
                    press(
                        secondary_button(ui, &label("update.open_github")).clicked(),
                        UpdateAction::OpenReleasePage,
                    );
                    press(
                        primary_button(ui, &label("update.retry")).clicked(),
                        UpdateAction::Retry,
                    );
                }
                Phase::Cancelled => {
                    press(
                        secondary_button(ui, &label("update.close")).clicked(),
                        UpdateAction::Close,
                    );
                    press(
                        primary_button(ui, &label("update.redownload")).clicked(),
                        UpdateAction::Start,
                    );
                }
                Phase::Ready => {
                    press(
                        secondary_button(ui, &label("update.later")).clicked(),
                        UpdateAction::Close,
                    );
                    let restart = if unsaved {
                        danger_button(ui, &label("update.discard_restart"))
                    } else {
                        primary_button(ui, &label("update.restart"))
                    };
                    press(restart.clicked(), UpdateAction::Restart);
                }
            }
        });
    });
}

/// 「下載中 X / Y MB」，有速度時接上速度；有速度時已下載量取速度換上當下的值，數字不會一直跳。
fn transfer_text(done: u64, total: u64, speed: Option<Speed>) -> String {
    let mb = |bytes: u64| format!("{:.1}", bytes as f64 / MB);
    let done = speed.map_or(done, |s| s.done);
    let mut line = t(
        "update.downloading",
        &[("done", &mb(done)), ("total", &mb(total))],
    );
    if let Some(s) = speed {
        line += &format!(" · {}", format_speed(s.rate));
    }
    line
}
