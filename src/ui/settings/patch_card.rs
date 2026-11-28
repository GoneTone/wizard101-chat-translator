//! 每份遊戲安裝的更新狀態：啟動視窗的狀態卡與設定頁安裝位置下方共用的進度條與文字。

use std::collections::BTreeMap;
use std::time::Instant;

use eframe::egui::{Color32, Frame, Label, Margin, RichText, Ui};

use super::local_rate::{Rates, Speed, transfer_text};
use super::quant_table::{fraction, gb, grouped};
use super::theme;
use super::widgets::{card, hint, progress_bar, separator};
use crate::i18n::t;
use crate::launcher::install::InstallSource;
use crate::launcher::patch::{PatchFailure, PatchStatus};

/// 各安裝最近一次的更新狀態與下載速度；`None` 狀態就收起該安裝。
#[derive(Default)]
pub struct PatchBoard {
    /// 依 `InstallSource` 排序：Steam 在前。
    statuses: BTreeMap<InstallSource, PatchStatus>,
    rates: Rates,
}

impl PatchBoard {
    /// 換上 `source` 的狀態（`None` 是收起），並以 `now` 更新下載速度。
    pub fn apply(&mut self, source: InstallSource, status: Option<PatchStatus>, now: Instant) {
        match status {
            Some(status) => self.statuses.insert(source, status),
            None => self.statuses.remove(&source),
        };
        // observe_running 會丟掉沒列進來的鍵，另一份安裝的下載也要一起帶
        let running = self
            .statuses
            .iter()
            .filter_map(|(source, status)| match status {
                PatchStatus::Downloading { done, .. } => {
                    Some((rate_key(*source).to_owned(), *done))
                }
                _ => None,
            })
            .collect();
        self.rates.observe_running(now, running);
    }

    pub fn is_empty(&self) -> bool {
        self.statuses.is_empty()
    }

    pub fn status(&self, source: InstallSource) -> Option<&PatchStatus> {
        self.statuses.get(&source)
    }
}

fn rate_key(source: InstallSource) -> &'static str {
    match source {
        InstallSource::Steam => "steam",
        InstallSource::Official => "official",
    }
}

/// 更新狀態的顯示文字與顏色：進行中說明色、通過綠、沒通過紅。
pub fn status_text(status: &PatchStatus, speed: Option<Speed>) -> (String, Color32) {
    match status {
        PatchStatus::Checking { done, total } => {
            let (done, total) = (grouped(*done), grouped(*total));
            let text = t("patch.checking", &[("done", &done), ("total", &total)]);
            (text, theme::TEXT_SECONDARY)
        }
        PatchStatus::Downloading { done, total } => {
            (transfer_text(*done, *total, speed), theme::TEXT_SECONDARY)
        }
        PatchStatus::UpToDate => (t("patch.up_to_date", &[]), theme::OK),
        PatchStatus::Updated => (t("patch.updated", &[]), theme::OK),
        PatchStatus::Failed(failure) => (failure_text(failure), theme::ERROR),
    }
}

fn failure_text(failure: &PatchFailure) -> String {
    let with_error = |key, error: &str| t(key, &[("error", error)]);
    match failure {
        PatchFailure::SteamOutdated => t("patch.steam_outdated", &[]),
        PatchFailure::GameOpen => t("patch.game_open", &[]),
        PatchFailure::Connect(e) => with_error("patch.failed_connect", e),
        PatchFailure::Protocol(e) => with_error("patch.failed_protocol", e),
        PatchFailure::Download(e) => with_error("patch.failed_download", e),
        PatchFailure::NoSpace { need, free } => t(
            "patch.no_space",
            &[("need", &gb(*need)), ("free", &gb(*free))],
        ),
        PatchFailure::Io(e) => with_error("patch.failed_io", e),
    }
}

/// 安裝的顯示名稱：官網版／Steam 版。
pub fn install_name(source: InstallSource) -> String {
    match source {
        InstallSource::Official => t("patch.install_official", &[]),
        InstallSource::Steam => t("patch.install_steam", &[]),
    }
}

/// 一份安裝的狀態：檢查與下載中先畫進度條，再畫文字；沒有狀態就不畫。
pub(super) fn status_block(ui: &mut Ui, board: &PatchBoard, source: InstallSource) {
    let Some(status) = board.status(source) else {
        return;
    };
    if let PatchStatus::Checking { done, total } | PatchStatus::Downloading { done, total } = status
    {
        progress_bar(ui, fraction(*done, *total), ui.available_width());
    }
    let (text, color) = status_text(status, board.rates.speed(rate_key(source)));
    hint(ui, &text, color);
}

/// 啟動視窗的狀態卡：每份安裝一段（粗體名稱＋狀態），段間分隔線；沒有狀態時不畫。
pub(super) fn board_card(ui: &mut Ui, board: &PatchBoard) {
    if board.is_empty() {
        return;
    }
    card(ui, |ui| {
        Frame::new()
            .inner_margin(Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for (i, &source) in board.statuses.keys().enumerate() {
                    if i > 0 {
                        ui.add_space(4.0);
                        separator(ui);
                        ui.add_space(10.0);
                    }
                    let name =
                        RichText::new(install_name(source)).font(theme::bold(theme::TEXT_SIZE));
                    ui.add(Label::new(name));
                    status_block(ui, board, source);
                }
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    /// 每種狀態都經 key 取字、配對應的顏色。
    #[test]
    fn status_text_for_every_variant() {
        let e = || "boom".to_owned();
        let secondary = theme::TEXT_SECONDARY;
        let failed = |f| PatchStatus::Failed(f);
        let cases = [
            (
                PatchStatus::Checking {
                    done: 2310,
                    total: 3735,
                },
                "patch.checking",
                vec![("done", "2,310"), ("total", "3,735")],
                secondary,
            ),
            (PatchStatus::UpToDate, "patch.up_to_date", vec![], theme::OK),
            (PatchStatus::Updated, "patch.updated", vec![], theme::OK),
            (
                failed(PatchFailure::SteamOutdated),
                "patch.steam_outdated",
                vec![],
                theme::ERROR,
            ),
            (
                failed(PatchFailure::GameOpen),
                "patch.game_open",
                vec![],
                theme::ERROR,
            ),
            (
                failed(PatchFailure::Connect(e())),
                "patch.failed_connect",
                vec![("error", "boom")],
                theme::ERROR,
            ),
            (
                failed(PatchFailure::Protocol(e())),
                "patch.failed_protocol",
                vec![("error", "boom")],
                theme::ERROR,
            ),
            (
                failed(PatchFailure::Download(e())),
                "patch.failed_download",
                vec![("error", "boom")],
                theme::ERROR,
            ),
            (
                failed(PatchFailure::NoSpace {
                    need: 3 * GB,
                    free: GB / 2,
                }),
                "patch.no_space",
                vec![("need", "3.0"), ("free", "0.5")],
                theme::ERROR,
            ),
            (
                failed(PatchFailure::Io(e())),
                "patch.failed_io",
                vec![("error", "boom")],
                theme::ERROR,
            ),
        ];
        for (status, key, args, color) in cases {
            let text = t(key, &args);
            assert_ne!(text, key, "{key} is missing from the locale");
            assert_eq!(status_text(&status, None), (text, color), "{status:?}");
        }
        let downloading = PatchStatus::Downloading {
            done: GB,
            total: 3 * GB,
        };
        assert_eq!(
            status_text(&downloading, None),
            (transfer_text(GB, 3 * GB, None), secondary)
        );
        for source in [InstallSource::Official, InstallSource::Steam] {
            assert!(!install_name(source).starts_with("patch."), "{source:?}");
        }
    }

    /// `None` 收起該安裝，另一份留著；Steam 排在前面；下載中的速度跟著狀態更新、收起後丟掉。
    #[test]
    fn board_clears_on_none() {
        let mut board = PatchBoard::default();
        let t0 = Instant::now();
        let at = |ms| t0 + std::time::Duration::from_millis(ms);
        let downloading = |done| Some(PatchStatus::Downloading { done, total: 4000 });
        board.apply(InstallSource::Official, downloading(0), at(0));
        board.apply(InstallSource::Steam, Some(PatchStatus::UpToDate), at(100));
        board.apply(InstallSource::Official, downloading(1000), at(1000));
        assert_eq!(
            board.statuses.keys().copied().collect::<Vec<_>>(),
            [InstallSource::Steam, InstallSource::Official]
        );
        assert!(board.rates.speed("official").is_some());

        board.apply(InstallSource::Steam, None, at(1100));
        assert!(board.status(InstallSource::Steam).is_none());
        assert!(
            board.rates.speed("official").is_some(),
            "kept across other installs"
        );
        board.apply(InstallSource::Official, None, at(1200));
        assert!(board.is_empty());
        assert!(board.rates.speed("official").is_none());
    }
}
