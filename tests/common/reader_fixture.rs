//! 收訊迴圈／supervisor／狀態彙整測試共用：翻譯池、虛擬時鐘與預期中的 panic。
#![allow(dead_code)]

use std::panic;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use tokio::runtime::Runtime;
use wizard101_chat_translator::translation::errors::TranslateError;
use wizard101_chat_translator::translation::gate::ConcurrencyGate;
use wizard101_chat_translator::translation::pool::{ResultFn, TranslateFn, TranslationPool};
use wizard101_chat_translator::ui::events::UiEvent;
use wizard101_chat_translator::util::Clock;

/// 測試刻意觸發的 panic 訊息帶這個標記，`quiet_panics` 不印出它。
pub const EXPECTED_PANIC: &str = "[expected test panic]";

/// 翻譯池跑在這個 runtime 上。
pub fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

/// 假翻譯池失敗時顯示的文字。
pub const FAILED: &str = "⚠  這則訊息翻譯不出來";

/// 假翻譯池描述自己的 log 片段。
pub const DESCRIBE: &str = "provider=fake-pool, model=fake-model";

/// 以固定的失敗文字與描述組一個翻譯池，結果交給 `on_result`；`handle` 是跑它的 runtime。
pub fn new_pool(
    translate: TranslateFn,
    on_result: ResultFn,
    gate: Arc<ConcurrencyGate>,
    handle: tokio::runtime::Handle,
) -> TranslationPool {
    TranslationPool::new(
        translate,
        on_result,
        gate,
        Arc::new(|| FAILED.to_owned()),
        Arc::new(|| DESCRIBE.to_owned()),
        handle,
    )
}

/// 以 `translate` 建一個翻譯池，結果送進回傳的 channel。
pub fn pool(
    rt: &Runtime,
    translate: TranslateFn,
) -> (Arc<TranslationPool>, Receiver<(u64, String, bool)>) {
    let (tx, rx) = std::sync::mpsc::channel::<(u64, String, bool)>();
    let tx: Mutex<Sender<_>> = Mutex::new(tx);
    let on_result: ResultFn = Arc::new(move |id, text, failed| {
        let _ = tx.lock().unwrap().send((id, text, failed));
    });
    let pool = new_pool(
        translate,
        on_result,
        Arc::new(ConcurrencyGate::new(4)),
        rt.handle().clone(),
    );
    (Arc::new(pool), rx)
}

/// 譯文＝原文＋上下文，結果就能還原每則提交了什麼。
pub fn echo() -> TranslateFn {
    Arc::new(|line, context| {
        Box::pin(async move { Ok(format!("{line}\u{1}{}", context.join("\u{2}"))) })
    })
}

/// 收齊 `n` 則結果，依 msg_id 排序後還原成（原文, 上下文, msg_id）。
pub fn submitted(rx: &Receiver<(u64, String, bool)>, n: usize) -> Vec<(String, Vec<String>, u64)> {
    let mut got: Vec<_> = (0..n)
        .map(|_| rx.recv_timeout(super::WAIT).expect("pool result"))
        .collect();
    assert!(
        rx.recv_timeout(Duration::from_millis(50)).is_err(),
        "more submissions than {n}"
    );
    got.sort_by_key(|(id, _, _)| *id);
    got.into_iter()
        .map(|(id, text, _)| {
            let (line, ctx) = text.split_once('\u{1}').unwrap();
            let ctx = if ctx.is_empty() {
                Vec::new()
            } else {
                ctx.split('\u{2}').map(str::to_owned).collect()
            };
            (line.to_owned(), ctx, id)
        })
        .collect()
}

/// 永遠以 `err` 失敗的翻譯池，回傳前已進入錯誤狀態（佔著一則在重試）。
pub fn failing_pool(rt: &Runtime, err: TranslateError) -> Arc<TranslationPool> {
    let (pool, _) = pool(
        rt,
        Arc::new(move |_, _| {
            let err = err.clone();
            Box::pin(async move { Err(err) })
        }),
    );
    pool.submit("prime".into(), Vec::new(), 0);
    super::wait_until("the pool to enter an error state", || {
        pool.error_state().is_some()
    });
    pool
}

/// 虛擬時鐘：`sleep` 只推進虛擬時間（另可真的睡 `real` 讓其他執行緒前進）。
#[derive(Clone)]
pub struct VirtualClock {
    base: Instant,
    offset: Arc<Mutex<Duration>>,
}

impl VirtualClock {
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            offset: Arc::default(),
        }
    }

    pub fn now(&self) -> Instant {
        self.base + *self.offset.lock().unwrap()
    }

    pub fn elapsed(&self) -> Duration {
        *self.offset.lock().unwrap()
    }

    pub fn clock(&self, real: Duration) -> Clock {
        let (now, sleep) = (self.clone(), self.clone());
        Clock {
            now: Box::new(move || now.now()),
            sleep: Box::new(move |d| {
                *sleep.offset.lock().unwrap() += d;
                if !real.is_zero() {
                    std::thread::sleep(real);
                }
            }),
        }
    }
}

/// 不印出帶 `EXPECTED_PANIC` 的 panic，其餘照舊交給原本的 hook。
pub fn quiet_panics() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let payload = info.payload();
            let text = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            if !text.contains(EXPECTED_PANIC) {
                previous(info);
            }
        }));
    });
}

/// 取出目前排隊的所有 UI 事件。
pub fn drain(rx: &Receiver<UiEvent>) -> Vec<UiEvent> {
    rx.try_iter().collect()
}

pub fn statuses(events: &[UiEvent]) -> Vec<&'static str> {
    events
        .iter()
        .filter_map(|e| match e {
            UiEvent::SetStatus(s) => Some(*s),
            _ => None,
        })
        .collect()
}

/// 橫幅事件：設定為（key, 參數），清除為 `("<clear>", [])`。
pub fn banners(events: &[UiEvent]) -> Vec<(&'static str, Vec<(&'static str, String)>)> {
    events
        .iter()
        .filter_map(|e| match e {
            UiEvent::SetError { key, args } => Some((*key, args.clone())),
            UiEvent::ClearError => Some(("<clear>", Vec::new())),
            _ => None,
        })
        .collect()
}
