//! 本版說明視窗的畫面：目前執行版本的 release 卡片，底部列提供前往 GitHub 與關閉。

use eframe::egui::{Frame, Id, Label, Margin, Panel, RichText, Ui};
use egui_commonmark::CommonMarkCache;

use super::release_card;
use super::theme;
use super::widgets::{
    error_with_retry, hint, primary_button, right_aligned, scroll_page, secondary_button,
};
use crate::i18n::t;
use crate::runtime::updater::Release;

/// 本版說明的載入狀態。
#[derive(Debug, Clone, PartialEq)]
pub enum NotesState {
    Loading,
    Loaded(Release),
    NotFound,
    Failed(String),
}

/// 使用者在本版說明視窗按下的動作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotesAction {
    Retry,
    OpenOnGitHub,
    Close,
}

/// 本版說明視窗的畫面狀態：Markdown 快取。
#[derive(Default)]
pub struct ReleaseNotesView {
    markdown: CommonMarkCache,
}

impl ReleaseNotesView {
    pub fn new() -> Self {
        Self::default()
    }

    /// 畫一幀；`version` 是目前執行的版本（不含 `v`）。
    pub fn ui(&mut self, ui: &mut Ui, version: &str, state: &NotesState) -> Vec<NotesAction> {
        let mut actions = Vec::new();
        Panel::bottom("release_notes_footer")
            .frame(
                Frame::new()
                    .fill(theme::MODAL_FOOT)
                    .inner_margin(release_card::FOOTER_MARGIN),
            )
            .show(ui, |ui| footer(ui, &mut actions));
        Panel::top("release_notes_header")
            .frame(Frame::new().inner_margin(release_card::HEADER_MARGIN))
            .show(ui, |ui| header(ui, version));
        let margin = Margin::symmetric(20, release_card::LIST_PADDING);
        scroll_page(
            ui,
            "release_notes_page",
            margin,
            theme::CARD_GAP,
            |ui| match state {
                NotesState::Loading => release_card::loading(ui, version),
                NotesState::Loaded(release) => {
                    release_card::show(ui, &mut self.markdown, release, false);
                }
                NotesState::NotFound => release_card::shell(
                    ui,
                    version,
                    |_| {},
                    |ui| {
                        let text = t("release_notes.not_found", &[("version", version)]);
                        hint(ui, &text, theme::TEXT_SECONDARY);
                    },
                ),
                NotesState::Failed(error) => release_card::shell(
                    ui,
                    version,
                    |_| {},
                    |ui| {
                        let text = t("release_notes.failed", &[("error", error)]);
                        if error_with_retry(ui, &text) {
                            actions.push(NotesAction::Retry);
                        }
                    },
                ),
            },
        );
        actions
    }
}

/// 固定在上方的標題與副標。
fn header(ui: &mut Ui, version: &str) {
    ui.spacing_mut().item_spacing.y = 4.0;
    let heading = t("release_notes.heading", &[("version", version)]);
    ui.add(Label::new(RichText::new(heading).font(theme::bold(20.0))));
    hint(
        ui,
        &t("release_notes.subheading", &[]),
        theme::TEXT_SECONDARY,
    );
}

/// 底部列：右側〔在 GitHub 上查看〕與〔關閉〕。
fn footer(ui: &mut Ui, actions: &mut Vec<NotesAction>) {
    let buttons = Id::unique("release_notes_buttons");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        right_aligned(ui, buttons, |ui| {
            if secondary_button(ui, &t("release_notes.open_github", &[])).clicked() {
                actions.push(NotesAction::OpenOnGitHub);
            }
            if primary_button(ui, &t("update.close", &[])).clicked() {
                actions.push(NotesAction::Close);
            }
        });
    });
}
