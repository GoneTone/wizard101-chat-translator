//! 「下載模型」對話框：推薦模型與自訂 Hugging Face repo 兩個分頁；精靈用 `Select` 模式取得選擇而不下載。

use std::collections::BTreeMap;

use eframe::egui::{Frame, Label, Margin, RichText, Ui, vec2};

use super::dialog::{Choice, DialogSpec, confirm, dialog};
use super::fields::text_field;
use super::quant_table::{self, dot, fit_color, fit_text, gb};
use super::theme;
use super::widgets::{primary_button, right_aligned, secondary_button};
use crate::i18n::t;
use crate::local::catalog::{RECOMMENDED, Recommended};
use crate::local::fit::{Fit, MemoryKind, memory_kind, total_size};
use crate::local::hf::parse_repo;
use crate::local::{Command, RepoInfo, RepoState, Snapshot};
use crate::logs::log;
use crate::ui::settings::env::SettingsEnv;

/// 對話框的用途：下載，或只回傳選擇（精靈）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogMode {
    Download,
    Select,
}

/// 分頁。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Recommended,
    Custom,
}

/// 選定的模型；`choice`／`mtp` 為該 repo `Catalog` 內的索引。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub repo: String,
    pub choice: usize,
    pub mtp: Option<usize>,
}

/// 對話框這一幀的結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogOutcome {
    Open,
    Closed,
    Selected(Selection),
}

/// 下載模型對話框的狀態。
pub struct DownloadDialog {
    mode: DialogMode,
    tab: Tab,
    custom_input: String,
    expanded: Option<String>,
    picks: BTreeMap<String, (usize, Option<usize>)>,
    started: bool,
    custom_repo: Option<String>,
    bad_repo: bool,
    /// 放不下的選擇，等使用者確認要不要下載。
    nofit: Option<Selection>,
}

/// 一個按下去會生效的選擇，附該版本的相容等級。
type Chosen = (Selection, Option<Fit>);

impl DownloadDialog {
    pub fn new(mode: DialogMode, tab: Tab) -> Self {
        log(format!(
            "[ui] download dialog opened: mode={mode:?} tab={tab:?}"
        ));
        Self {
            mode,
            tab,
            custom_input: String::new(),
            expanded: None,
            picks: BTreeMap::new(),
            started: false,
            custom_repo: None,
            bad_repo: false,
            nofit: None,
        }
    }

    /// 畫一幀；回傳對話框是否還開著、被關閉，或（`Select` 模式）選了哪個。
    pub fn ui(&mut self, ui: &Ui, env: &dyn SettingsEnv, snapshot: &Snapshot) -> DialogOutcome {
        if !self.started {
            self.started = true;
            for rec in RECOMMENDED
                .iter()
                .filter(|r| !snapshot.repos.contains_key(r.repo))
            {
                env.local_command(Command::QueryRepo(rec.repo.to_owned()));
            }
        }
        let custom_ready = self.custom_choice(snapshot);
        let spec = DialogSpec {
            salt: "download_model",
            width: theme::DOWNLOAD_DIALOG_WIDTH,
            title: t(self.title_key(), &[]),
            closable: false,
            pinned_top: false,
        };
        let mode = self.mode;
        let shown = dialog(
            ui,
            spec,
            |ui| {
                Frame::new()
                    .inner_margin(Margin::symmetric(20, 4))
                    .show(ui, |ui| self.body(ui, env, snapshot))
                    .inner
            },
            |ui| {
                let leave_key = match mode {
                    DialogMode::Download => "local.dialog.close",
                    DialogMode::Select => "button.cancel",
                };
                let leave = secondary_button(ui, &t(leave_key, &[])).clicked();
                let go = custom_ready.is_some()
                    && primary_button(ui, &t(Self::custom_button_key(mode), &[])).clicked();
                (leave, go)
            },
        );
        let (leave, go) = shown.footer;
        let mut chosen = shown.body;
        if go {
            chosen = custom_ready;
        }
        if let Some(outcome) = self.confirm_nofit(ui, env) {
            return outcome;
        }
        if let Some(chosen) = chosen {
            return self.choose(env, chosen);
        }
        if leave || shown.cancel {
            log("[ui] download dialog closed");
            return DialogOutcome::Closed;
        }
        DialogOutcome::Open
    }

    fn title_key(&self) -> &'static str {
        match self.mode {
            DialogMode::Download => "local.dialog.title",
            DialogMode::Select => "local.dialog.select_title",
        }
    }

    fn custom_button_key(mode: DialogMode) -> &'static str {
        match mode {
            DialogMode::Download => "local.dialog.download_selected",
            DialogMode::Select => "local.dialog.select",
        }
    }

    /// 這個 repo 目前的選擇；沒有手動選過就用 hub 的自動挑選。
    fn pick_of(&self, repo: &str, info: &RepoInfo) -> Option<(usize, Option<usize>)> {
        let valid = |p: &(usize, Option<usize>)| {
            p.0 < info.catalog.choices.len() && p.1.is_none_or(|m| m < info.catalog.mtp.len())
        };
        self.picks
            .get(repo)
            .copied()
            .filter(valid)
            .or(info.auto.map(|a| (a.choice, a.mtp)))
    }

    fn chosen(&self, repo: &str, info: &RepoInfo) -> Option<Chosen> {
        let (choice, mtp) = self.pick_of(repo, info)?;
        let fit = info.fits.get(choice).and_then(|f| f.1);
        Some((
            Selection {
                repo: repo.to_owned(),
                choice,
                mtp,
            },
            fit,
        ))
    }

    fn custom_choice(&self, snapshot: &Snapshot) -> Option<Chosen> {
        if self.tab != Tab::Custom {
            return None;
        }
        let repo = self.custom_repo.as_deref()?;
        match snapshot.repos.get(repo) {
            Some(RepoState::Ready(info)) => self.chosen(repo, info),
            _ => None,
        }
    }

    /// 按下〔下載〕／〔選這個〕後：選擇模式回傳選擇，放不下的先確認，其餘直接下載。
    fn choose(&mut self, env: &dyn SettingsEnv, (sel, fit): Chosen) -> DialogOutcome {
        log(format!(
            "[ui] model chosen: mode={:?} repo={} choice={} mtp={:?} fit={fit:?}",
            self.mode, sel.repo, sel.choice, sel.mtp
        ));
        match self.mode {
            DialogMode::Select => DialogOutcome::Selected(sel),
            DialogMode::Download if fit == Some(Fit::NoFit) => {
                self.nofit = Some(sel);
                DialogOutcome::Open
            }
            DialogMode::Download => Self::download(env, sel),
        }
    }

    fn download(env: &dyn SettingsEnv, sel: Selection) -> DialogOutcome {
        env.local_command(Command::Download {
            repo: sel.repo,
            choice: sel.choice,
            mtp: sel.mtp,
        });
        DialogOutcome::Closed
    }

    /// 放不下的確認；有結果（確認下載）時回傳。
    fn confirm_nofit(&mut self, ui: &Ui, env: &dyn SettingsEnv) -> Option<DialogOutcome> {
        self.nofit.as_ref()?;
        let ok = |ui: &mut Ui| primary_button(ui, &t("local.dialog.download", &[]));
        match confirm(ui, t("local.dialog.nofit_confirm", &[]), ok) {
            Choice::Open => None,
            Choice::Cancel => {
                self.nofit = None;
                None
            }
            Choice::Done(()) => self.nofit.take().map(|sel| Self::download(env, sel)),
        }
    }

    fn body(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, snapshot: &Snapshot) -> Option<Chosen> {
        ui.horizontal(|ui| {
            for (tab, key) in [
                (Tab::Recommended, "local.dialog.tab_recommended"),
                (Tab::Custom, "local.dialog.tab_custom"),
            ] {
                if ui.selectable_label(self.tab == tab, t(key, &[])).clicked() && self.tab != tab {
                    log(format!("[ui] download dialog tab: {tab:?}"));
                    self.tab = tab;
                }
            }
        });
        ui.add_space(8.0);
        let chosen = match self.tab {
            Tab::Recommended => RECOMMENDED
                .iter()
                .enumerate()
                .filter_map(|(i, rec)| self.recommended_row(ui, i, rec, snapshot))
                .next(),
            Tab::Custom => {
                self.custom(ui, env, snapshot);
                None
            }
        };
        ui.add_space(12.0);
        chosen
    }

    fn recommended_row(
        &mut self,
        ui: &mut Ui,
        index: usize,
        rec: &Recommended,
        snapshot: &Snapshot,
    ) -> Option<Chosen> {
        let mut chosen = None;
        let name = RichText::new(rec.name).font(theme::bold(theme::TEXT_SIZE));
        let state = snapshot.repos.get(rec.repo);
        Frame::new()
            .inner_margin(Margin::symmetric(0, 8))
            .show(ui, |ui| match state {
                Some(RepoState::Ready(info)) => {
                    chosen = self.ready_row(ui, rec, name, info, snapshot);
                }
                Some(RepoState::Failed(error)) => {
                    ui.add(Label::new(name));
                    ui.add(Label::new(RichText::new(error).color(theme::ERROR)).wrap());
                }
                _ => {
                    ui.add(Label::new(name));
                    ui.label(
                        RichText::new(t("local.dialog.loading", &[])).color(theme::TEXT_SECONDARY),
                    );
                }
            });
        if index + 1 < RECOMMENDED.len() {
            let y = ui.cursor().top();
            ui.painter().hline(
                ui.max_rect().x_range(),
                y,
                eframe::egui::Stroke::new(1.0, theme::CARD_BORDER),
            );
        }
        chosen
    }

    fn ready_row(
        &mut self,
        ui: &mut Ui,
        rec: &Recommended,
        name: RichText,
        info: &RepoInfo,
        snapshot: &Snapshot,
    ) -> Option<Chosen> {
        let (choice, mtp) = self.pick_of(rec.repo, info)?;
        let chosen = self.chosen(rec.repo, info)?;
        let level = chosen.1;
        let required = info.fits.get(choice).map_or(0, |f| f.0)
            + mtp.map_or(0, |m| info.mtp_required.get(m).copied().unwrap_or(0));
        let kind = memory_kind(info.meta.expert_count > 0, &snapshot.hardware);
        let memory = t(memory_key(kind), &[("size", &gb(required))]);
        let c = &info.catalog.choices[choice];
        let mut version = format!("{} · {} GB", c.quant.label, gb(total_size(c)));
        if let Some(m) = mtp {
            version += &format!(" + MTP {} GB", gb(info.catalog.mtp[m].file.size));
        }
        let downloaded = self.mode == DialogMode::Download
            && snapshot
                .models
                .iter()
                .any(|m| m.manifest.repo == rec.repo && m.manifest.quant == c.quant.label);
        let open = self.expanded.as_deref() == Some(rec.repo);
        let (mut toggle, mut go) = (false, false);
        let id = ui.scope_id().with(rec.repo);
        ui.horizontal(|ui| {
            let reserved = super::widgets::right_aligned_width(ui, id);
            let width = (ui.available_width() - reserved - 12.0).max(120.0);
            ui.allocate_ui(vec2(width, 0.0), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(Label::new(name).truncate());
                    ui.add(
                        Label::new(
                            RichText::new(memory)
                                .size(theme::HINT_SIZE)
                                .color(theme::TEXT_SECONDARY),
                        )
                        .wrap(),
                    );
                    ui.label(
                        RichText::new(version)
                            .size(theme::HINT_SIZE)
                            .color(theme::TEXT_SECONDARY),
                    );
                    if downloaded {
                        ui.label(
                            RichText::new(format!("✓ {}", t("local.downloaded", &[])))
                                .size(theme::HINT_SIZE)
                                .color(theme::OK),
                        );
                    }
                });
            });
            right_aligned(ui, id, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                dot(ui, fit_color(level));
                ui.label(RichText::new(fit_text(level)).color(fit_color(level)));
                let single = info.catalog.choices.len() < 2;
                let version_key = if open {
                    "local.dialog.collapse"
                } else {
                    "local.dialog.choose_version"
                };
                ui.add_enabled_ui(!single, |ui| {
                    toggle = secondary_button(ui, &t(version_key, &[])).clicked();
                });
                let (key, action_key) = match (self.mode, downloaded) {
                    (DialogMode::Select, _) => ("local.dialog.select", false),
                    (_, true) => ("local.downloaded", true),
                    _ => ("local.dialog.download", false),
                };
                ui.add_enabled_ui(!action_key, |ui| {
                    go = primary_button(ui, &t(key, &[])).clicked();
                });
            });
        });
        if toggle {
            self.expanded = (!open).then(|| rec.repo.to_owned());
        }
        if open {
            self.table(ui, rec.repo, info, snapshot);
        }
        go.then_some(chosen)
    }

    fn table(&mut self, ui: &mut Ui, repo: &str, info: &RepoInfo, snapshot: &Snapshot) {
        let Some(mut pick) = self.pick_of(repo, info) else {
            return;
        };
        let before = pick;
        ui.add_space(6.0);
        quant_table::show(ui, info, &snapshot.hardware, &mut pick);
        if pick != before {
            self.picks.insert(repo.to_owned(), pick);
        }
    }

    fn custom(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, snapshot: &Snapshot) {
        let mut query = false;
        ui.horizontal(|ui| {
            let button = 80.0;
            let input = ui
                .allocate_ui(
                    vec2(
                        (ui.available_width() - button - 8.0).max(120.0),
                        theme::CONTROL_HEIGHT,
                    ),
                    |ui| text_field(ui, &mut self.custom_input),
                )
                .inner;
            let hint = t("local.dialog.repo_hint", &[]);
            ui.ctx()
                .accesskit_node_builder(input.id, |node| node.set_label(hint));
            let enter = input.lost_focus() && ui.input(|i| i.key_pressed(eframe::egui::Key::Enter));
            query = secondary_button(ui, &t("local.dialog.query", &[])).clicked() || enter;
        });
        if query {
            match parse_repo(&self.custom_input) {
                Some(repo) => {
                    log(format!("[ui] custom repo query: repo={repo}"));
                    self.bad_repo = false;
                    env.local_command(Command::QueryRepo(repo.clone()));
                    self.custom_repo = Some(repo);
                }
                None => {
                    log("[ui] custom repo input rejected");
                    self.bad_repo = true;
                    self.custom_repo = None;
                }
            }
        }
        ui.add_space(8.0);
        if self.bad_repo {
            ui.label(RichText::new(t("local.dialog.bad_repo", &[])).color(theme::ERROR));
            return;
        }
        let Some(repo) = self.custom_repo.clone() else {
            return;
        };
        match snapshot.repos.get(&repo) {
            Some(RepoState::Ready(info)) => {
                ui.label(RichText::new(&repo).color(theme::TEXT_SECONDARY));
                self.table(ui, &repo, info, snapshot);
            }
            Some(RepoState::Failed(error)) => {
                ui.add(Label::new(RichText::new(error).color(theme::ERROR)).wrap());
            }
            _ => {
                ui.label(
                    RichText::new(t("local.dialog.loading", &[])).color(theme::TEXT_SECONDARY),
                );
            }
        }
    }
}

fn memory_key(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Vram => "local.memory.vram",
        MemoryKind::VramPlusRam => "local.memory.vram_ram",
        MemoryKind::Ram => "local.memory.ram",
    }
}
