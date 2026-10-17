//! 收訊端的 supervisor：定期列舉遊戲視窗，每個視窗配一個編號並起一條收訊執行緒，視窗消失就釋出編號。
//!
//! 收訊執行緒怎麼跑由 `spawn(hwnd, slot)` 決定（呼叫端的閉包帶著共用物件），這裡只管生命週期；
//! `spawn` 回 `None`（起不了執行緒）時不佔編號，下一輪列舉再試。

use std::panic::{self, AssertUnwindSafe};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::run_loop::wait;
use super::status::StatusBoard;
use crate::logs::log;
use crate::util::{Clock, panic_message};

/// 列舉遊戲視窗的間隔。
pub const SCAN_INTERVAL: Duration = Duration::from_secs(5);
/// 關閉時等所有收訊執行緒 unhook 的總預算（全體共用，不是每條各等）。
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(8);
/// 等執行緒結束時的輪詢間隔（std 的 join 沒有逾時）。
const JOIN_POLL: Duration = Duration::from_millis(10);

/// 最小的未使用正整數：客戶端關掉後編號釋出，下一個新視窗補進來。
pub fn allocate_slot(used: impl IntoIterator<Item = u32>) -> u32 {
    let taken: Vec<u32> = used.into_iter().collect();
    (1..).find(|slot| !taken.contains(slot)).unwrap_or(1)
}

/// supervisor 執行緒的進入點；stop 被設定後等所有收訊執行緒結束再返回。
pub fn supervise(
    stop: &CancellationToken,
    spawn: impl FnMut(isize, u32) -> Option<JoinHandle<()>>,
    windows: impl FnMut() -> Vec<isize>,
    board: &StatusBoard,
) {
    supervise_with(stop, spawn, windows, board, &Clock::default());
}

/// 同 [`supervise`]，時鐘由呼叫端提供。
pub fn supervise_with(
    stop: &CancellationToken,
    mut spawn: impl FnMut(isize, u32) -> Option<JoinHandle<()>>,
    mut windows: impl FnMut() -> Vec<isize>,
    board: &StatusBoard,
    clock: &Clock,
) {
    let mut active: Vec<(isize, u32, JoinHandle<()>)> = Vec::new();
    while !stop.is_cancelled() {
        // 列舉或起執行緒失敗不可讓整個收訊端死掉
        let scan = panic::catch_unwind(AssertUnwindSafe(|| {
            let (finished, running): (Vec<_>, Vec<_>) =
                active.drain(..).partition(|(_, _, t)| t.is_finished());
            active = running;
            for (hwnd, slot, thread) in finished {
                let _ = thread.join();
                log(format!(
                    "[reader] reader thread exited hwnd={hwnd:#x} slot={slot} freed"
                ));
            }
            for hwnd in windows() {
                if active.iter().any(|(h, _, _)| *h == hwnd) {
                    continue;
                }
                let slot = allocate_slot(active.iter().map(|(_, s, _)| *s));
                log(format!(
                    "[reader] client window appeared hwnd={hwnd:#x} slot={slot}"
                ));
                if let Some(thread) = spawn(hwnd, slot) {
                    active.push((hwnd, slot, thread));
                }
            }
            board.refresh();
        }));
        if let Err(payload) = scan {
            log(format!(
                "[reader] supervisor scan failed: {}",
                panic_message(payload.as_ref())
            ));
        }
        wait(clock, stop, SCAN_INTERVAL);
    }
    let deadline = (clock.now)() + JOIN_TIMEOUT;
    for (hwnd, slot, thread) in active {
        loop {
            if thread.is_finished() {
                let _ = thread.join();
                break;
            }
            let remaining = deadline.saturating_duration_since((clock.now)());
            if remaining.is_zero() {
                log(format!(
                    "[reader] reader thread still alive after shutdown budget (slot={slot}, hwnd={hwnd:#x}); hooks may be left for next-launch repair"
                ));
                break;
            }
            (clock.sleep)(remaining.min(JOIN_POLL));
        }
    }
}
