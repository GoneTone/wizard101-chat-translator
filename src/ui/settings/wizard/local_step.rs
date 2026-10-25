//! 精靈的本地模型一鍵安裝：確認卡片、換模型、引擎與模型的下載進度。

use std::mem;

use eframe::egui::{Frame, Label, Margin, ProgressBar, RichText, Ui};

use crate::i18n::t;
use crate::local::catalog::{RECOMMENDED, wizard_pick};
use crate::local::engine::{Variant, auto_variant};
use crate::local::fit::{Fit, memory_kind, total_size};
use crate::local::store::model_id;
use crate::local::{Command, DownloadState, EngineState, RepoInfo, RepoState, Snapshot};
use crate::logs::log;
use crate::ui::settings::env::SettingsEnv;
use crate::ui::settings::local_dialog::{
    DialogMode, DialogOutcome, DownloadDialog, Selection, Tab, memory_key,
};
use crate::ui::settings::quant_table::{combined, dot, fit_color, fit_text, gb};
use crate::ui::settings::theme;
use crate::ui::settings::widgets::{card, primary_button, secondary_button};

/// 這一幀安裝流程所處的階段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallState {
    Choosing,
    Downloading,
    /// 所選模型已下載完成（模型 id）。
    Done(String),
    Failed(String),
}

/// 確認卡片要顯示的一個選擇與它的數字。
struct Plan {
    selection: Selection,
    title: String,
    fit: Option<Fit>,
    model_bytes: u64,
    required: u64,
    moe: bool,
    id: String,
}

/// 精靈裡選了本地服務後的安裝流程狀態。
#[derive(Default)]
pub struct LocalInstall {
    /// 使用者在對話框改選的；`None` 用自動挑的。
    selection: Option<Selection>,
    dialog: Option<DownloadDialog>,
    started: bool,
    /// 按了〔下載〕後追蹤的模型 id。
    tracked: Option<String>,
    cancelled: bool,
    /// 剛按〔下載〕：hub 還沒蓋掉舊的失敗紀錄前，不顯示它。
    stale_failure: bool,
    /// 表單原本就有的模型 id；進入時若已在 hub 的清單內就直接沿用。
    existing: String,
    state: Option<InstallState>,
}

impl LocalInstall {
    /// `existing` 是服務原有的模型 id（沒有就空字串）。
    pub fn new(existing: String) -> Self {
        Self {
            existing,
            ..Self::default()
        }
    }

    /// 所選模型是否已下載完成。
    pub fn is_done(&self) -> bool {
        matches!(self.state, Some(InstallState::Done(_)))
    }

    /// 畫一幀；回傳目前階段。`snapshot` 是這一幀唯一的一份狀態。
    pub fn ui(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, snapshot: &Snapshot) -> InstallState {
        self.start(env, snapshot);
        self.adopt(snapshot);
        let state = self.phase(snapshot);
        match &state {
            InstallState::Choosing => self.choosing(ui, env, snapshot),
            _ => self.downloading(ui, env, snapshot, &state),
        }
        self.show_dialog(ui, env, snapshot);
        self.state = Some(state.clone());
        state
    }

    fn start(&mut self, env: &dyn SettingsEnv, snapshot: &Snapshot) {
        if mem::replace(&mut self.started, true) {
            return;
        }
        log(format!(
            "[ui] wizard local install entered: engine={:?} existing_model={}",
            snapshot.engine, self.existing
        ));
        if snapshot.models.iter().any(|m| m.id == self.existing) {
            self.tracked = Some(self.existing.clone());
        }
        env.local_command(Command::RefreshHardware);
        for rec in RECOMMENDED
            .iter()
            .filter(|r| !snapshot.repos.contains_key(r.repo))
        {
            env.local_command(Command::QueryRepo(rec.repo.to_owned()));
        }
        if snapshot.engine == EngineState::Missing {
            env.local_command(Command::InstallEngine {
                build: None,
                variant: None,
            });
        }
    }

    /// 還沒追蹤任何模型時，認領 hub 裡正在跑的、同一個選擇的下載（例如返回上一步再進來）。
    fn adopt(&mut self, snapshot: &Snapshot) {
        if self.tracked.is_some() || self.cancelled {
            return;
        }
        let Some(plan) = self.plan(snapshot) else {
            return;
        };
        if matches!(
            snapshot.downloads.get(&plan.id),
            Some(DownloadState::Running { .. } | DownloadState::Paused { .. })
        ) {
            log(format!(
                "[ui] wizard local adopted running download: id={}",
                plan.id
            ));
            self.tracked = Some(plan.id);
        }
    }

    fn phase(&mut self, snapshot: &Snapshot) -> InstallState {
        let Some(id) = &self.tracked else {
            return InstallState::Choosing;
        };
        if snapshot.ready_model(id).is_some() {
            return InstallState::Done(id.clone());
        }
        match snapshot.downloads.get(id) {
            Some(DownloadState::Failed(e)) if !self.stale_failure => {
                InstallState::Failed(e.clone())
            }
            Some(DownloadState::Failed(_)) => InstallState::Downloading,
            _ => {
                self.stale_failure = false;
                InstallState::Downloading
            }
        }
    }

    /// 使用者改選的，沒有就用 `wizard_pick` 從已查好的推薦裡挑。
    fn plan(&self, snapshot: &Snapshot) -> Option<Plan> {
        let selection = match &self.selection {
            Some(s) => s.clone(),
            None => default_selection(snapshot)?,
        };
        let Some(RepoState::Ready(info)) = snapshot.repos.get(&selection.repo) else {
            return None;
        };
        let choice = info.catalog.choices.get(selection.choice)?;
        let mtp = match selection.mtp {
            Some(m) => Some(info.catalog.mtp.get(m)?),
            None => None,
        };
        let name = RECOMMENDED
            .iter()
            .find(|r| r.repo == selection.repo)
            .map_or(selection.repo.as_str(), |r| r.name);
        let mut title = format!("{name} · {}", choice.quant.label);
        if let Some(m) = mtp {
            title += &format!(" + MTP {}", m.quant.label);
        }
        let (required, fit) = combined(info, &snapshot.hardware, (selection.choice, selection.mtp));
        Some(Plan {
            id: model_id(&selection.repo, &choice.quant.label),
            title,
            fit,
            model_bytes: total_size(choice) + mtp.map_or(0, |m| m.file.size),
            required,
            moe: info.meta.expert_count > 0,
            selection,
        })
    }

    fn choosing(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, snapshot: &Snapshot) {
        ui.add(Label::new(RichText::new(t("local.wizard.intro", &[])).color(theme::TEXT)).wrap());
        let Some(plan) = self.plan(snapshot) else {
            hint(ui, &t("local.dialog.loading", &[]));
            self.engine_status(ui, env, snapshot);
            return;
        };
        let engine = engine_download(snapshot);
        let total = plan.model_bytes + engine.as_ref().map_or(0, |e| e.1);
        card(ui, |ui| {
            Frame::new()
                .inner_margin(Margin::symmetric(16, 14))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    ui.horizontal(|ui| {
                        let title = RichText::new(&plan.title).font(theme::bold(theme::TEXT_SIZE));
                        ui.add(Label::new(title));
                        dot(ui, fit_color(plan.fit));
                        ui.label(RichText::new(fit_text(plan.fit)).color(fit_color(plan.fit)));
                    });
                    hint(
                        ui,
                        &t(
                            "local.wizard.model_size",
                            &[("size", &gb(plan.model_bytes))],
                        ),
                    );
                    if let Some((variant, bytes)) = &engine {
                        let args = [("variant", variant.to_string()), ("size", gb(*bytes))];
                        let args: Vec<_> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
                        hint(ui, &t("local.wizard.engine_size", &args));
                    }
                    hint(ui, &t("local.wizard.total", &[("size", &gb(total))]));
                    let kind = memory_kind(plan.moe, &snapshot.hardware);
                    hint(ui, &t(memory_key(kind), &[("size", &gb(plan.required))]));
                });
        });
        let ready = matches!(snapshot.engine, EngineState::Ready(_));
        let mut change = false;
        let mut download = false;
        ui.horizontal(|ui| {
            download = ui
                .add_enabled_ui(ready, |ui| {
                    primary_button(ui, &t("local.wizard.download", &[]))
                })
                .inner
                .clicked();
            change = secondary_button(ui, &t("local.wizard.change", &[])).clicked();
        });
        if !ready && !matches!(snapshot.engine, EngineState::Failed(_)) {
            hint(ui, &t("local.wizard.waiting_engine", &[]));
        }
        self.engine_status(ui, env, snapshot);
        if self.cancelled {
            hint(ui, &t("local.wizard.cancelled_hint", &[]));
        }
        if change {
            self.dialog = Some(DownloadDialog::new(DialogMode::Select, Tab::Recommended));
        }
        if download && ready {
            log(format!(
                "[ui] wizard local download requested: repo={} choice={} mtp={:?} id={}",
                plan.selection.repo, plan.selection.choice, plan.selection.mtp, plan.id
            ));
            env.local_command(Command::Download {
                repo: plan.selection.repo,
                choice: plan.selection.choice,
                mtp: plan.selection.mtp,
            });
            self.stale_failure = matches!(
                snapshot.downloads.get(&plan.id),
                Some(DownloadState::Failed(_))
            );
            self.tracked = Some(plan.id);
            self.cancelled = false;
        }
    }

    fn show_dialog(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, snapshot: &Snapshot) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        match dialog.ui(ui, env, snapshot) {
            DialogOutcome::Open => {}
            DialogOutcome::Closed => self.dialog = None,
            DialogOutcome::Selected(selection) => {
                self.selection = Some(selection);
                self.dialog = None;
                self.tracked = None;
                self.cancelled = false;
            }
        }
    }

    /// 引擎安裝中的進度，或失敗的原文與〔重試〕。
    fn engine_status(&self, ui: &mut Ui, env: &dyn SettingsEnv, snapshot: &Snapshot) {
        match &snapshot.engine {
            EngineState::Installing { done, total, .. } => progress(
                ui,
                &t(
                    "local.wizard.engine_installing",
                    &[("done", &gb(*done)), ("total", &gb(*total))],
                ),
                *done,
                *total,
            ),
            EngineState::Failed(error) => {
                error_with_retry(ui, error, || {
                    env.local_command(Command::InstallEngine {
                        build: None,
                        variant: None,
                    })
                });
            }
            EngineState::Missing | EngineState::Ready(_) => {}
        }
    }

    fn downloading(
        &mut self,
        ui: &mut Ui,
        env: &dyn SettingsEnv,
        snapshot: &Snapshot,
        state: &InstallState,
    ) {
        let Some(id) = self.tracked.clone() else {
            return;
        };
        if matches!(snapshot.engine, EngineState::Ready(_)) {
            hint(ui, &t("local.wizard.engine_ready", &[]));
        }
        self.engine_status(ui, env, snapshot);
        match state {
            InstallState::Done(_) => {
                hint(ui, &t("local.wizard.done", &[]));
                if secondary_button(ui, &t("local.wizard.change", &[])).clicked() {
                    self.dialog = Some(DownloadDialog::new(DialogMode::Select, Tab::Recommended));
                }
            }
            InstallState::Failed(error) => {
                error_with_retry(ui, error, || env.local_command(Command::Resume(id.clone())));
            }
            _ => {
                let (done, total) = match snapshot.downloads.get(&id) {
                    Some(
                        DownloadState::Running { done, total }
                        | DownloadState::Paused { done, total },
                    ) => (*done, *total),
                    _ => (0, 0),
                };
                let text = t(
                    "local.models.downloading",
                    &[("done", &gb(done)), ("total", &gb(total))],
                );
                progress(ui, &text, done, total);
                let interrupted = !snapshot.downloads.contains_key(&id)
                    && snapshot.models.iter().any(|m| m.id == id);
                if interrupted && primary_button(ui, &t("local.continue_download", &[])).clicked() {
                    env.local_command(Command::Resume(id.clone()));
                }
            }
        }
        if !matches!(state, InstallState::Done(_))
            && secondary_button(ui, &t("local.wizard.cancel", &[])).clicked()
        {
            log(format!("[ui] wizard local download cancelled: id={id}"));
            env.local_command(Command::CancelDownload(id));
            self.tracked = None;
            self.cancelled = true;
        }
    }
}

/// 六個推薦裡查好的依 `wizard_pick` 挑一個，用該 repo 的自動挑選；等級不明的排最後。
fn default_selection(snapshot: &Snapshot) -> Option<Selection> {
    let rated: Vec<_> = RECOMMENDED
        .iter()
        .filter_map(|rec| {
            let info = ready_info(snapshot, rec.repo)?;
            let fit = info
                .auto
                .and_then(|a| combined(info, &snapshot.hardware, (a.choice, a.mtp)).1)
                .unwrap_or(Fit::NoFit);
            Some((rec, fit))
        })
        .collect();
    let rec = wizard_pick(&rated)?;
    let auto = ready_info(snapshot, rec.repo)?.auto?;
    Some(Selection {
        repo: rec.repo.to_owned(),
        choice: auto.choice,
        mtp: auto.mtp,
    })
}

fn ready_info<'a>(snapshot: &'a Snapshot, repo: &str) -> Option<&'a RepoInfo> {
    match snapshot.repos.get(repo) {
        Some(RepoState::Ready(info)) => Some(info),
        _ => None,
    }
}

/// 引擎還沒裝好時要下載的變體與位元組；已裝好或版本清單還沒到則 `None`。
fn engine_download(snapshot: &Snapshot) -> Option<(Variant, u64)> {
    if matches!(snapshot.engine, EngineState::Ready(_)) {
        return None;
    }
    let Some(Ok(builds)) = &snapshot.builds else {
        return None;
    };
    let variant = auto_variant(snapshot.hardware.gpu.as_ref());
    let (_, assets) = builds
        .first()?
        .variants
        .iter()
        .find(|(v, _)| *v == variant)?;
    Some((variant, assets.iter().map(|a| a.size).sum()))
}

fn hint(ui: &mut Ui, text: &str) {
    let text = RichText::new(text)
        .size(theme::HINT_SIZE)
        .color(theme::TEXT_SECONDARY);
    ui.add(Label::new(text).wrap());
}

fn progress(ui: &mut Ui, text: &str, done: u64, total: u64) {
    hint(ui, text);
    let fraction = if total == 0 {
        0.0
    } else {
        (done as f64 / total as f64).clamp(0.0, 1.0) as f32
    };
    ui.add(ProgressBar::new(fraction));
}

fn error_with_retry(ui: &mut Ui, error: &str, retry: impl FnOnce()) {
    ui.add(Label::new(RichText::new(error).color(theme::ERROR)).wrap());
    if secondary_button(ui, &t("local.dialog.retry", &[])).clicked() {
        log(format!("[ui] wizard local retry: error={error}"));
        retry();
    }
}
