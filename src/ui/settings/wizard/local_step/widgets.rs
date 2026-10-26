//! 一鍵安裝畫面用的小零件：預設選擇、引擎下載量、提示文字、進度條與錯誤加〔重試〕。

use eframe::egui::{Label, ProgressBar, RichText, Ui};

use crate::i18n::t;
use crate::local::catalog::{RECOMMENDED, wizard_pick};
use crate::local::engine::{Variant, auto_variant};
use crate::local::fit::Fit;
use crate::local::{EngineState, RepoInfo, RepoState, Snapshot};
use crate::logs::log;
use crate::ui::settings::local_dialog::Selection;
use crate::ui::settings::quant_table::{combined, fraction};
use crate::ui::settings::theme;
use crate::ui::settings::widgets::secondary_button;

/// 六個推薦裡查好的依 `wizard_pick` 挑一個，用該 repo 的自動挑選；等級不明的排最後。
pub(super) fn default_selection(snapshot: &Snapshot) -> Option<Selection> {
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
        auto: true,
    })
}

fn ready_info<'a>(snapshot: &'a Snapshot, repo: &str) -> Option<&'a RepoInfo> {
    match snapshot.repos.get(repo) {
        Some(RepoState::Ready(info)) => Some(info),
        _ => None,
    }
}

/// 引擎還沒裝好時要下載的變體與位元組；已裝好或版本清單還沒到則 `None`。
pub(super) fn engine_download(snapshot: &Snapshot) -> Option<(Variant, u64)> {
    if matches!(snapshot.engine, EngineState::Ready(_)) {
        return None;
    }
    let Some(Ok(builds)) = &snapshot.builds else {
        return None;
    };
    let build = builds.first()?;
    let variant = auto_variant(snapshot.hardware.gpu.as_ref(), Some(build));
    let (_, assets) = build.variants.iter().find(|(v, _)| *v == variant)?;
    Some((variant, assets.iter().map(|a| a.size).sum()))
}

pub(super) fn hint(ui: &mut Ui, text: &str) {
    let text = RichText::new(text)
        .size(theme::HINT_SIZE)
        .color(theme::TEXT_SECONDARY);
    ui.add(Label::new(text).wrap());
}

pub(super) fn progress(ui: &mut Ui, text: &str, done: u64, total: u64) {
    hint(ui, text);
    ui.add(ProgressBar::new(fraction(done, total)));
}

pub(super) fn error_with_retry(ui: &mut Ui, error: &str, retry: impl FnOnce()) {
    ui.add(Label::new(RichText::new(error).color(theme::ERROR)).wrap());
    if secondary_button(ui, &t("local.dialog.retry", &[])).clicked() {
        log(format!("[ui] wizard local retry: error={error}"));
        retry();
    }
}
