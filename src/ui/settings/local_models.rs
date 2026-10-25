//! 本地模型頁的「已下載模型」卡：總佔用空間、每個模型的狀態與動作，以及沒有模型時的空白提示。

use std::time::Duration;

use eframe::egui::{Label, ProgressBar, RichText, Ui, vec2};

use super::env::SettingsEnv;
use super::local_card::{padded, section};
use super::local_rate::{Rates, eta, format_speed};
use super::quant_table::{dot, fit_color, fit_text, gb};
use super::theme;
use super::widgets::{
    danger_secondary_button, primary_button, right_aligned, right_aligned_width, secondary_button,
    toggle,
};
use crate::config::local::LocalSettings;
use crate::i18n::t;
use crate::local::catalog::RECOMMENDED;
use crate::local::fit::{Fit, fit};
use crate::local::hardware::{GpuKind, Hardware};
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
    ui.link(t("local.models.empty_link", &[])).clicked()
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
    let id = ui.scope_id().with(("local_model_row", &model.id));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let reserved = right_aligned_width(ui, id);
        let width = (ui.available_width() - reserved - 12.0).max(120.0);
        ui.allocate_ui(vec2(width, 0.0), |ui| {
            texts(ui, model, &state, rates.speed(&model.id))
        });
        right_aligned(ui, id, |ui| {
            controls(ui, env, snapshot, settings, model, &state)
        })
    })
    .inner
}

fn texts(ui: &mut Ui, model: &Installed, state: &RowState, speed: Option<f64>) {
    let small = |text: String| RichText::new(text).size(theme::HINT_SIZE);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let name = RichText::new(display_name(model)).font(theme::bold(theme::TEXT_SIZE));
        ui.add(Label::new(name).truncate());
        let size = gb(total_size(std::slice::from_ref(model)));
        let line = match state {
            RowState::Ready => small(format!("{} · {size} GB", model.manifest.repo)),
            RowState::Running(done, total) => {
                let mut line = t(
                    "local.models.downloading",
                    &[("done", &gb(*done)), ("total", &gb(*total))],
                );
                if let Some(rate) = speed {
                    let left = eta_text(eta(*done, *total, rate));
                    let args = [("speed", format_speed(rate)), ("eta", left)];
                    let args: Vec<(&str, &str)> =
                        args.iter().map(|(k, v)| (*k, v.as_str())).collect();
                    line += &format!(" · {}", t("local.models.speed_eta", &args));
                }
                small(line)
            }
            RowState::Paused(done, total) => small(t(
                "local.models.paused",
                &[("done", &gb(*done)), ("total", &gb(*total))],
            )),
            RowState::Interrupted => small(format!(
                "{} · {}",
                t("local.models.interrupted", &[]),
                model.manifest.repo
            )),
            RowState::Failed(error) => small((*error).to_owned()),
        };
        let color = if matches!(state, RowState::Failed(_)) {
            theme::ERROR
        } else {
            theme::TEXT_SECONDARY
        };
        ui.add(Label::new(line.color(color)).wrap());
    });
}

fn controls(
    ui: &mut Ui,
    env: &dyn SettingsEnv,
    snapshot: &Snapshot,
    settings: &mut LocalSettings,
    model: &Installed,
    state: &RowState,
) -> Option<ModelAction> {
    ui.spacing_mut().item_spacing.x = 8.0;
    let id = &model.id;
    let mut action = None;
    match state {
        RowState::Ready => {
            let manifest = &model.manifest;
            if manifest.mtp.is_some() || manifest.has_builtin_mtp {
                let mut on = settings.mtp.get(id).copied().unwrap_or(true);
                ui.label(
                    RichText::new("MTP")
                        .size(theme::HINT_SIZE)
                        .color(theme::TEXT_SECONDARY),
                );
                let label = format!("MTP {}", display_name(model));
                if toggle(ui, &mut on, &label).changed() {
                    settings.mtp.insert(id.clone(), on);
                }
            }
            let level = rate(manifest.required, manifest.moe, &snapshot.hardware);
            dot(ui, fit_color(level));
            ui.label(RichText::new(fit_text(level)).color(fit_color(level)));
        }
        RowState::Running(done, total) => {
            let fraction = if *total == 0 {
                0.0
            } else {
                (*done as f64 / *total as f64).clamp(0.0, 1.0) as f32
            };
            ui.add(ProgressBar::new(fraction).desired_width(120.0));
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

/// 剩餘時間文字：不到一分鐘用秒，不到一小時用分，其餘用小時加分。
fn eta_text(left: Duration) -> String {
    let secs = left.as_secs_f64().ceil() as u64;
    let n = |v: u64| v.to_string();
    match secs {
        0..60 => t("local.models.eta_seconds", &[("n", &n(secs.max(1)))]),
        60..3600 => t("local.models.eta_minutes", &[("n", &n(secs.div_ceil(60)))]),
        _ => t(
            "local.models.eta_hours",
            &[("h", &n(secs / 3600)), ("m", &n(secs % 3600 / 60))],
        ),
    }
}

/// 推薦清單內的 repo 用顯示名稱，其餘用 repo 名；後接量化。
fn display_name(model: &Installed) -> String {
    let repo = model.manifest.repo.as_str();
    let name = RECOMMENDED
        .iter()
        .find(|r| r.repo == repo)
        .map_or_else(|| repo.rsplit('/').next().unwrap_or(repo), |r| r.name);
    format!("{name} · {}", model.manifest.quant)
}

/// 用清單上存的所需量對目前硬體評等；VRAM 還沒量到（且不是內顯）時為 `None`。
fn rate(required: u64, moe: bool, hw: &Hardware) -> Option<Fit> {
    let integrated = hw
        .gpu
        .as_ref()
        .is_some_and(|g| g.kind == GpuKind::Integrated);
    (hw.vram_total > 0 || integrated).then(|| fit(required, moe, hw))
}
