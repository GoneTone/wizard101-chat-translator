//! ConcurrencyGate 與 TranslationPool：總量上限、平行翻譯、失敗退避與重試、resize／shutdown。

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::{Instant, timeout};
use tokio_util::sync::CancellationToken;
use wizard101_chat_translator::logs::{self, Sink};
use wizard101_chat_translator::translation::errors::TranslateError;
use wizard101_chat_translator::translation::gate::ConcurrencyGate;
use wizard101_chat_translator::translation::pool::{
    ErrorState, ResultFn, TranslateFn, TranslationPool,
};

const FAILED: &str = "⚠  這則訊息翻譯不出來";
const DESCRIBE: &str = "provider=fake-pool, model=fake-model";

type Report = (u64, String, bool);

fn tfn<F, Fut>(f: F) -> TranslateFn
where
    F: Fn(String, Vec<String>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<String, TranslateError>> + Send + 'static,
{
    Arc::new(move |line, context| Box::pin(f(line, context)))
}

fn make_pool(
    translate: TranslateFn,
    gate: Arc<ConcurrencyGate>,
) -> (TranslationPool, UnboundedReceiver<Report>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let on_result: ResultFn = Arc::new(move |id, text, failed| {
        let _ = tx.send((id, text, failed));
    });
    let pool = TranslationPool::new(
        translate,
        on_result,
        gate,
        Arc::new(|| FAILED.to_owned()),
        Arc::new(|| DESCRIBE.to_owned()),
        tokio::runtime::Handle::current(),
    );
    (pool, rx)
}

fn gate(limit: usize) -> Arc<ConcurrencyGate> {
    Arc::new(ConcurrencyGate::new(limit))
}

/// 下一則回報；等太久即失敗（暫停時鐘下逾時也會自動推進，不會真的等）。
async fn next<T>(rx: &mut UnboundedReceiver<T>) -> T {
    timeout(Duration::from_secs(3600), rx.recv())
        .await
        .expect("timed out waiting for a report")
        .expect("channel closed")
}

/// 讓其他 task 跑到下一個等待點，不推進時鐘。
async fn settle() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
}

fn secs(a: Instant, b: Instant) -> u64 {
    ((b - a).as_millis() as f64 / 1000.0).round() as u64
}

/// 每次呼叫都把時間點送出，並依 `reply` 決定這次的結果。
fn recording(
    calls: UnboundedSender<Instant>,
    reply: impl Fn(usize) -> Result<String, TranslateError> + Send + Sync + 'static,
) -> TranslateFn {
    let count = Arc::new(AtomicUsize::new(0));
    let reply = Arc::new(reply);
    tfn(move |_line, _context| {
        let n = count.fetch_add(1, Ordering::SeqCst);
        let _ = calls.send(Instant::now());
        let result = reply(n);
        async move { result }
    })
}

fn offline(detail: &str) -> TranslateError {
    TranslateError::offline(detail)
}

// ---------- ConcurrencyGate ----------

#[tokio::test(start_paused = true)]
async fn gate_allows_up_to_the_limit() {
    let gate = ConcurrencyGate::new(2);
    let stop = CancellationToken::new();
    let a = gate.acquire(&stop).await;
    let b = gate.acquire(&stop).await;
    assert!(a.is_some() && b.is_some());
    assert_eq!(gate.in_use(), 2);
    drop(a);
    assert_eq!(gate.in_use(), 1);
}

#[tokio::test(start_paused = true)]
async fn gate_blocks_beyond_the_limit_until_released() {
    let gate = gate(1);
    let stop = CancellationToken::new();
    let held = gate.acquire(&stop).await;
    let waiter = tokio::spawn({
        let (gate, stop) = (gate.clone(), stop.clone());
        async move { gate.acquire(&stop).await.is_some() }
    });
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!waiter.is_finished(), "額度已滿時不該放行");
    drop(held);
    assert!(waiter.await.unwrap(), "額度釋出後等待者要被喚醒");
}

#[tokio::test(start_paused = true)]
async fn shutdown_stops_waiting_acquires() {
    let gate = gate(1);
    let stop = CancellationToken::new();
    let _held = gate.acquire(&stop).await;
    let waiter = tokio::spawn({
        let (gate, stop) = (gate.clone(), stop.clone());
        async move { gate.acquire(&stop).await.is_none() }
    });
    settle().await;
    let started = Instant::now();
    stop.cancel();
    assert!(waiter.await.unwrap(), "關閉中應放棄等待、回傳 None");
    assert_eq!(Instant::now(), started, "要立即醒來，不等分段逾時");
    assert!(gate.acquire(&stop).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn gate_raising_the_limit_wakes_waiters() {
    let gate = gate(1);
    let stop = CancellationToken::new();
    let _held = gate.acquire(&stop).await;
    let waiter = tokio::spawn({
        let (gate, stop) = (gate.clone(), stop.clone());
        async move { gate.acquire(&stop).await.is_some() }
    });
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!waiter.is_finished());
    gate.set_limit(2);
    assert!(waiter.await.unwrap(), "上限調大後等待者要被喚醒");
}

#[tokio::test(start_paused = true)]
async fn gate_lowering_the_limit_does_not_overcommit() {
    let gate = ConcurrencyGate::new(4);
    let stop = CancellationToken::new();
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(gate.acquire(&stop).await.unwrap());
    }
    gate.set_limit(2);
    assert_eq!(gate.in_use(), 4, "已發出的額度不會被收回");
    held.truncate(1);
    assert_eq!(gate.in_use(), 1);
    let _again = gate
        .acquire(&stop)
        .await
        .expect("降到 2 之後，1 → 2 仍可再取一個");
    assert_eq!(gate.in_use(), 2);
    assert!(
        timeout(Duration::from_secs(1), gate.acquire(&stop))
            .await
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn gate_limit_is_at_least_one() {
    let gate = ConcurrencyGate::new(0);
    assert_eq!(gate.limit(), 1);
    gate.set_limit(0);
    assert_eq!(gate.limit(), 1);
}

// ---------- TranslationPool ----------

#[tokio::test(start_paused = true)]
async fn submitted_lines_are_translated_and_reported() {
    let translate = tfn(|line: String, context: Vec<String>| async move {
        Ok(format!("譯:{line}|ctx={}", context.len()))
    });
    let (pool, mut rx) = make_pool(translate, gate(4));
    for i in 0..4 {
        pool.submit(format!("[A] m{i}"), vec!["[Z] ctx".into()], i);
    }
    let mut got: Vec<Report> = Vec::new();
    for _ in 0..4 {
        got.push(next(&mut rx).await);
    }
    got.sort();
    let expected: Vec<Report> = (0..4)
        .map(|i| (i, format!("譯:[A] m{i}|ctx=1"), false))
        .collect();
    assert_eq!(got, expected);
    assert_eq!(pool.in_flight(), 0);
}

#[tokio::test(start_paused = true)]
async fn blocked_line_does_not_block_the_others() {
    let release = CancellationToken::new();
    let translate = tfn({
        let release = release.clone();
        move |line: String, _| {
            let release = release.clone();
            async move {
                if line == "[A] stuck" {
                    release.cancelled().await;
                }
                Ok(format!("譯:{line}"))
            }
        }
    });
    let (pool, mut rx) = make_pool(translate, gate(4));
    pool.submit("[A] stuck".into(), vec![], 0);
    for i in 1..4 {
        pool.submit(format!("[A] m{i}"), vec![], i);
    }
    let mut ids: Vec<u64> = Vec::new();
    for _ in 0..3 {
        ids.push(next(&mut rx).await.0);
    }
    ids.sort();
    assert_eq!(ids, [1, 2, 3], "卡住那則還沒回來，其餘三則已完成");
    release.cancel();
    assert_eq!(next(&mut rx).await, (0, "譯:[A] stuck".into(), false));
}

#[tokio::test(start_paused = true)]
async fn in_flight_counts_outstanding_work() {
    let release = CancellationToken::new();
    let (started_tx, mut started) = mpsc::unbounded_channel();
    let translate = tfn({
        let release = release.clone();
        move |_, _| {
            let (release, started_tx) = (release.clone(), started_tx.clone());
            async move {
                let _ = started_tx.send(());
                release.cancelled().await;
                Ok("譯".to_owned())
            }
        }
    });
    let (pool, mut rx) = make_pool(translate, gate(1));
    assert_eq!(pool.in_flight(), 0);
    pool.submit("[A] one".into(), vec![], 1);
    assert_eq!(pool.in_flight(), 1, "提交當下就算進行中");
    next(&mut started).await;
    assert_eq!(pool.in_flight(), 1);
    release.cancel();
    next(&mut rx).await;
    assert_eq!(pool.in_flight(), 0);
}

#[tokio::test(start_paused = true)]
async fn in_flight_already_excludes_the_line_inside_on_result() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let pool_slot: Arc<OnceLock<TranslationPool>> = Arc::new(OnceLock::new());
    let on_result: ResultFn = Arc::new({
        let (seen, pool_slot) = (seen.clone(), pool_slot.clone());
        move |_, _, _| {
            seen.lock()
                .unwrap()
                .push(pool_slot.get().unwrap().in_flight())
        }
    });
    let pool = TranslationPool::new(
        tfn(|line: String, _| async move { Ok(line) }),
        on_result,
        gate(1),
        Arc::new(|| FAILED.to_owned()),
        Arc::new(|| DESCRIBE.to_owned()),
        tokio::runtime::Handle::current(),
    );
    let _ = pool_slot.set(pool);
    pool_slot.get().unwrap().submit("[A] one".into(), vec![], 1);
    settle().await;
    assert_eq!(*seen.lock().unwrap(), [0]);
}

#[tokio::test(start_paused = true)]
async fn display_order_independent_of_completion() {
    // 第一則等其餘各則都翻完才放行：完成順序與讀取順序相反
    let others_done = Arc::new(tokio::sync::Semaphore::new(0));
    let translate = tfn({
        let others_done = others_done.clone();
        move |line: String, _| {
            let others_done = others_done.clone();
            async move {
                if line == "[A] one" {
                    others_done.acquire_many(2).await.unwrap().forget();
                } else {
                    others_done.add_permits(1);
                }
                Ok(format!("譯:{line}"))
            }
        }
    });
    let (pool, mut rx) = make_pool(translate, gate(3));
    let lines = ["[A] one", "[B] two", "[C] three"];
    let mut display: Vec<(&str, Option<String>)> = lines.iter().map(|l| (*l, None)).collect();
    for (i, line) in lines.iter().enumerate() {
        pool.submit((*line).into(), vec![], i as u64 + 1);
    }
    let mut completion = Vec::new();
    for _ in 0..3 {
        let (id, text, failed) = next(&mut rx).await;
        assert!(!failed);
        completion.push(id);
        display[id as usize - 1].1 = Some(text);
    }
    assert_eq!(completion.last(), Some(&1), "被擋住的第一則最後才完成");
    assert_eq!(
        display,
        [
            ("[A] one", Some("譯:[A] one".to_owned())),
            ("[B] two", Some("譯:[B] two".to_owned())),
            ("[C] three", Some("譯:[C] three".to_owned())),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn offline_backoff_5_15_30_then_stays_30() {
    let (calls_tx, mut calls) = mpsc::unbounded_channel();
    let translate = recording(calls_tx, |_| Err(offline("down")));
    let (pool, _rx) = make_pool(translate, gate(1));
    pool.submit("[A] one".into(), vec![], 1);
    let mut times = Vec::new();
    for _ in 0..6 {
        times.push(next(&mut calls).await);
    }
    let gaps: Vec<u64> = times.windows(2).map(|w| secs(w[0], w[1])).collect();
    assert_eq!(gaps, [5, 15, 30, 30, 30]);
    assert_eq!(pool.error_state(), Some(ErrorState::Offline));
    assert_eq!(pool.error_detail(), Some((None, "down".to_owned())));
    assert_eq!(pool.in_flight(), 1, "離線的那則留著重試，不回報");
    pool.shutdown();
}

#[tokio::test(start_paused = true)]
async fn config_error_retries_every_15s() {
    let (calls_tx, mut calls) = mpsc::unbounded_channel();
    let translate = recording(calls_tx, |_| {
        Err(TranslateError::Config {
            status: Some(401),
            detail: "Incorrect API key".into(),
        })
    });
    let (pool, _rx) = make_pool(translate, gate(1));
    pool.submit("[A] one".into(), vec![], 1);
    let mut times = Vec::new();
    for _ in 0..4 {
        times.push(next(&mut calls).await);
    }
    let gaps: Vec<u64> = times.windows(2).map(|w| secs(w[0], w[1])).collect();
    assert_eq!(gaps, [15, 15, 15]);
    assert_eq!(pool.error_state(), Some(ErrorState::Config));
    assert_eq!(
        pool.error_detail(),
        Some((Some(401), "Incorrect API key".to_owned()))
    );
    pool.shutdown();
}

#[tokio::test(start_paused = true)]
async fn success_resets_backoff() {
    // 前兩次離線、第三次成功；之後再離線一次，退避要從 5 秒重新算起
    let (calls_tx, mut calls) = mpsc::unbounded_channel();
    let translate = recording(calls_tx, |n| match n {
        0 | 1 | 3 => Err(TranslateError::Offline {
            status: Some(503),
            detail: String::new(),
        }),
        _ => Ok(format!("ok{n}")),
    });
    let (pool, mut rx) = make_pool(translate, gate(1));
    pool.submit("[A] one".into(), vec![], 1);
    let first: Vec<Instant> = vec![
        next(&mut calls).await,
        next(&mut calls).await,
        next(&mut calls).await,
    ];
    assert_eq!(secs(first[0], first[1]), 5);
    assert_eq!(secs(first[1], first[2]), 15);
    assert_eq!(next(&mut rx).await, (1, "ok2".into(), false));
    assert_eq!(pool.error_state(), None, "成功後狀態清除");
    assert_eq!(pool.error_detail(), None);

    pool.submit("[A] two".into(), vec![], 2);
    let failed_at = next(&mut calls).await;
    settle().await;
    assert_eq!(pool.error_state(), Some(ErrorState::Offline));
    assert_eq!(
        pool.error_detail(),
        Some((Some(503), String::new())),
        "API 沒說明：橫幅仍顯示狀態碼"
    );
    let retried_at = next(&mut calls).await;
    assert_eq!(secs(failed_at, retried_at), 5);
    assert_eq!(next(&mut rx).await, (2, "ok4".into(), false));
}

#[tokio::test(start_paused = true)]
async fn error_detail_is_none_without_status_or_message() {
    let (calls_tx, mut calls) = mpsc::unbounded_channel();
    let (pool, _rx) = make_pool(recording(calls_tx, |_| Err(offline(""))), gate(1));
    pool.submit("[A] one".into(), vec![], 1);
    next(&mut calls).await;
    settle().await;
    assert_eq!(pool.error_state(), Some(ErrorState::Offline));
    assert_eq!(pool.error_detail(), None);
    pool.shutdown();
}

#[tokio::test(start_paused = true)]
async fn bad_output_gives_failed_notice() {
    let (calls_tx, mut calls) = mpsc::unbounded_channel();
    let translate = recording(calls_tx, |_| {
        Err(TranslateError::BadOutput("truncated".into()))
    });
    let (pool, mut rx) = make_pool(translate, gate(1));
    pool.submit("[A] one".into(), vec![], 1);
    assert_eq!(next(&mut rx).await, (1, FAILED.into(), true));
    next(&mut calls).await;
    settle().await;
    assert!(calls.try_recv().is_err(), "截斷不重試");
    assert_eq!(pool.in_flight(), 0);
    assert_eq!(pool.error_state(), None);
}

#[tokio::test(start_paused = true)]
async fn unexpected_error_gives_failed_notice() {
    let (calls_tx, mut calls) = mpsc::unbounded_channel();
    let translate = recording(calls_tx, |_| Err(TranslateError::Unexpected("boom".into())));
    let (pool, mut rx) = make_pool(translate, gate(1));
    pool.submit("[A] one".into(), vec![], 1);
    assert_eq!(next(&mut rx).await, (1, FAILED.into(), true));
    next(&mut calls).await;
    settle().await;
    assert!(calls.try_recv().is_err());
    assert_eq!(pool.in_flight(), 0);
}

#[tokio::test(start_paused = true)]
async fn backoff_is_shared_across_workers() {
    // 伺服器離線是全域事實：同一輪的四則併發失敗只推進一層退避，下一輪也一起等。
    // barrier 逼四則每輪都真正同時在請求中，才測得到「同輪不重複推進」
    let (calls_tx, mut calls) = mpsc::unbounded_channel();
    let barrier = Arc::new(tokio::sync::Barrier::new(4));
    let translate = tfn(move |_, _| {
        let (barrier, calls_tx) = (barrier.clone(), calls_tx.clone());
        async move {
            let _ = calls_tx.send(Instant::now());
            barrier.wait().await;
            Err(offline("down"))
        }
    });
    let (pool, _rx) = make_pool(translate, gate(4));
    for i in 0..4 {
        pool.submit(format!("[A] m{i}"), vec![], i);
    }
    let mut times = Vec::new();
    for _ in 0..12 {
        times.push(next(&mut calls).await);
    }
    let t0 = times[0];
    let rounds: Vec<u64> = times.iter().map(|t| secs(t0, *t)).collect();
    assert_eq!(rounds, [0, 0, 0, 0, 5, 5, 5, 5, 20, 20, 20, 20]);
    pool.shutdown();
}

#[tokio::test(start_paused = true)]
async fn resize_changes_parallelism() {
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let release = CancellationToken::new();
    let translate = tfn({
        let (running, peak, release) = (running.clone(), peak.clone(), release.clone());
        move |line: String, _| {
            let (running, peak, release) = (running.clone(), peak.clone(), release.clone());
            async move {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                release.cancelled().await;
                running.fetch_sub(1, Ordering::SeqCst);
                Ok(line)
            }
        }
    });
    let shared = gate(1);
    let (pool, mut rx) = make_pool(translate, shared.clone());
    for i in 0..3 {
        pool.submit(format!("[A] m{i}"), vec![], i);
    }
    settle().await;
    assert_eq!(running.load(Ordering::SeqCst), 1);
    pool.resize(3);
    assert_eq!(shared.limit(), 3, "閘的上限跟著 resize 調整");
    settle().await;
    assert_eq!(
        running.load(Ordering::SeqCst),
        3,
        "調大後排隊的各則立即開跑"
    );
    release.cancel();
    for _ in 0..3 {
        next(&mut rx).await;
    }
    assert_eq!(peak.load(Ordering::SeqCst), 3);
    assert_eq!(pool.in_flight(), 0);
}

#[tokio::test(start_paused = true)]
async fn shutdown_cancels_queued_work_without_leaking_in_flight() {
    let release = CancellationToken::new();
    let (started_tx, mut started) = mpsc::unbounded_channel();
    let translate = tfn({
        let release = release.clone();
        move |_, _| {
            let (release, started_tx) = (release.clone(), started_tx.clone());
            async move {
                let _ = started_tx.send(());
                release.cancelled().await;
                Ok("譯".to_owned())
            }
        }
    });
    let (pool, mut rx) = make_pool(translate, gate(1));
    pool.submit("[A] running".into(), vec![], 1);
    next(&mut started).await;
    for i in 2..6 {
        pool.submit(format!("[A] queued{i}"), vec![], i);
    }
    settle().await;
    assert_eq!(pool.in_flight(), 5);

    let at = Instant::now();
    pool.shutdown();
    settle().await;
    assert_eq!(pool.in_flight(), 1, "只剩正在執行中的那一則");
    assert_eq!(Instant::now(), at);
    pool.submit("[A] late".into(), vec![], 9);
    assert_eq!(pool.in_flight(), 1, "關閉後不再收件");

    release.cancel();
    assert_eq!(next(&mut rx).await, (1, "譯".into(), false));
    settle().await;
    assert_eq!(pool.in_flight(), 0);
    assert!(rx.try_recv().is_err(), "被取消的各則不回報");
    assert!(started.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn shutdown_interrupts_a_backoff_wait() {
    let (calls_tx, mut calls) = mpsc::unbounded_channel();
    let (pool, _rx) = make_pool(recording(calls_tx, |_| Err(offline("down"))), gate(1));
    pool.submit("[A] one".into(), vec![], 1);
    next(&mut calls).await;
    settle().await;
    let at = Instant::now();
    pool.shutdown();
    settle().await;
    assert_eq!(pool.in_flight(), 0);
    assert_eq!(Instant::now(), at);
}

#[tokio::test(start_paused = true)]
async fn two_pools_sharing_a_gate_never_exceed_the_total_limit() {
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let translate = tfn({
        let (running, peak) = (running.clone(), peak.clone());
        move |line: String, _| {
            let (running, peak) = (running.clone(), peak.clone());
            async move {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                running.fetch_sub(1, Ordering::SeqCst);
                Ok(format!("譯:{line}"))
            }
        }
    });
    let shared = gate(2);
    let (a, mut a_rx) = make_pool(translate.clone(), shared.clone());
    let (b, mut b_rx) = make_pool(translate, shared);
    for i in 0..6 {
        a.submit(format!("a{i}"), vec![], i);
        b.submit(format!("b{i}"), vec![], i);
    }
    for _ in 0..6 {
        next(&mut a_rx).await;
        next(&mut b_rx).await;
    }
    assert_eq!(peak.load(Ordering::SeqCst), 2, "總併發不超過閘的上限");
}

#[test]
fn submit_survives_concurrent_resize() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let (pool, mut rx) = {
        let _guard = rt.enter();
        make_pool(tfn(|line: String, _| async move { Ok(line) }), gate(2))
    };
    let pool = Arc::new(pool);
    std::thread::scope(|s| {
        s.spawn(|| {
            for i in 0..2000 {
                pool.submit(format!("[A] m{i}"), vec![], i);
            }
        });
        s.spawn(|| {
            for w in [1, 3, 2, 4, 1, 2].repeat(50) {
                pool.resize(w);
            }
        });
    });
    let mut ids: Vec<u64> = rt.block_on(async {
        let mut ids = Vec::new();
        for _ in 0..2000 {
            ids.push(next(&mut rx).await.0);
        }
        ids
    });
    ids.sort();
    assert_eq!(ids, (0..2000).collect::<Vec<_>>());
    assert_eq!(pool.in_flight(), 0);
}

fn log_file() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let path = std::env::temp_dir().join(format!("pool_{}.log", std::process::id()));
        logs::init(Sink::File(std::fs::File::create(&path).unwrap()));
        path
    })
}

#[tokio::test(start_paused = true)]
async fn the_failure_log_names_the_service() {
    log_file();
    let (calls_tx, _calls) = mpsc::unbounded_channel();
    let translate = recording(calls_tx, |n| {
        if n == 0 {
            Err(offline("connection refused"))
        } else {
            Ok("ok".into())
        }
    });
    let (pool, mut rx) = make_pool(translate, gate(1));
    pool.submit("[A] log-marker".into(), vec![], 1);
    next(&mut rx).await;
    let text = std::fs::read_to_string(log_file()).unwrap();
    assert!(text.contains(&format!(
        "[translate] provider offline error ({DESCRIBE}): connection refused; retrying with backoff"
    )));
}

#[tokio::test(start_paused = true)]
async fn queued_lines_respect_backoff_set_while_waiting_for_a_slot() {
    // 額度只有 1、三則排隊：每輪只能打一次伺服器，排隊的不得在退避中插隊送出。
    // 請求先讓出一次，其餘兩則才會在第一則失敗前就通過退避檢查、排在額度上
    let (calls_tx, mut calls) = mpsc::unbounded_channel();
    let translate = tfn(move |_, _| {
        let _ = calls_tx.send(Instant::now());
        async {
            tokio::task::yield_now().await;
            Err(offline("down"))
        }
    });
    let (pool, _rx) = make_pool(translate, gate(1));
    for i in 0..3 {
        pool.submit(format!("[A] m{i}"), vec![], i);
    }
    let mut times = Vec::new();
    for _ in 0..5 {
        times.push(next(&mut calls).await);
    }
    let t0 = times[0];
    let rounds: Vec<u64> = times.iter().map(|t| secs(t0, *t)).collect();
    assert_eq!(rounds, [0, 5, 20, 50, 80]);
    pool.shutdown();
}

#[tokio::test(start_paused = true)]
async fn panicking_translate_gives_failed_notice() {
    let shared = gate(1);
    let translate = tfn(|line: String, _| async move {
        if line.contains("boom") {
            panic!("fake translate panicked");
        }
        Ok(line)
    });
    let (pool, mut rx) = make_pool(translate, shared.clone());
    pool.submit("[A] boom".into(), vec![], 1);
    assert_eq!(next(&mut rx).await, (1, FAILED.into(), true));
    assert_eq!(pool.in_flight(), 0);
    assert_eq!(shared.in_use(), 0, "額度要歸還");
    pool.submit("[A] fine".into(), vec![], 2);
    assert_eq!(next(&mut rx).await, (2, "[A] fine".into(), false));
}

#[tokio::test(start_paused = true)]
async fn dropping_the_pool_stops_retries() {
    let (calls_tx, mut calls) = mpsc::unbounded_channel();
    let (pool, _rx) = make_pool(recording(calls_tx, |_| Err(offline("down"))), gate(1));
    pool.submit("[A] one".into(), vec![], 1);
    next(&mut calls).await;
    drop(pool);
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert!(calls.try_recv().is_err(), "pool 丟棄後不該再重試");
}
