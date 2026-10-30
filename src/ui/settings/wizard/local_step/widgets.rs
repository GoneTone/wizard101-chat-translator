//! 一鍵安裝畫面用的小零件：預設選擇、引擎下載量、進度條與錯誤加〔重試〕。

use eframe::egui::Ui;

use crate::local::catalog::{RECOMMENDED, wizard_pick};
use crate::local::engine::{Variant, auto_variant};
use crate::local::fit::Fit;
use crate::local::{EngineState, RepoInfo, RepoState, Snapshot};
use crate::logs::log;
use crate::ui::settings::local_dialog::Selection;
use crate::ui::settings::quant_table::{combined, fraction};
use crate::ui::settings::theme;
use crate::ui::settings::widgets::{self, hint, progress_bar};

/// 六個推薦裡查好的依 `wizard_pick` 挑一個，用該 repo 的自動挑選；等級不明的排最後。
pub(super) fn default_selection(snapshot: &Snapshot) -> Option<Selection> {
    let rated: Vec<_> = RECOMMENDED
        .iter()
        .filter_map(|rec| {
            let info = ready_info(snapshot, rec.repo)?;
            let fit = info
                .auto
                .and_then(|a| combined(info, (a.choice, a.mtp)).fit)
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

/// 進度條，下方是目前的狀態文字。
pub(super) fn progress(ui: &mut Ui, text: &str, done: u64, total: u64) {
    progress_bar(ui, fraction(done, total), ui.available_width());
    hint(ui, text, theme::TEXT_SECONDARY);
}

pub(super) fn error_with_retry(ui: &mut Ui, error: &str, retry: impl FnOnce()) {
    if widgets::error_with_retry(ui, error) {
        log(format!("[ui] wizard local retry: error={error}"));
        retry();
    }
}
