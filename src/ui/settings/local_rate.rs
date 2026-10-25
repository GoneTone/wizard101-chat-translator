//! 下載速度與剩餘時間：由前後兩次快照的已下載量推算，速度做指數平滑。

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::local::DownloadState;

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
    /// 用 `now` 時的快照更新；不在下載中的模型記錄會被丟掉。
    pub fn observe(&mut self, now: Instant, downloads: &BTreeMap<String, DownloadState>) {
        self.seen
            .retain(|id, _| matches!(downloads.get(id), Some(DownloadState::Running { .. })));
        for (id, state) in downloads {
            let DownloadState::Running { done, .. } = state else {
                continue;
            };
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

/// 速度文字，如 `18.0 MB/s`、`512 KB/s`。
pub fn format_speed(rate: f64) -> String {
    const MB: f64 = (1u64 << 20) as f64;
    if rate >= MB {
        format!("{:.1} MB/s", rate / MB)
    } else {
        format!("{:.0} KB/s", rate / 1024.0)
    }
}
