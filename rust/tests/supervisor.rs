//! supervisor：遊戲視窗列舉 → 編號配置 → 起／收 reader 執行緒；以假列舉與虛擬時鐘驗證，不需遊戲。

#[allow(dead_code)]
mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use common::reader_fixture::{
    EXPECTED_PANIC, VirtualClock, capture_logs, drain, echo, log_lines_with, pool, quiet_panics,
    runtime, statuses,
};
use tokio_util::sync::CancellationToken;
use wizard101_chat_translator::reader::status::StatusBoard;
use wizard101_chat_translator::reader::supervisor::{
    JOIN_TIMEOUT, SCAN_INTERVAL, allocate_slot, supervise_with,
};
use wizard101_chat_translator::ui::events::{UiEvent, UiPoster};
use wizard101_chat_translator::util::Clock;

fn board() -> (
    StatusBoard,
    mpsc::Receiver<UiEvent>,
    tokio::runtime::Runtime,
) {
    capture_logs();
    let rt = runtime();
    let (ui, rx) = UiPoster::for_test();
    let board = StatusBoard::new(ui, pool(&rt, echo()).0, pool(&rt, echo()).0);
    (board, rx, rt)
}

/// 腳本化的視窗列舉：每輪推進一步；跑完設 stop 並放行所有 reader。
///
/// 某個視窗從列舉消失後，等 supervisor 記下它被收回（`freed` log）才推進下一步，
/// 編號重用的斷言才不會受執行緒結束的時機影響。
struct Harness {
    script: Vec<Vec<isize>>,
    stop: CancellationToken,
    spawned: Arc<Mutex<Vec<(isize, u32)>>>,
    gone: Arc<Mutex<HashMap<isize, Sender<()>>>>,
}

impl Harness {
    fn new(script: Vec<Vec<isize>>) -> Self {
        Self {
            script,
            stop: CancellationToken::new(),
            spawned: Arc::default(),
            gone: Arc::default(),
        }
    }

    fn run(self, clock: &Clock) -> (Vec<(isize, u32)>, Vec<UiEvent>) {
        let (board, rx, _rt) = board();
        let (spawned, gone) = (Arc::clone(&self.spawned), Arc::clone(&self.gone));
        let spawn = move |hwnd: isize, slot: u32| -> Option<JoinHandle<()>> {
            spawned.lock().unwrap().push((hwnd, slot));
            let (tx, rx) = mpsc::channel::<()>();
            gone.lock().unwrap().insert(hwnd, tx);
            Some(std::thread::spawn(move || {
                let _ = rx.recv();
            }))
        };
        let mut script = self.script.into_iter();
        let mut current: Vec<isize> = Vec::new();
        let mut awaiting: Vec<isize> = Vec::new();
        let (stop, gone) = (self.stop.clone(), Arc::clone(&self.gone));
        let windows = move || {
            awaiting.retain(|hwnd| {
                log_lines_with(&format!("reader thread exited hwnd={hwnd:#x} ")).is_empty()
            });
            if !awaiting.is_empty() {
                return current.clone();
            }
            let Some(next) = script.next() else {
                stop.cancel();
                gone.lock().unwrap().clear();
                return current.clone();
            };
            for hwnd in current.iter().filter(|h| !next.contains(h)) {
                gone.lock().unwrap().remove(hwnd);
                awaiting.push(*hwnd);
            }
            current = next;
            current.clone()
        };
        supervise_with(&self.stop, spawn, windows, &board, clock);
        self.gone.lock().unwrap().clear();
        let spawned = self.spawned.lock().unwrap().clone();
        (spawned, drain(&rx))
    }
}

fn fast_clock() -> Clock {
    VirtualClock::new().clock(Duration::from_millis(1))
}

#[test]
fn allocate_slot_takes_the_smallest_free_number() {
    assert_eq!(allocate_slot([]), 1);
    assert_eq!(allocate_slot([1]), 2);
    assert_eq!(allocate_slot([2]), 1);
    assert_eq!(allocate_slot([1, 2, 4]), 3);
}

#[test]
fn each_new_window_gets_a_thread_and_the_next_slot() {
    let (spawned, _) = Harness::new(vec![vec![0xA1], vec![0xA1, 0xB1]]).run(&fast_clock());
    assert_eq!(spawned, [(0xA1, 1), (0xB1, 2)]);
}

#[test]
fn smallest_free_slot_reused() {
    // A 關掉後 C 才開：C 拿回編號 1
    let script = vec![vec![0xA2, 0xB2], vec![0xB2], vec![0xB2, 0xC2]];
    let (spawned, _) = Harness::new(script).run(&fast_clock());
    assert_eq!(spawned, [(0xA2, 1), (0xB2, 2), (0xC2, 1)]);
    assert_eq!(
        log_lines_with("[reader] reader thread exited hwnd=0xa2 slot=1 freed").len(),
        1
    );
    assert_eq!(
        log_lines_with("[reader] client window appeared hwnd=0xc2 slot=1").len(),
        1
    );
}

#[test]
fn board_is_refreshed_every_scan() {
    let (_, events) = Harness::new(vec![vec![], vec![]]).run(&fast_clock());
    // 沒有任何客戶端時第一次 refresh 就推出「等待遊戲」與「找不到遊戲」
    assert_eq!(statuses(&events), ["waiting_game"]);
}

#[test]
fn scans_are_spaced_by_the_scan_interval() {
    let virtual_clock = VirtualClock::new();
    let times = Arc::new(Mutex::new(Vec::new()));
    let stop = CancellationToken::new();
    let (board, _rx, _rt) = board();
    let (seen, at, halt) = (Arc::clone(&times), virtual_clock.clone(), stop.clone());
    let windows = move || {
        let mut seen = seen.lock().unwrap();
        seen.push(at.elapsed());
        if seen.len() == 3 {
            halt.cancel();
        }
        Vec::new()
    };
    supervise_with(
        &stop,
        |_, _| unreachable!(),
        windows,
        &board,
        &virtual_clock.clock(Duration::ZERO),
    );
    assert_eq!(
        *times.lock().unwrap(),
        [Duration::ZERO, SCAN_INTERVAL, SCAN_INTERVAL * 2]
    );
}

#[test]
fn stop_joins_every_reader_thread() {
    // 關閉程式：兩條 reader 都要跑完（unhook）supervisor 才能返回
    let stop = CancellationToken::new();
    let joined = Arc::new(Mutex::new(Vec::new()));
    let (board, _rx, _rt) = board();
    let (token, done) = (stop.clone(), Arc::clone(&joined));
    let spawn = move |hwnd: isize, _slot: u32| {
        let (token, done) = (token.clone(), Arc::clone(&done));
        Some(std::thread::spawn(move || {
            while !token.is_cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            std::thread::sleep(Duration::from_millis(20));
            done.lock().unwrap().push(hwnd);
        }))
    };
    let halt = stop.clone();
    let windows = move || {
        halt.cancel();
        vec![0xA3, 0xB3]
    };
    supervise_with(&stop, spawn, windows, &board, &Clock::default());
    let mut joined = joined.lock().unwrap().clone();
    joined.sort_unstable();
    assert_eq!(joined, [0xA3, 0xB3]);
}

#[test]
fn scan_failure_does_not_kill_the_loop() {
    quiet_panics();
    let stop = CancellationToken::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let (board, _rx, _rt) = board();
    let (count, halt) = (Arc::clone(&calls), stop.clone());
    let windows = move || {
        if count.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("{EXPECTED_PANIC} EnumWindows exploded");
        }
        halt.cancel();
        Vec::new()
    };
    supervise_with(&stop, |_, _| unreachable!(), windows, &board, &fast_clock());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let failed = log_lines_with("[reader] supervisor scan failed: ");
    assert!(
        failed.iter().any(|l| l.contains("EnumWindows exploded")),
        "{failed:?}"
    );
}

#[test]
fn failed_spawn_frees_the_slot_and_retries_next_scan() {
    let stop = CancellationToken::new();
    let (board, _rx, _rt) = board();
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let (seen, halt) = (Arc::clone(&attempts), stop.clone());
    let spawn = move |hwnd: isize, slot: u32| {
        let mut seen = seen.lock().unwrap();
        seen.push((hwnd, slot));
        if seen.len() == 2 {
            halt.cancel();
            return Some(std::thread::spawn(|| {}));
        }
        None
    };
    supervise_with(&stop, spawn, || vec![0xA6], &board, &fast_clock());
    assert_eq!(*attempts.lock().unwrap(), [(0xA6, 1), (0xA6, 1)]);
}

#[test]
fn join_budget_shared_8s() {
    // 兩條卡死的 reader 共用一個預算，不是各等 JOIN_TIMEOUT
    let virtual_clock = VirtualClock::new();
    let stop = CancellationToken::new();
    let (board, _rx, _rt) = board();
    let release: Arc<Mutex<Vec<Sender<()>>>> = Arc::default();
    let senders = Arc::clone(&release);
    let spawn = move |_hwnd: isize, _slot: u32| {
        let (tx, rx) = mpsc::channel::<()>();
        senders.lock().unwrap().push(tx);
        Some(std::thread::spawn(move || {
            let _ = rx.recv();
        }))
    };
    let halt = stop.clone();
    let windows = move || {
        halt.cancel();
        vec![0xA4, 0xB4]
    };
    let started = Instant::now();
    supervise_with(
        &stop,
        spawn,
        windows,
        &board,
        &virtual_clock.clock(Duration::ZERO),
    );
    release.lock().unwrap().clear();
    assert_eq!(virtual_clock.elapsed(), JOIN_TIMEOUT);
    assert!(started.elapsed() < Duration::from_secs(5));
    for (slot, hwnd) in [(1, "0xa4"), (2, "0xb4")] {
        let line = format!(
            "[reader] reader thread still alive after shutdown budget (slot={slot}, hwnd={hwnd}); hooks may be left for next-launch repair"
        );
        assert_eq!(log_lines_with(&line).len(), 1, "{line}");
    }
}
