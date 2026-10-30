//! 精靈的本地模型安裝：先由使用者下載引擎，裝好後再顯示確認卡片、換模型與模型的下載進度。

use std::mem;
use std::time::Instant;

use eframe::egui::{Frame, Label, Margin, RichText, TextStyle, Ui};

use self::widgets::{default_selection, engine_download, error_with_retry, progress};
use crate::i18n::t;
use crate::local::catalog::RECOMMENDED;
use crate::local::fit::{Rating, total_size};
use crate::local::store::choice_id;
use crate::local::{Command, DownloadState, EngineState, RepoState, Snapshot};
use crate::logs::log;
use crate::ui::settings::env::SettingsEnv;
use crate::ui::settings::local_dialog::{
    DialogMode, DialogOutcome, DownloadDialog, Selection, Tab,
};
use crate::ui::settings::local_engine::variant_label;
use crate::ui::settings::local_rate::{ENGINE, Rates, engine_text, paused_text, transfer_text};
use crate::ui::settings::quant_table::{combined, fit_badge, gb, mtp_label};
use crate::ui::settings::theme;
use crate::ui::settings::widgets::{card, hint, primary_button, secondary_button, skeleton};

mod widgets;

/// 這一幀安裝流程所處的階段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallState {
    /// 引擎還沒裝好：先下載引擎才挑模型。
    Engine,
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
    rating: Rating,
    model_bytes: u64,
    id: String,
    /// 選的 MTP 檔（repo 內路徑）。
    mtp_path: Option<String>,
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
    /// 表單原本就有的模型 id；進入時若已在 hub 的清單內就直接沿用。
    existing: String,
    state: Option<InstallState>,
    rates: Rates,
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

    /// 返回上一步後再進來：留著選擇與追蹤中的下載（還在跑的照樣接著顯示），取消提示、對話框與舊表單的模型不留。
    pub fn reenter(&mut self) {
        log(format!(
            "[ui] wizard local install left: tracked={:?} cancelled={}",
            self.tracked, self.cancelled
        ));
        self.cancelled = false;
        self.dialog = None;
        self.existing.clear();
    }

    /// 畫一幀；回傳目前階段。`snapshot` 是這一幀唯一的一份狀態。
    pub fn ui(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, snapshot: &Snapshot) -> InstallState {
        self.start(env, snapshot);
        self.rates.observe(Instant::now(), snapshot);
        let state = self.phase(snapshot);
        match &state {
            InstallState::Engine => self.engine_step(ui, env, snapshot),
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
        // 引擎那段要顯示下載量
        if snapshot.builds.is_none() {
            env.local_command(Command::ListBuilds);
        }
        for rec in RECOMMENDED
            .iter()
            .filter(|r| !snapshot.repos.contains_key(r.repo))
        {
            env.local_command(Command::QueryRepo(rec.repo.to_owned()));
        }
    }

    /// 還沒追蹤任何模型時，認領同一個選擇：hub 裡正在跑的下載（例如返回上一步再進來），或已經下載好的模型。
    fn adopt(&mut self, snapshot: &Snapshot) {
        if self.tracked.is_some() || self.cancelled {
            return;
        }
        let Some(plan) = self.plan(snapshot) else {
            return;
        };
        let running = matches!(
            snapshot.downloads.get(&plan.id),
            Some(DownloadState::Running { .. } | DownloadState::Paused { .. })
        );
        // 量化相同但 MTP 檔不同的不算，要的 MTP 檔還得下載
        let downloaded = snapshot.ready_model(&plan.id).is_some_and(|m| {
            m.manifest.mtp.as_ref().map(|f| f.path.as_str()) == plan.mtp_path.as_deref()
        });
        if running || downloaded {
            log(format!(
                "[ui] wizard local adopted {}: id={}",
                if running {
                    "running download"
                } else {
                    "downloaded model"
                },
                plan.id
            ));
            self.tracked = Some(plan.id);
        }
    }

    /// 這一幀的階段；引擎沒裝好時一律停在引擎那段，連已下載好的模型也要等引擎才算完成。
    fn phase(&mut self, snapshot: &Snapshot) -> InstallState {
        if !matches!(snapshot.engine, EngineState::Ready(_)) {
            return InstallState::Engine;
        }
        self.adopt(snapshot);
        let Some(id) = &self.tracked else {
            return InstallState::Choosing;
        };
        if snapshot.ready_model(id).is_some() {
            return InstallState::Done(id.clone());
        }
        match snapshot.downloads.get(id) {
            Some(DownloadState::Failed(e)) => InstallState::Failed(e.clone()),
            _ => InstallState::Downloading,
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
        let mut title = format!("{} · {}", selection.repo, choice.quant.label);
        if let Some(m) = selection.mtp {
            title += &format!(" + {}", mtp_label(&info.catalog.mtp, m));
        }
        let rating = combined(info, (selection.choice, selection.mtp));
        Some(Plan {
            id: choice_id(&selection.repo, choice),
            title,
            rating,
            model_bytes: total_size(choice) + mtp.map_or(0, |m| m.file.size),
            mtp_path: mtp.map(|m| m.file.path.clone()),
            selection,
        })
    }

    /// 推薦卡的內容之後還會變：硬體偵測中（裝好引擎後會依它列出的裝置重量），或自動挑選時還有推薦 repo 沒查完。
    fn loading(&self, snapshot: &Snapshot) -> bool {
        let repos_pending = self.selection.is_none()
            && RECOMMENDED.iter().filter(|r| r.wizard).any(|r| {
                !matches!(
                    snapshot.repos.get(r.repo),
                    Some(RepoState::Ready(_) | RepoState::Failed(_))
                )
            });
        snapshot.detecting || repos_pending
    }

    /// 推薦卡要用、但查詢失敗的 repo 與錯誤原文：改選過就只看那個 repo，否則看精靈的推薦。
    fn failed_repos<'a>(&'a self, snapshot: &'a Snapshot) -> Vec<(&'a str, &'a str)> {
        let repos: Vec<&str> = match &self.selection {
            Some(s) => vec![s.repo.as_str()],
            None => RECOMMENDED
                .iter()
                .filter(|r| r.wizard)
                .map(|r| r.repo)
                .collect(),
        };
        repos
            .into_iter()
            .filter_map(|repo| match snapshot.repos.get(repo) {
                Some(RepoState::Failed(error)) => Some((repo, error.as_str())),
                _ => None,
            })
            .collect()
    }

    fn choosing(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, snapshot: &Snapshot) {
        ui.add(Label::new(RichText::new(t("local.wizard.intro", &[])).color(theme::TEXT)).wrap());
        let loading = self.loading(snapshot);
        let Some(plan) = self.plan(snapshot).filter(|_| !loading) else {
            match self.failed_repos(snapshot).first().filter(|_| !loading) {
                Some((_, error)) => error_with_retry(ui, &repo_error(error), "repos", || {
                    for (repo, _) in self.failed_repos(snapshot) {
                        env.local_command(Command::QueryRepo(repo.to_owned()));
                    }
                }),
                None => {
                    loading_card(ui);
                    ui.horizontal(|ui| {
                        let button = ui.spacing().interact_size.y;
                        skeleton(ui, 96.0, button);
                        skeleton(ui, 72.0, button);
                    });
                }
            }
            return;
        };
        card(ui, |ui| {
            Frame::new()
                .inner_margin(Margin::symmetric(16, 14))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    title_row(ui, &plan.title, Some(plan.rating));
                    hint(
                        ui,
                        &t(
                            "local.wizard.model_size",
                            &[("size", &gb(plan.model_bytes))],
                        ),
                        theme::TEXT_SECONDARY,
                    );
                });
        });
        let mut change = false;
        let mut download = false;
        ui.horizontal(|ui| {
            download = primary_button(ui, &t("local.wizard.download", &[])).clicked();
            change = secondary_button(ui, &t("local.wizard.change", &[])).clicked();
        });
        if self.cancelled {
            hint(
                ui,
                &t("local.wizard.cancelled_hint", &[]),
                theme::TEXT_SECONDARY,
            );
        }
        if change {
            self.dialog = Some(DownloadDialog::new(DialogMode::Select, Tab::Recommended));
        }
        if download {
            log(format!(
                "[ui] wizard local download requested: repo={} choice={} mtp={:?} id={}",
                plan.selection.repo, plan.selection.choice, plan.selection.mtp, plan.id
            ));
            env.local_command(Command::Download {
                repo: plan.selection.repo,
                choice: plan.selection.choice,
                mtp: plan.selection.mtp,
                auto: plan.selection.auto,
            });
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

    /// 引擎那段：說明與下載量，使用者按〔下載引擎〕才裝；安裝中顯示進度，失敗顯示原文與〔重試〕。
    fn engine_step(&self, ui: &mut Ui, env: &dyn SettingsEnv, snapshot: &Snapshot) {
        let intro = RichText::new(t("local.wizard.engine_intro", &[])).color(theme::TEXT);
        ui.add(Label::new(intro).wrap());
        let install = || {
            env.local_command(Command::InstallEngine {
                build: None,
                variant: None,
            })
        };
        match &snapshot.engine {
            EngineState::Installing {
                done,
                total,
                extracting,
                ..
            } => {
                let status = engine_text(*done, *total, *extracting, self.rates.speed(ENGINE));
                let text = t("local.wizard.engine_installing", &[("status", &status)]);
                progress(ui, &text, *done, *total);
            }
            EngineState::Failed(error) => error_with_retry(ui, error, "engine", install),
            // 移除中（設定頁剛按了刪除）：等它變成未安裝再給下載鈕
            EngineState::Removing => {}
            EngineState::Missing | EngineState::Ready(_) => {
                match (&snapshot.builds, engine_download(snapshot)) {
                    (_, Some((variant, bytes))) => {
                        let args = [("variant", variant_label(&variant)), ("size", gb(bytes))];
                        let args: Vec<_> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
                        hint(
                            ui,
                            &t("local.wizard.engine_size", &args),
                            theme::TEXT_SECONDARY,
                        );
                    }
                    // 版本清單還沒到：大小先畫佔位；拿不到清單就不列大小，照樣可以下載
                    (None, None) => {
                        skeleton(ui, 180.0, ui.text_style_height(&TextStyle::Small));
                    }
                    (Some(_), None) => {}
                }
                if primary_button(ui, &t("local.wizard.download_engine", &[])).clicked() {
                    log("[ui] wizard engine download requested");
                    install();
                }
            }
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
        match self.plan(snapshot).filter(|p| p.id == id) {
            Some(plan) => title_row(ui, &plan.title, Some(plan.rating)),
            None => {
                if let Some(m) = snapshot.models.iter().find(|m| m.id == id) {
                    title_row(ui, &m.manifest.display_name(), None);
                }
            }
        }
        match state {
            InstallState::Done(_) => {
                hint(ui, &t("local.wizard.done", &[]), theme::TEXT_SECONDARY);
                if secondary_button(ui, &t("local.wizard.change", &[])).clicked() {
                    self.dialog = Some(DownloadDialog::new(DialogMode::Select, Tab::Recommended));
                }
            }
            InstallState::Failed(error) => {
                error_with_retry(ui, error, "download", || {
                    env.local_command(Command::Resume(id.clone()))
                });
            }
            _ => {
                let download = snapshot.downloads.get(&id);
                let (done, total) = match download {
                    Some(
                        DownloadState::Running { done, total }
                        | DownloadState::Paused { done, total },
                    ) => (*done, *total),
                    _ => (0, 0),
                };
                let stopped = matches!(download, None | Some(DownloadState::Paused { .. }));
                let resumable = stopped && snapshot.models.iter().any(|m| m.id == id);
                let text = match download {
                    Some(DownloadState::Paused { .. }) => paused_text(done, total),
                    None if resumable => t("local.models.interrupted", &[]),
                    _ => transfer_text(done, total, self.rates.speed(&id)),
                };
                progress(ui, &text, done, total);
                if resumable && primary_button(ui, &t("local.continue_download", &[])).clicked() {
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

/// 模型名稱，有相容等級就接在後面；太長時換行，不超出卡片。
fn title_row(ui: &mut Ui, title: &str, rating: Option<Rating>) {
    ui.horizontal_wrapped(|ui| {
        let title = RichText::new(title).font(theme::bold(theme::TEXT_SIZE));
        ui.add(Label::new(title));
        if let Some(r) = rating {
            fit_badge(ui, r.fit, r.need.estimated(), false);
        }
    });
}

fn repo_error(error: &str) -> String {
    t("local.error.repo", &[("error", error)])
}

/// 推薦卡還在載入時的佔位：標題與三行說明。
fn loading_card(ui: &mut Ui) {
    card(ui, |ui| {
        Frame::new()
            .inner_margin(Margin::symmetric(16, 14))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                skeleton(ui, 240.0, ui.text_style_height(&TextStyle::Body));
                let small = ui.text_style_height(&TextStyle::Small);
                for width in [160.0, 200.0, 180.0] {
                    skeleton(ui, width, small);
                }
            });
    });
}
