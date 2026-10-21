//! 收訊翻譯工作池：每則一個 task 平行翻譯，單則卡住不影響其他則。
//!
//! 呼叫端只負責提交（line, context, msg_id）與接收 `on_result(msg_id, text, failed)`；
//! 顯示順序不由完成順序決定 —— overlay 在提交當下就已佔好位置。

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::FutureExt;
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::BoxFuture;
use super::errors::{MAX_RETRY_AFTER, TranslateError};
use super::gate::ConcurrencyGate;
use crate::logs::log;
use crate::util::{lock, panic_message};

/// 一次翻譯請求：`(line, context)` → 譯文。
pub type TranslateFn = Arc<
    dyn Fn(String, Vec<String>) -> BoxFuture<'static, Result<String, TranslateError>> + Send + Sync,
>;
/// 結果回報 `(msg_id, text, failed)`；`failed` 時 `text` 是失敗提示而非譯文。
pub type ResultFn = Arc<dyn Fn(u64, String, bool) + Send + Sync>;
/// 回傳字串的 callable（失敗提示、服務描述），每次取用時才求值。
pub type TextFn = Arc<dyn Fn() -> String + Send + Sync>;

/// 翻譯伺服器離線時的重試間隔（秒），用完停在最後一階；伺服器要求等更久時照它（上限 [`MAX_RETRY_AFTER`]）。
pub const BACKOFF_STEPS: [u64; 3] = [5, 15, 30];
/// API 設定錯誤時的重試間隔（秒）；使用者修正後自動恢復。
pub const CONFIG_ERROR_INTERVAL: u64 = 15;
/// 單則最久重試多久；超過就放棄，免得久遠的舊訊息在恢復後才補翻。
const MAX_JOB_AGE: Duration = Duration::from_secs(5 * 60);
/// Config 錯誤重試前的禮讓時間：同一刻醒來的其他則先取額度，壞掉的那則才不會獨占並餓死它們。
const CONFIG_RETRY_YIELD: Duration = Duration::from_millis(100);

/// 翻譯錯誤狀態，供上層決定錯誤橫幅。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorState {
    /// 伺服器連不上或暫時無法服務（連線失敗、逾時、429、5xx）。
    Offline,
    /// API 設定錯誤（4xx），等使用者修正。
    Config,
}

impl ErrorState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Config => "config",
        }
    }
}

#[derive(Default)]
struct State {
    workers: usize,
    in_flight: usize,
    error_state: Option<ErrorState>,
    error_detail: Option<(Option<u16>, String)>,
    backoff_index: usize,
    /// 這之前不得送出新請求；全域退避閘門。
    gate_until: Option<Instant>,
    /// 累計成功則數；Config 錯誤期間它有變，代表設定沒問題、錯在這則本身。
    successes: u64,
    /// 設定重建次數；舊設定送出的請求回 Config 錯誤時，別拿新設定下的成功當證據。
    config_epoch: u64,
}

struct Inner {
    translate: TranslateFn,
    on_result: ResultFn,
    gate: Arc<ConcurrencyGate>,
    failed_notice: TextFn,
    describe: TextFn,
    stop: CancellationToken,
    state: Mutex<State>,
    /// 別則成功、提早放開退避閘門時喚醒等待者。
    gate_opened: Notify,
}

/// 平行收訊翻譯；`on_result` 於 runtime 的 task 中呼叫，呼叫端負責轉交回 UI 執行緒。
pub struct TranslationPool {
    inner: Arc<Inner>,
    rt: Handle,
}

impl TranslationPool {
    /// 建立翻譯池；平行度即共用閘 `gate` 的上限。
    ///
    /// `failed_notice` 取 callable 而非字串：介面語言可能在執行中被改掉。
    /// `describe` 指名目前是哪一組服務，失敗 log 用。
    pub fn new(
        translate: TranslateFn,
        on_result: ResultFn,
        gate: Arc<ConcurrencyGate>,
        failed_notice: TextFn,
        describe: TextFn,
        rt: Handle,
    ) -> Self {
        let state = State {
            workers: gate.limit(),
            ..State::default()
        };
        Self {
            inner: Arc::new(Inner {
                translate,
                on_result,
                gate,
                failed_notice,
                describe,
                stop: CancellationToken::new(),
                state: Mutex::new(state),
                gate_opened: Notify::new(),
            }),
            rt,
        }
    }

    /// 提交一則翻譯；可在任何執行緒呼叫。context 為提交當下的快照，重試時沿用同一份。
    pub fn submit(&self, line: String, context: Vec<String>, msg_id: u64) {
        if self.inner.stop.is_cancelled() {
            return;
        }
        self.inner.state().in_flight += 1;
        let job = Job {
            inner: Arc::clone(&self.inner),
            pending: true,
            born: Instant::now(),
        };
        self.rt.spawn(job.run(line, context, msg_id));
    }

    /// 已提交但尚未回報結果的則數（供狀態指示判斷是否顯示「翻譯中…」）。
    pub fn in_flight(&self) -> usize {
        self.inner.state().in_flight
    }

    /// 目前的翻譯錯誤狀態；`None` 表示正常。
    pub fn error_state(&self) -> Option<ErrorState> {
        self.inner.state().error_state
    }

    /// 最近一次失敗的（HTTP 狀態碼，API 說明）；沒在失敗，或兩者都沒有時為 `None`。
    pub fn error_detail(&self) -> Option<(Option<u16>, String)> {
        self.inner.state().error_detail.clone()
    }

    /// 變更平行度（調整共用閘的上限）；進行中的各則照常回報。
    pub fn resize(&self, workers: usize) {
        {
            let mut state = self.inner.state();
            if state.workers == workers {
                return;
            }
            state.workers = workers;
        }
        self.inner.gate.set_limit(workers);
        log(format!("[translate] pool resized to {workers} workers"));
    }

    /// 通知翻譯設定已重建：之前送出的請求若回 Config 錯誤，改為重試而非判定為該則本身的問題。
    pub fn note_reconfigured(&self) {
        self.inner.state().config_epoch += 1;
    }

    /// 停止接受新工作；等待中的各則立即放棄（不回報），已送出的請求跑完照常回報。
    pub fn shutdown(&self) {
        self.inner.stop.cancel();
    }
}

impl Drop for TranslationPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// 退避閘門還要多久才開；已開為 `None`。
    fn backoff_remaining(&self) -> Option<Duration> {
        let now = Instant::now();
        self.state()
            .gate_until
            .filter(|&until| until > now)
            .map(|until| until - now)
    }

    /// 等到退避閘門開啟；關閉中回傳 `false`。
    async fn wait_for_backoff(&self) -> bool {
        while !self.stop.is_cancelled() {
            // 先建好再看閘門：notify_waiters 從建立當下就算數，檢查與入睡之間的放開才不會漏接
            let opened = self.gate_opened.notified();
            let Some(remaining) = self.backoff_remaining() else {
                return true;
            };
            tokio::select! {
                _ = self.stop.cancelled() => return false,
                _ = opened => {}
                _ = tokio::time::sleep(remaining) => {}
            }
        }
        false
    }

    /// 推進全域退避閘門並記錄狀態。閘門未到期代表同一輪已有別則推進過，不再推進階數 ——
    /// 否則 N 則同時失敗會把退避一口氣推 N 階；伺服器要求等更久時只延長閘門。
    fn note_failure(&self, kind: ErrorState, err: &TranslateError) {
        let detail = match err {
            TranslateError::Offline { detail, .. } | TranslateError::Config { detail, .. } => {
                detail.clone()
            }
            _ => String::new(),
        };
        let asked = match err {
            TranslateError::Offline { retry_after, .. } => *retry_after,
            _ => None,
        };
        let mut honoured = None;
        let changed = {
            let mut state = self.state();
            let now = Instant::now();
            let asked = asked.map(|wait| wait.min(MAX_RETRY_AFTER));
            match state.gate_until.filter(|&until| now < until) {
                None => {
                    let delay = match kind {
                        ErrorState::Config => Duration::from_secs(CONFIG_ERROR_INTERVAL),
                        ErrorState::Offline => {
                            let index = state.backoff_index.min(BACKOFF_STEPS.len() - 1);
                            let step = Duration::from_secs(BACKOFF_STEPS[index]);
                            state.backoff_index += 1;
                            honoured = asked.filter(|wait| *wait > step);
                            honoured.unwrap_or(step)
                        }
                    };
                    state.gate_until = Some(now + delay);
                }
                Some(until) => {
                    honoured = asked.filter(|wait| now + *wait > until);
                    if let Some(wait) = honoured {
                        state.gate_until = Some(now + wait);
                    }
                }
            }
            let changed = state.error_state != Some(kind);
            state.error_state = Some(kind);
            let status = err.status();
            state.error_detail =
                (status.is_some() || !detail.is_empty()).then_some((status, detail));
            changed
        };
        if changed {
            log(format!(
                "[translate] provider {} error ({}): {err}; retrying with backoff",
                kind.as_str(),
                (self.describe)()
            ));
        }
        if let (Some(asked), Some(wait)) = (asked, honoured) {
            log(format!(
                "[translate] server asked to wait {}s; next retry in {}s ({})",
                asked.as_secs_f64(),
                wait.as_secs_f64(),
                (self.describe)()
            ));
        }
    }

    /// 任何一次成功都代表伺服器與設定已恢復：清狀態、重置退避、放開閘門。
    fn note_success(&self) {
        let changed = {
            let mut state = self.state();
            let changed = state.error_state.is_some();
            state.error_state = None;
            state.error_detail = None;
            state.backoff_index = 0;
            state.gate_until = None;
            state.successes += 1;
            changed
        };
        self.gate_opened.notify_waiters();
        if changed {
            log("[translate] recovered, resuming normal speed");
        }
    }
}

/// 一則翻譯工作；未回報就結束（關閉中放棄、task 被丟棄）時由 Drop 補上 in_flight 遞減。
struct Job {
    inner: Arc<Inner>,
    pending: bool,
    born: Instant,
}

impl Job {
    async fn run(mut self, line: String, context: Vec<String>, msg_id: u64) {
        let inner = Arc::clone(&self.inner);
        let mut attempts = 0;
        let mut last_error: Option<ErrorState> = None;
        // 連續 Config 錯誤共用同一個基準：別則在兩次嘗試之間成功也要看得出來
        let mut baseline: Option<u64> = None;
        while inner.wait_for_backoff().await {
            if self.born.elapsed() > MAX_JOB_AGE {
                log(format!(
                    "[translate] line dropped after {attempts} attempt(s), gave up after outage (last error: {}): {line}",
                    last_error.map_or("none", ErrorState::as_str)
                ));
                self.report(msg_id, (inner.failed_notice)(), true);
                return;
            }
            if last_error == Some(ErrorState::Config) {
                tokio::select! {
                    _ = inner.stop.cancelled() => return,
                    _ = tokio::time::sleep(CONFIG_RETRY_YIELD) => {}
                }
            }
            let Some(permit) = inner.gate.acquire(&inner.stop).await else {
                return;
            };
            // 等額度期間閘門可能已被別則的失敗推遠：放回額度重新等，排隊的各則才不會繞過退避
            if inner.backoff_remaining().is_some() {
                continue;
            }
            attempts += 1;
            let (successes_now, epoch_before) = {
                let state = inner.state();
                (state.successes, state.config_epoch)
            };
            let successes_before = *baseline.get_or_insert(successes_now);
            let request = async { (inner.translate)(line.clone(), context.clone()).await };
            // 額度留到 note_* 更新閘門之後才放回，下一則才不會讀到舊的 gate_until
            let (reason, message) = match AssertUnwindSafe(request).catch_unwind().await {
                Ok(Ok(translated)) => {
                    inner.note_success();
                    drop(permit);
                    self.report(msg_id, translated, false);
                    return;
                }
                Ok(Err(err @ TranslateError::Offline { .. })) => {
                    last_error = Some(ErrorState::Offline);
                    baseline = None;
                    inner.note_failure(ErrorState::Offline, &err);
                    continue;
                }
                Ok(Err(err @ TranslateError::Config { .. })) => {
                    last_error = Some(ErrorState::Config);
                    if inner.state().config_epoch != epoch_before {
                        // 請求是用舊設定送的：用新設定重來
                        baseline = None;
                        drop(permit);
                        continue;
                    }
                    // 別則成功過，設定沒問題；錯在這則本身，不動閘門與橫幅
                    if inner.state().successes != successes_before {
                        ("rejected while other lines succeed", err.to_string())
                    } else {
                        inner.note_failure(ErrorState::Config, &err);
                        continue;
                    }
                }
                // temperature=0 下重試必得同一結果：直接放棄，讓 overlay 至少顯示失敗提示
                Ok(Err(err @ TranslateError::BadOutput(_))) => {
                    ("bad model output", err.to_string())
                }
                Ok(Err(err)) => ("unexpected error", err.to_string()),
                Err(panic) => ("unexpected error", panic_message(panic.as_ref())),
            };
            drop(permit);
            log(format!(
                "[translate] line dropped after {attempts} attempt(s), {reason} ({message}): {line}"
            ));
            self.report(msg_id, (inner.failed_notice)(), true);
            return;
        }
    }

    /// in_flight 先遞減再回報：on_result 內讀 in_flight 不該把這則算進行中。
    fn report(&mut self, msg_id: u64, text: String, failed: bool) {
        self.pending = false;
        self.inner.state().in_flight -= 1;
        (self.inner.on_result)(msg_id, text, failed);
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if self.pending {
            self.inner.state().in_flight -= 1;
        }
    }
}
