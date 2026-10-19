//! 收訊狀態的彙整：多個客戶端各自回報，這裡算出 overlay 唯一一條狀態列與橫幅該顯示什麼。
//!
//! `banner_for` 決定橫幅文案（純函式）；`StatusBoard` 收各編號的回報、彙整後只在結果改變時投遞 UI 事件。

use std::sync::{Arc, Mutex};

use crate::translation::pool::{ErrorState, TranslationPool};
use crate::ui::events::{UiEvent, UiPoster};
use crate::util::lock;

/// 橫幅：（文案 key，format 參數）。
pub type Banner = (&'static str, Vec<(&'static str, String)>);

// 狀態字的優先序：要使用者動手的永久性失敗排在「連線中」之後，只在沒有任何客戶端正常時才浮上來
const STATE_PRIORITY: [&str; 5] = [
    "translating",
    "listening",
    "locating",
    "access_denied",
    "version_mismatch",
];
const PERMANENT_ISSUES: [&str; 2] = ["notice.access_denied", "notice.version_mismatch"];
const ATTACHED_STATES: [&str; 2] = ["listening", "translating"];

/// 決定該顯示哪一條錯誤橫幅；`None`＝不顯示。
///
/// 遊戲端問題優先於翻譯錯誤；`error_detail`（HTTP 狀態碼，API 說明）有說明就照實顯示，
/// 只有狀態碼就顯示狀態碼，兩者都沒有才退回固定文案。
pub fn banner_for(
    game_issue: Option<&'static str>,
    error_state: Option<ErrorState>,
    error_detail: Option<(Option<u16>, String)>,
) -> Option<Banner> {
    if let Some(issue) = game_issue {
        return Some((issue, Vec::new()));
    }
    let config = match error_state? {
        ErrorState::Config => true,
        ErrorState::Offline => false,
    };
    let Some((status, message)) = error_detail else {
        let key = if config {
            "notice.config_error"
        } else {
            "notice.offline"
        };
        return Some((key, Vec::new()));
    };
    let Some(status) = status else {
        // 沒有狀態碼的設定錯誤（如金鑰格式無效）：說明多半是英文技術細節，只顯示在地化固定文案
        return Some(match (config, message.is_empty()) {
            (true, _) => ("notice.config_error", Vec::new()),
            (false, true) => ("notice.offline", Vec::new()),
            (false, false) => ("notice.offline_detail", vec![("message", message)]),
        });
    };
    let status = status.to_string();
    Some(match (message.is_empty(), config) {
        (true, true) => ("notice.config_error_http", vec![("status", status)]),
        (true, false) => ("notice.offline_status", vec![("status", status)]),
        (false, true) => (
            "notice.config_error_detail",
            vec![("status", status), ("message", message)],
        ),
        (false, false) => (
            "notice.offline_http",
            vec![("status", status), ("message", message)],
        ),
    })
}

/// 兩條翻譯佇列任一有錯就顯示：玩家對話優先（它才是主要用途）。
fn translation_banner(
    game_issue: Option<&'static str>,
    pool: &TranslationPool,
    system_pool: &TranslationPool,
) -> Option<Banner> {
    let failing = if pool.error_state().is_some() {
        pool
    } else {
        system_pool
    };
    banner_for(game_issue, failing.error_state(), failing.error_detail())
}

/// 一個客戶端最近一次回報：狀態字 key 與遊戲端問題的文案 key（`None`＝遊戲正常）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotReport {
    /// 狀態字的 key（`status.*`）。
    pub state: &'static str,
    /// 遊戲端問題的文案 key；`None`＝遊戲正常。
    pub game_issue: Option<&'static str>,
}

/// 多個客戶端的狀態字取一個給狀態列；沒有客戶端或全部一般性失敗＝等待遊戲。
pub fn aggregate_state(states: impl IntoIterator<Item = &'static str>) -> &'static str {
    let states: Vec<_> = states.into_iter().collect();
    STATE_PRIORITY
        .into_iter()
        .find(|state| states.contains(state))
        .unwrap_or("waiting_game")
}

/// 橫幅要顯示的遊戲端問題。
///
/// 永久性問題（權限、版本）任一客戶端有就顯示；否則只要有一個客戶端掛入成功就不顯示；
/// 沒有任何客戶端、或有客戶端失敗，才顯示找不到遊戲；全部仍在掛入中則不顯示。
pub fn aggregate_game_issue(reports: &[SlotReport]) -> Option<&'static str> {
    if let Some(issue) = reports
        .iter()
        .find_map(|r| r.game_issue.filter(|i| PERMANENT_ISSUES.contains(i)))
    {
        return Some(issue);
    }
    if reports.iter().any(|r| ATTACHED_STATES.contains(&r.state)) {
        return None;
    }
    (reports.is_empty() || reports.iter().any(|r| r.game_issue.is_some()))
        .then_some("notice.game_missing")
}

#[derive(Default)]
struct Board {
    // 依首次回報的順序（同 Python dict），決定多個永久性問題時顯示哪一個
    slots: Vec<(u32, SlotReport)>,
    status: Option<&'static str>,
    banner: Option<Banner>,
}

/// 各客戶端只回報自己；這裡彙整成 overlay 的狀態字與橫幅，只在結果改變時投遞。
pub struct StatusBoard {
    ui: UiPoster,
    pool: Arc<TranslationPool>,
    system_pool: Arc<TranslationPool>,
    board: Mutex<Board>,
}

impl StatusBoard {
    /// 以 UI 投遞端與兩條翻譯池（玩家對話、系統訊息）建立。
    pub fn new(
        ui: UiPoster,
        pool: Arc<TranslationPool>,
        system_pool: Arc<TranslationPool>,
    ) -> Self {
        Self {
            ui,
            pool,
            system_pool,
            board: Mutex::default(),
        }
    }

    /// 某編號的最新狀態。
    pub fn report(&self, slot: u32, state: &'static str, game_issue: Option<&'static str>) {
        let report = SlotReport { state, game_issue };
        let mut board = self.lock();
        match board.slots.iter_mut().find(|(s, _)| *s == slot) {
            Some(entry) => entry.1 = report,
            None => board.slots.push((slot, report)),
        }
        self.push(&mut board);
    }

    /// 某編號的執行緒結束了。
    pub fn remove(&self, slot: u32) {
        let mut board = self.lock();
        board.slots.retain(|(s, _)| *s != slot);
        self.push(&mut board);
    }

    /// 回報沒變但翻譯池錯誤狀態可能變了（或還沒有任何客戶端）時重算一次。
    pub fn refresh(&self) {
        let mut board = self.lock();
        self.push(&mut board);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Board> {
        lock(&self.board)
    }

    // 持鎖投遞：多條收訊執行緒同時回報時，UI 收到的順序才與彙整結果的變化順序一致
    fn push(&self, board: &mut Board) {
        let reports: Vec<SlotReport> = board.slots.iter().map(|(_, r)| *r).collect();
        let status = aggregate_state(reports.iter().map(|r| r.state));
        let banner = translation_banner(
            aggregate_game_issue(&reports),
            &self.pool,
            &self.system_pool,
        );
        if board.status != Some(status) {
            board.status = Some(status);
            self.ui.post(UiEvent::SetStatus(status));
        }
        if board.banner != banner {
            board.banner.clone_from(&banner);
            self.ui.post(match banner {
                Some((key, args)) => UiEvent::SetError { key, args },
                None => UiEvent::ClearError,
            });
        }
    }
}
