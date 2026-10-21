//! 多客戶端狀態彙整：優先序、橫幅規則（純函式），以及 StatusBoard 只在變化時推 UI。

use crate::common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::reader_fixture::{
    banners, drain, echo, failing_pool, new_pool, pool, runtime, statuses,
};
use wizard101_chat_translator::reader::status::{
    SlotReport, StatusBoard, aggregate_game_issue, aggregate_state, banner_for,
};
use wizard101_chat_translator::translation::errors::TranslateError;
use wizard101_chat_translator::translation::gate::ConcurrencyGate;
use wizard101_chat_translator::translation::pool::{ErrorState, TranslateFn};
use wizard101_chat_translator::ui::events::UiPoster;

fn report(state: &'static str, game_issue: Option<&'static str>) -> SlotReport {
    SlotReport { state, game_issue }
}

fn args(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    pairs.iter().map(|(k, v)| (*k, (*v).to_owned())).collect()
}

#[test]
fn state_priority_translating_beats_everything() {
    assert_eq!(
        aggregate_state(["listening", "translating", "locating"]),
        "translating"
    );
}

#[test]
fn state_priority_listening_beats_locating_and_failures() {
    assert_eq!(
        aggregate_state(["locating", "listening", "access_denied"]),
        "listening"
    );
}

#[test]
fn permanent_failure_shows_when_nothing_is_attached() {
    assert_eq!(
        aggregate_state(["waiting_game", "version_mismatch"]),
        "version_mismatch"
    );
}

#[test]
fn no_slots_means_waiting_for_the_game() {
    assert_eq!(aggregate_state([]), "waiting_game");
    assert_eq!(
        aggregate_state(["waiting_game", "waiting_game"]),
        "waiting_game"
    );
}

#[test]
fn permanent_issue_wins_and_attached_suppresses_missing() {
    // 權限不足要使用者動手才解得掉，不能被另一個正常的客戶端蓋掉
    let reports = [
        report("listening", None),
        report("access_denied", Some("notice.access_denied")),
    ];
    assert_eq!(aggregate_game_issue(&reports), Some("notice.access_denied"));
    // 第二個客戶端還在更新器畫面：第一個正常收訊時不跳「找不到遊戲」
    let reports = [
        report("listening", None),
        report("waiting_game", Some("notice.game_missing")),
    ];
    assert_eq!(aggregate_game_issue(&reports), None);
}

#[test]
fn all_slots_failing_reports_game_missing() {
    let reports = [report("waiting_game", Some("notice.game_missing"))];
    assert_eq!(aggregate_game_issue(&reports), Some("notice.game_missing"));
}

#[test]
fn no_slots_reports_game_missing() {
    assert_eq!(aggregate_game_issue(&[]), Some("notice.game_missing"));
}

#[test]
fn all_slots_still_locating_shows_no_banner() {
    assert_eq!(aggregate_game_issue(&[report("locating", None)]), None);
}

#[test]
fn banner_prefers_game_issue_over_translation_error() {
    let offline = Some(ErrorState::Offline);
    let config = Some(ErrorState::Config);
    assert_eq!(
        banner_for(Some("notice.game_missing"), offline, None),
        Some(("notice.game_missing", vec![]))
    );
    assert_eq!(
        banner_for(Some("notice.access_denied"), offline, None),
        Some(("notice.access_denied", vec![]))
    );
    assert_eq!(
        banner_for(None, config, None),
        Some(("notice.config_error", vec![]))
    );
    assert_eq!(
        banner_for(None, offline, None),
        Some(("notice.offline", vec![]))
    );
    assert_eq!(banner_for(None, None, None), None);
}

#[test]
fn banner_shows_the_api_message_when_there_is_one() {
    let offline = Some(ErrorState::Offline);
    let config = Some(ErrorState::Config);
    let detail = |status, message: &str| Some((status, message.to_owned()));
    assert_eq!(
        banner_for(None, config, detail(Some(401), "Incorrect API key")),
        Some((
            "notice.config_error_detail",
            args(&[("status", "401"), ("message", "Incorrect API key")])
        ))
    );
    assert_eq!(
        banner_for(None, offline, detail(Some(503), "upstream down")),
        Some((
            "notice.offline_http",
            args(&[("status", "503"), ("message", "upstream down")])
        ))
    );
    assert_eq!(
        banner_for(None, offline, detail(None, "getaddrinfo failed")),
        Some((
            "notice.offline_detail",
            args(&[("message", "getaddrinfo failed")])
        ))
    );
    // 沒有狀態碼的設定錯誤（金鑰格式無效）不可顯示「HTTP None」
    assert_eq!(
        banner_for(
            None,
            config,
            detail(None, "api key contains invalid characters")
        ),
        Some(("notice.config_error", vec![]))
    );
    // 只有狀態碼沒有說明：顯示狀態碼，不退回「伺服器離線」這類猜測的固定文案
    assert_eq!(
        banner_for(None, offline, detail(Some(429), "")),
        Some(("notice.offline_status", args(&[("status", "429")])))
    );
    assert_eq!(
        banner_for(None, config, detail(Some(400), "")),
        Some(("notice.config_error_http", args(&[("status", "400")])))
    );
    assert_eq!(
        banner_for(Some("notice.game_missing"), config, detail(Some(401), "x")),
        Some(("notice.game_missing", vec![]))
    );
}

#[test]
fn board_pushes_only_on_change() {
    let rt = runtime();
    let (ui, rx) = UiPoster::for_test();
    let board = StatusBoard::new(ui, pool(&rt, echo()).0, pool(&rt, echo()).0);
    board.report(1, "listening", None);
    board.report(1, "listening", None);
    board.report(2, "locating", None);
    assert_eq!(statuses(&drain(&rx)), ["listening"]);
}

#[test]
fn board_drop_recomputes() {
    let rt = runtime();
    let (ui, rx) = UiPoster::for_test();
    let board = StatusBoard::new(ui, pool(&rt, echo()).0, pool(&rt, echo()).0);
    board.report(1, "listening", None);
    board.report(2, "locating", None);
    board.remove(1);
    assert_eq!(statuses(&drain(&rx)), ["listening", "locating"]);
}

#[test]
fn board_banner_follows_game_issue_and_pool_errors() {
    let rt = runtime();
    let (ui, rx) = UiPoster::for_test();
    let board = StatusBoard::new(
        ui,
        failing_pool(&rt, TranslateError::offline("")),
        pool(&rt, echo()).0,
    );
    board.report(1, "waiting_game", Some("notice.game_missing"));
    board.report(1, "listening", None);
    board.refresh();
    assert_eq!(
        banners(&drain(&rx)),
        [("notice.game_missing", vec![]), ("notice.offline", vec![])]
    );
}

#[test]
fn board_banner_clears_when_the_game_comes_back() {
    let rt = runtime();
    let (ui, rx) = UiPoster::for_test();
    let board = StatusBoard::new(ui, pool(&rt, echo()).0, pool(&rt, echo()).0);
    board.report(1, "waiting_game", Some("notice.game_missing"));
    board.report(1, "listening", None);
    assert_eq!(
        banners(&drain(&rx)),
        [("notice.game_missing", vec![]), ("<clear>", vec![])]
    );
}

#[test]
fn board_shows_a_system_pool_error_when_the_player_pool_is_clean() {
    let rt = runtime();
    let (ui, rx) = UiPoster::for_test();
    let board = StatusBoard::new(
        ui,
        pool(&rt, echo()).0,
        failing_pool(&rt, TranslateError::offline("")),
    );
    board.report(1, "listening", None);
    assert_eq!(banners(&drain(&rx)), [("notice.offline", vec![])]);
}

#[test]
fn board_refresh_with_no_slots_reports_waiting_game() {
    let rt = runtime();
    let (ui, rx) = UiPoster::for_test();
    let board = StatusBoard::new(ui, pool(&rt, echo()).0, pool(&rt, echo()).0);
    board.refresh();
    let events = drain(&rx);
    assert_eq!(statuses(&events), ["waiting_game"]);
    assert_eq!(banners(&events), [("notice.game_missing", vec![])]);
}

#[tokio::test(start_paused = true)]
async fn banner_clears_after_the_pool_recovers_from_offline() {
    let calls = Arc::new(AtomicUsize::new(0));
    let translate: TranslateFn = Arc::new(move |line, _| {
        let first = calls.fetch_add(1, Ordering::SeqCst) == 0;
        Box::pin(async move {
            if first {
                Err(TranslateError::offline(""))
            } else {
                Ok(line)
            }
        })
    });
    let pool = Arc::new(new_pool(
        translate,
        Arc::new(|_, _, _| {}),
        Arc::new(ConcurrencyGate::new(1)),
        tokio::runtime::Handle::current(),
    ));
    let (ui, rx) = UiPoster::for_test();
    let board = StatusBoard::new(ui, pool.clone(), pool.clone());
    board.report(1, "listening", None);

    pool.submit("hello".into(), Vec::new(), 1);
    tokio::time::sleep(Duration::from_secs(1)).await;
    board.refresh();
    assert_eq!(pool.error_state(), Some(ErrorState::Offline));

    tokio::time::sleep(Duration::from_secs(10)).await;
    board.refresh();
    assert_eq!(pool.error_state(), None);
    assert_eq!(
        banners(&drain(&rx)),
        [("notice.offline", vec![]), ("<clear>", vec![])]
    );
    pool.shutdown();
}
