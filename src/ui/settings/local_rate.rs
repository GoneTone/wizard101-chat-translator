//! 下載速度與剩餘時間：由前後兩次快照的已下載量推算，速度做指數平滑。

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::i18n::t;
use crate::local::{DownloadState, EngineState, Snapshot};
use crate::ui::settings::quant_table::gb;

/// 引擎安裝在 `Rates` 裡的鍵；模型 id 是資料夾名，不會含 `:`。
pub const ENGINE: &str = ":engine";

/// 兩次取樣至少相隔這麼久才算一筆（同一幀多次繪製不重複取樣）。
const MIN_INTERVAL: Duration = Duration::from_millis(200);
const ALPHA: f64 = 0.3;

struct Seen {
    at: Instant,
    done: u64,
    rate: Option<f64>,
}

/// 各下載中模型的速度（位元組／秒）。
#[derive(Default)]
pub struct Rates {
    seen: BTreeMap<String, Seen>,
}

impl Rates {
    /// 用 `now` 時的快照更新下載中的模型與引擎；已不在下載的記錄會被丟掉。
    pub fn observe(&mut self, now: Instant, snapshot: &Snapshot) {
        let mut running: BTreeMap<String, u64> = snapshot
            .downloads
            .iter()
            .filter_map(|(id, state)| match state {
                DownloadState::Running { done, .. } => Some((id.clone(), *done)),
                _ => None,
            })
            .collect();
        if let EngineState::Installing { done, .. } = &snapshot.engine {
            running.insert(ENGINE.to_owned(), *done);
        }
        self.seen.retain(|id, _| running.contains_key(id));
        for (id, done) in &running {
            let Some(prev) = self.seen.get_mut(id) else {
                self.seen.insert(
                    id.clone(),
                    Seen {
                        at: now,
                        done: *done,
                        rate: None,
                    },
                );
                continue;
            };
            let dt = now.saturating_duration_since(prev.at);
            if dt < MIN_INTERVAL {
                continue;
            }
            // 已下載量倒退代表重新開始，舊速度作廢
            prev.rate = match done.checked_sub(prev.done) {
                Some(delta) => {
                    let sample = delta as f64 / dt.as_secs_f64();
                    Some(
                        prev.rate
                            .map_or(sample, |r| ALPHA * sample + (1.0 - ALPHA) * r),
                    )
                }
                None => None,
            };
            (prev.at, prev.done) = (now, *done);
        }
    }

    /// 目前的速度；還沒有可用取樣或速度為 0 時為 `None`。
    pub fn speed(&self, id: &str) -> Option<f64> {
        self.seen.get(id)?.rate.filter(|r| *r > 0.0)
    }
}

/// 以 `rate` 下載完剩下的部分要多久。
pub fn eta(done: u64, total: u64, rate: f64) -> Duration {
    Duration::from_secs_f64(total.saturating_sub(done) as f64 / rate)
}

/// 下載中的狀態文字：「下載中 X / Y GB」，有速度時接上速度與剩餘時間。
pub fn transfer_text(done: u64, total: u64, speed: Option<f64>) -> String {
    let mut line = t(
        "local.models.downloading",
        &[("done", &gb(done)), ("total", &gb(total))],
    );
    if let Some(rate) = speed {
        let left = eta_text(eta(done, total, rate));
        let speed = format_speed(rate);
        line += &format!(
            " · {}",
            t(
                "local.models.speed_eta",
                &[("speed", &speed), ("eta", &left)]
            )
        );
    }
    line
}

/// 引擎安裝的狀態文字：下載中同 `transfer_text`，下載完是解壓縮。
pub fn engine_text(done: u64, total: u64, speed: Option<f64>) -> String {
    if total > 0 && done >= total {
        t("local.engine.extracting", &[])
    } else {
        transfer_text(done, total, speed)
    }
}

/// 剩餘時間文字：不到一分鐘用秒，不到一小時用分，其餘用小時加分。
fn eta_text(left: Duration) -> String {
    let secs = left.as_secs_f64().ceil() as u64;
    let n = |v: u64| v.to_string();
    match secs {
        0..60 => t("local.models.eta_seconds", &[("n", &n(secs.max(1)))]),
        60..3600 => t("local.models.eta_minutes", &[("n", &n(secs.div_ceil(60)))]),
        _ => t(
            "local.models.eta_hours",
            &[("h", &n(secs / 3600)), ("m", &n(secs % 3600 / 60))],
        ),
    }
}

/// 速度文字，如 `18.0 MB/s`、`512 KB/s`。
pub fn format_speed(rate: f64) -> String {
    const MB: f64 = (1u64 << 20) as f64;
    if rate >= MB {
        format!("{:.1} MB/s", rate / MB)
    } else {
        format!("{:.0} KB/s", rate / 1024.0)
    }
}
