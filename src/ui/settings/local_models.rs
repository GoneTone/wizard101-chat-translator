//! 本地模型頁的「已下載模型」卡：總佔用空間、每個模型的狀態與動作，以及沒有模型時的空白提示。

use eframe::egui::{Label, RichText, Ui, vec2};

use super::env::SettingsEnv;
use super::local_card::{padded, section};
use super::local_rate::{Rates, Speed, paused_text, transfer_text};
use super::quant_table::{fit_badge, fraction, gb};
use super::theme;
use super::widgets::{
    danger_secondary_button, primary_button, progress_bar, right_aligned, right_aligned_width,
    secondary_button, toggle,
};
use crate::config::local::LocalSettings;
use crate::i18n::t;
use crate::local::fit::{Need, rate};
use crate::local::store::{Installed, total_size};
use crate::local::{Command, DownloadState, Snapshot};
use crate::logs::log;

/// 這張卡要頁面處理的事（需要對話框或服務草稿的）。
pub(super) enum ModelAction {
    OpenDownload,
    Delete(String),
}

/// 一列模型這一刻的狀態。
enum RowState<'a> {
    Ready,
    Running(u64, u64),
    Paused(u64, u64),
    Interrupted,
    Failed(&'a str),
}

pub(super) fn ui(
    ui: &mut Ui,
    env: &dyn SettingsEnv,
    snapshot: &Snapshot,
    settings: &mut LocalSettings,
    rates: &Rates,
) -> Option<ModelAction> {
    let mut header_open = false;
    let mut empty_open = false;
    let mut action = None;
    section(
        ui,
        &t("local.models.title", &[]),
        |ui| {
            if !snapshot.models.is_empty() {
                let total = gb(total_size(&snapshot.models));
                ui.label(
                    RichText::new(t("local.models.total", &[("size", &total)]))
                        .color(theme::TEXT_SECONDARY),
                );
            }
            if primary_button(ui, &t("local.models.download", &[])).clicked() {
                header_open = true;
            }
        },
        |ui| {
            padded(ui, |ui| {
                if snapshot.models.is_empty() {
                    empty_open = empty(ui);
                }
                for model in &snapshot.models {
                    action = action
                        .take()
                        .or(row(ui, env, snapshot, settings, rates, model));
                }
            });
        },
    );
    (header_open || empty_open)
        .then_some(ModelAction::OpenDownload)
        .or(action)
}

/// 空白提示與連結；回傳是否點了連結。
fn empty(ui: &mut Ui) -> bool {
    ui.add(Label::new(
        RichText::new(t("local.models.empty", &[])).color(theme::TEXT_SECONDARY),
    ));
    ui.link(RichText::new(t("local.models.empty_link", &[])).color(theme::LINK))
        .clicked()
}

fn row(
    ui: &mut Ui,
    env: &dyn SettingsEnv,
    snapshot: &Snapshot,
    settings: &mut LocalSettings,
    rates: &Rates,
    model: &Installed,
) -> Option<ModelAction> {
    let state = match snapshot.downloads.get(&model.id) {
        Some(DownloadState::Running { done, total }) => RowState::Running(*done, *total),
        Some(DownloadState::Paused { done, total }) => RowState::Paused(*done, *total),
        Some(DownloadState::Failed(error)) => RowState::Failed(error),
        None if model.complete => RowState::Ready,
        None => RowState::Interrupted,
    };
    let need = snapshot.model_need(model, settings);
    let id = ui.scope_id().with(("local_model_row", &model.id));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let reserved = right_aligned_width(ui, id);
        let width = (ui.available_width() - reserved - 12.0).max(120.0);
        ui.allocate_ui(vec2(width, 0.0), |ui| {
            texts(ui, model, &state, rates.speed(&model.id))
        });
        right_aligned(ui, id, |ui| {
            controls(ui, env, snapshot, settings, model, &state, &need)
        })
    })
    .inner
}

fn texts(ui: &mut Ui, model: &Installed, state: &RowState, speed: Option<Speed>) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let name = RichText::new(&model.manifest.repo).font(theme::bold(theme::TEXT_SIZE));
        ui.add(Label::new(name).truncate());
        let size = gb(total_size(std::slice::from_ref(model)));
        // 標題只放 repo，量化放副標題開頭；錯誤照原文另起一行
        let with_quant = |text: String| format!("{} · {text}", model.manifest.quant);
        let line = match state {
            RowState::Ready | RowState::Failed(_) => with_quant(format!("{size} GB")),
            RowState::Running(done, total) => with_quant(transfer_text(*done, *total, speed)),
            RowState::Paused(done, total) => with_quant(paused_text(*done, *total)),
            RowState::Interrupted => with_quant(t("local.models.interrupted", &[])),
        };
        let line = RichText::new(line)
            .size(theme::HINT_SIZE)
            .color(theme::TEXT_SECONDARY);
        ui.add(Label::new(line).wrap());
        if let RowState::Failed(error) = state {
            let error = RichText::new(*error)
                .size(theme::HINT_SIZE)
                .color(theme::ERROR);
            ui.add(Label::new(error).wrap());
        }
    });
}

fn controls(
    ui: &mut Ui,
    env: &dyn SettingsEnv,
    snapshot: &Snapshot,
    settings: &mut LocalSettings,
    model: &Installed,
    state: &RowState,
    need: &Need,
) -> Option<ModelAction> {
    ui.spacing_mut().item_spacing.x = 8.0;
    let id = &model.id;
    let mut action = None;
    match state {
        RowState::Ready => {
            let manifest = &model.manifest;
            if manifest.has_mtp() {
                let mut on = settings.mtp_on(id);
                ui.label(
                    RichText::new("MTP")
                        .size(theme::HINT_SIZE)
                        .color(theme::TEXT_SECONDARY),
                );
                let label = format!("MTP {}", manifest.display_name());
                if toggle(ui, &mut on, &label).changed() {
                    settings.mtp.insert(id.clone(), on);
                }
            }
            let hw = &snapshot.hardware;
            let level = hw.measured.then(|| rate(need, manifest.moe, hw));
            fit_badge(ui, level, need.estimated(), snapshot.detecting);
        }
        RowState::Running(done, total) => {
            progress_bar(ui, fraction(*done, *total), 120.0);
            if secondary_button(ui, &t("button.cancel", &[])).clicked() {
                log(format!("[ui] model download cancel requested: id={id}"));
                env.local_command(Command::CancelDownload(id.clone()));
            }
        }
        RowState::Paused(..) | RowState::Interrupted | RowState::Failed(_) => {
            if primary_button(ui, &t("local.continue_download", &[])).clicked() {
                log(format!("[ui] model download resume requested: id={id}"));
                env.local_command(Command::Resume(id.clone()));
            }
        }
    }
    if !matches!(state, RowState::Running(..))
        && danger_secondary_button(ui, &t("button.delete", &[])).clicked()
    {
        action = Some(ModelAction::Delete(id.clone()));
    }
    action
}
