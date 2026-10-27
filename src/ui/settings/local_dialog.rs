//! 「下載模型」對話框：推薦模型與自訂 Hugging Face repo 兩個分頁；精靈用 `Select` 模式取得選擇而不下載。

use std::collections::BTreeMap;

use eframe::egui::{Frame, Id, Label, Margin, RichText, TextStyle, Ui, vec2};

use super::dialog::{Choice, DialogSpec, confirm, dialog};
use super::fields::text_field_with_hint;
use super::local_rows::{Have, basis, have_of, row_buttons, row_text};
use super::quant_table::{self, combined, gb};
use super::theme;
use super::widgets::{primary_button, right_aligned, secondary_button, skeleton};
use crate::i18n::t;
use crate::local::catalog::RECOMMENDED;
use crate::local::fit::{Fit, Rating, total_size};
use crate::local::hf::{SearchHit, parse_repo};
use crate::local::store::choice_id;
use crate::local::{Command, RepoInfo, RepoState, Search, Snapshot};
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

/// 選定的模型；`choice`／`mtp` 為該 repo `Catalog` 內的索引，`auto` 表示沿用自動挑選（沒動過量化表）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub repo: String,
    pub choice: usize,
    pub mtp: Option<usize>,
    pub auto: bool,
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
    /// 目前顯示結果的搜尋關鍵字；直接輸入 repo 時為 `None`。
    searched: Option<String>,
    bad_repo: bool,
    /// 放不下的選擇，等使用者確認要不要下載。
    nofit: Option<Selection>,
}

/// 一個按下去會生效的選擇，附該版本的需求與相容等級。
type Chosen = (Selection, Rating);

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
            searched: None,
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
        let spec = DialogSpec {
            salt: "download_model",
            width: theme::DOWNLOAD_DIALOG_WIDTH,
            title: t(self.title_key(), &[]),
            closable: true,
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
                secondary_button(ui, &t(leave_key, &[])).clicked()
            },
        );
        let leave = shown.footer;
        let chosen = shown.body;
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

    /// 這個 repo 手動選過的量化（索引已不合用的不算）。
    fn manual_pick(&self, repo: &str, info: &RepoInfo) -> Option<(usize, Option<usize>)> {
        let valid = |p: &(usize, Option<usize>)| {
            p.0 < info.catalog.choices.len() && p.1.is_none_or(|m| m < info.catalog.mtp.len())
        };
        self.picks.get(repo).copied().filter(valid)
    }

    /// 這個 repo 目前的選擇；沒有手動選過就用 hub 的自動挑選。
    fn pick_of(&self, repo: &str, info: &RepoInfo) -> Option<(usize, Option<usize>)> {
        self.manual_pick(repo, info)
            .or(info.auto.map(|a| (a.choice, a.mtp)))
    }

    fn chosen(&self, repo: &str, info: &RepoInfo) -> Option<Chosen> {
        let (choice, mtp) = self.pick_of(repo, info)?;
        Some((
            Selection {
                repo: repo.to_owned(),
                choice,
                mtp,
                auto: self.manual_pick(repo, info).is_none(),
            },
            combined(info, (choice, mtp)),
        ))
    }

    /// 按下〔下載〕／〔選這個〕後：選擇模式回傳選擇，放不下的先確認，其餘直接下載。
    fn choose(&mut self, env: &dyn SettingsEnv, (sel, rating): Chosen) -> DialogOutcome {
        let fit = rating.fit;
        log(format!(
            "[ui] model chosen: mode={:?} repo={} choice={} mtp={:?} auto={} fit={fit:?}",
            self.mode, sel.repo, sel.choice, sel.mtp, sel.auto
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
            auto: sel.auto,
        });
        DialogOutcome::Closed
    }

    /// 放不下的確認；有結果（確認下載）時回傳。
    fn confirm_nofit(&mut self, ui: &Ui, env: &dyn SettingsEnv) -> Option<DialogOutcome> {
        self.nofit.as_ref()?;
        let ok = |ui: &mut Ui| primary_button(ui, &t("local.dialog.download", &[]));
        match confirm(
            ui,
            "confirm_nofit",
            t("local.dialog.nofit_confirm", &[]),
            ok,
        ) {
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
            Tab::Recommended => {
                if snapshot.detecting {
                    let height = ui.text_style_height(&TextStyle::Small);
                    skeleton(ui, 260.0, height);
                } else {
                    basis(ui, &snapshot.hardware);
                }
                let mut first = None;
                for (i, rec) in RECOMMENDED.iter().enumerate() {
                    let name = RichText::new(rec.name).font(theme::bold(theme::TEXT_SIZE));
                    let row = self.repo_row(ui, env, rec.repo, name, snapshot);
                    first = first.or(row);
                    if i + 1 < RECOMMENDED.len() {
                        separator(ui);
                    }
                }
                first
            }
            Tab::Custom => self.custom(ui, env, snapshot),
        };
        ui.add_space(12.0);
        chosen
    }

    /// 一個 repo 的列：查詢中畫佔位、失敗可重試、查好的顯示自動挑的版本與按鈕。
    fn repo_row(
        &mut self,
        ui: &mut Ui,
        env: &dyn SettingsEnv,
        repo: &str,
        name: RichText,
        snapshot: &Snapshot,
    ) -> Option<Chosen> {
        let mut chosen = None;
        let state = snapshot.repos.get(repo);
        Frame::new()
            .inner_margin(Margin::symmetric(0, 8))
            .show(ui, |ui| match state {
                // 硬體偵測中的相容等級與自動挑的量化之後會變，跟查詢中一樣先畫佔位
                Some(RepoState::Ready(info)) if !snapshot.detecting => {
                    chosen = self.ready_row(ui, repo, name, info, snapshot);
                }
                Some(RepoState::Failed(error)) => {
                    ui.add(Label::new(name));
                    ui.add(Label::new(RichText::new(error).color(theme::ERROR)).wrap());
                    if secondary_button(ui, &t("local.dialog.retry", &[])).clicked() {
                        log(format!("[ui] retry repo query: repo={repo}"));
                        env.local_command(Command::QueryRepo(repo.to_owned()));
                    }
                }
                _ => loading_repo(ui, name, ui.scope_id().with(repo)),
            });
        chosen
    }

    fn ready_row(
        &mut self,
        ui: &mut Ui,
        repo: &str,
        name: RichText,
        info: &RepoInfo,
        snapshot: &Snapshot,
    ) -> Option<Chosen> {
        let chosen = self.chosen(repo, info)?;
        let (choice, mtp) = (chosen.0.choice, chosen.0.mtp);
        let rating = chosen.1;
        let c = &info.catalog.choices[choice];
        let mut version = format!("{} · {} GB", c.quant.label, gb(total_size(c)));
        if let Some(m) = mtp {
            version += &format!(" + MTP {} GB", gb(info.catalog.mtp[m].file.size));
        }
        let have = match self.mode {
            DialogMode::Download => have_of(snapshot, &choice_id(repo, c)),
            DialogMode::Select => Have::No,
        };
        let open = self.expanded.as_deref() == Some(repo);
        let id = ui.scope_id().with(repo);
        let mut clicks = (false, false);
        ui.horizontal(|ui| {
            let reserved = super::widgets::right_aligned_width(ui, id);
            let width = (ui.available_width() - reserved - 12.0).max(120.0);
            ui.allocate_ui(vec2(width, 0.0), |ui| row_text(ui, name, &version, have));
            right_aligned(ui, id, |ui| {
                clicks = row_buttons(
                    ui,
                    rating,
                    info.catalog.choices.len() > 1,
                    open,
                    self.mode,
                    have,
                )
            });
        });
        if clicks.0 {
            self.expanded = (!open).then(|| repo.to_owned());
        }
        if open {
            self.table(ui, repo, info, snapshot);
        }
        clicks.1.then_some(chosen)
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

    /// 自訂分頁：關鍵字搜尋 Hugging Face，或直接輸入 repo 名稱／網址；點一筆結果才查它的版本。
    fn custom(
        &mut self,
        ui: &mut Ui,
        env: &dyn SettingsEnv,
        snapshot: &Snapshot,
    ) -> Option<Chosen> {
        let intro = RichText::new(t("local.dialog.search_intro", &[])).color(theme::TEXT_SECONDARY);
        ui.add(Label::new(intro).wrap());
        ui.add_space(6.0);
        let mut submit = false;
        ui.horizontal(|ui| {
            let button = 80.0;
            let hint = t("local.dialog.search_hint", &[]);
            let input = ui
                .allocate_ui(
                    vec2(
                        (ui.available_width() - button - 8.0).max(120.0),
                        theme::CONTROL_HEIGHT,
                    ),
                    |ui| text_field_with_hint(ui, &mut self.custom_input, &hint),
                )
                .inner;
            ui.ctx()
                .accesskit_node_builder(input.id, |node| node.set_label(hint));
            let enter = input.lost_focus() && ui.input(|i| i.key_pressed(eframe::egui::Key::Enter));
            submit = secondary_button(ui, &t("local.dialog.search", &[])).clicked() || enter;
        });
        if submit {
            self.submit_custom(env, snapshot);
        }
        ui.add_space(8.0);
        if self.bad_repo {
            ui.label(RichText::new(t("local.dialog.bad_repo", &[])).color(theme::ERROR));
            return None;
        }
        let Some(query) = self.searched.clone() else {
            // 直接輸入的 repo 不經搜尋，單獨畫一列
            let repo = self.custom_repo.clone()?;
            let name = RichText::new(&repo).font(theme::bold(theme::TEXT_SIZE));
            return self.repo_row(ui, env, &repo, name, snapshot);
        };
        let search = snapshot
            .search
            .clone()
            .filter(|s| s.query == query)
            .unwrap_or_default();
        match search.results.clone() {
            None => {
                for i in 0..3 {
                    loading_repo(ui, RichText::new(""), ui.scope_id().with(("search", i)));
                }
                None
            }
            Some(Err(error)) => {
                ui.add(Label::new(RichText::new(error).color(theme::ERROR)).wrap());
                if secondary_button(ui, &t("local.dialog.retry", &[])).clicked() {
                    self.submit_custom(env, snapshot);
                }
                None
            }
            Some(Ok(hits)) if hits.is_empty() => {
                let empty =
                    RichText::new(t("local.dialog.no_results", &[])).color(theme::TEXT_SECONDARY);
                ui.add(Label::new(empty).wrap());
                None
            }
            Some(Ok(hits)) => {
                let chosen = self.search_results(ui, env, &hits, snapshot);
                load_more(ui, env, &search);
                chosen
            }
        }
    }

    /// 搜尋結果：一列一個 repo；點開的那個換成完整的 repo 列。
    fn search_results(
        &mut self,
        ui: &mut Ui,
        env: &dyn SettingsEnv,
        hits: &[SearchHit],
        snapshot: &Snapshot,
    ) -> Option<Chosen> {
        let mut chosen = None;
        for (i, hit) in hits.iter().enumerate() {
            let name = RichText::new(&hit.id).font(theme::bold(theme::TEXT_SIZE));
            if self.custom_repo.as_deref() == Some(hit.id.as_str()) {
                chosen = chosen.or(self.repo_row(ui, env, &hit.id, name, snapshot));
            } else if hit_row(ui, name, hit) {
                self.open_custom(env, snapshot, hit.id.clone());
            }
            if i + 1 < hits.len() {
                separator(ui);
            }
        }
        chosen
    }

    /// 送出輸入框：含 `/` 的當 repo 名稱或網址直接查，否則當關鍵字搜尋。
    fn submit_custom(&mut self, env: &dyn SettingsEnv, snapshot: &Snapshot) {
        let input = self.custom_input.trim().to_owned();
        if input.is_empty() {
            return;
        }
        self.bad_repo = false;
        self.custom_repo = None;
        if !input.contains('/') {
            log(format!("[ui] model search: query={input:?}"));
            env.local_command(Command::SearchRepos(input.clone()));
            self.searched = Some(input);
            return;
        }
        self.searched = None;
        match parse_repo(&input) {
            Some(repo) => self.open_custom(env, snapshot, repo),
            None => {
                log("[ui] custom repo input rejected");
                self.bad_repo = true;
            }
        }
    }

    /// 展開自訂分頁的一個 repo；還沒查過（或查失敗）才查。
    fn open_custom(&mut self, env: &dyn SettingsEnv, snapshot: &Snapshot, repo: String) {
        log(format!("[ui] custom repo query: repo={repo}"));
        if !matches!(
            snapshot.repos.get(&repo),
            Some(RepoState::Ready(_) | RepoState::Loading)
        ) {
            env.local_command(Command::QueryRepo(repo.clone()));
        }
        self.custom_repo = Some(repo);
    }
}

/// 結果最下方：還有下一頁時的〔載入更多〕；載入中畫佔位，失敗時顯示原因、按鈕改為重試。
fn load_more(ui: &mut Ui, env: &dyn SettingsEnv, search: &Search) {
    if search.next.is_none() {
        return;
    }
    separator(ui);
    ui.add_space(8.0);
    if search.loading_more {
        loading_repo(ui, RichText::new(""), ui.scope_id().with("search_more"));
        return;
    }
    if let Some(error) = &search.more_error {
        ui.add(Label::new(RichText::new(error).color(theme::ERROR)).wrap());
    }
    let key = match search.more_error {
        Some(_) => "local.dialog.retry",
        None => "local.dialog.load_more",
    };
    if secondary_button(ui, &t(key, &[])).clicked() {
        log(format!(
            "[ui] model search: more results for {:?}",
            search.query
        ));
        env.local_command(Command::SearchMore);
    }
}

/// 還沒點開的搜尋結果：repo 名稱、下載數與〔查看〕；回傳是否按了〔查看〕。
fn hit_row(ui: &mut Ui, name: RichText, hit: &SearchHit) -> bool {
    let downloads = t(
        "local.dialog.downloads",
        &[("count", &grouped(hit.downloads))],
    );
    let id = ui.scope_id().with(&hit.id);
    let mut view = false;
    Frame::new()
        .inner_margin(Margin::symmetric(0, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let reserved = super::widgets::right_aligned_width(ui, id);
                let width = (ui.available_width() - reserved - 12.0).max(120.0);
                ui.allocate_ui(vec2(width, 0.0), |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.add(Label::new(name).truncate());
                        ui.label(RichText::new(downloads).color(theme::TEXT_SECONDARY));
                    })
                });
                right_aligned(ui, id, |ui| {
                    view = secondary_button(ui, &t("local.dialog.view", &[])).clicked();
                });
            });
        });
    view
}

/// 列與列之間的分隔線。
fn separator(ui: &Ui) {
    let y = ui.cursor().top();
    ui.painter().hline(
        ui.max_rect().x_range(),
        y,
        eframe::egui::Stroke::new(1.0, theme::CARD_BORDER),
    );
}

/// 千分位分隔的整數（下載數）。
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 查詢中（或硬體偵測中）的 repo 列：版面同查好的列，說明、徽章與按鈕都是佔位。
fn loading_repo(ui: &mut Ui, name: RichText, id: Id) {
    ui.horizontal(|ui| {
        let reserved = super::widgets::right_aligned_width(ui, id);
        let width = (ui.available_width() - reserved - 12.0).max(120.0);
        ui.allocate_ui(vec2(width, 0.0), |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.add(Label::new(name).truncate());
                loading_row(ui);
            })
        });
        right_aligned(ui, id, |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let button = ui.spacing().interact_size.y;
            skeleton(ui, 64.0, ui.text_style_height(&TextStyle::Body));
            skeleton(ui, 88.0, button);
            skeleton(ui, 64.0, button);
        });
    });
}

/// 查詢中（或硬體偵測中）的說明或量化表：兩行佔位。
fn loading_row(ui: &mut Ui) {
    let height = ui.text_style_height(&TextStyle::Small);
    skeleton(ui, 220.0, height);
    skeleton(ui, 140.0, height);
}
