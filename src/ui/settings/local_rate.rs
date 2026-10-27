//! 下載速度與剩餘時間：取最近幾秒已下載量的平均速度推算。

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crate::i18n::t;
use crate::local::{DownloadState, EngineState, Snapshot};
use crate::ui::settings::quant_table::gb;

/// 引擎安裝在 `Rates` 裡的鍵；模型 id 是資料夾名，不會含 `:`。
pub const ENGINE: &str = ":engine";

/// 每隔這麼久記一筆已下載量（速度與剩餘時間也就這麼久才變一次）。
const SAMPLE_EVERY: Duration = Duration::from_secs(1);
/// 速度取最近這段時間的平均：資料分段送達，短時間內量到的忽快忽慢。
const WINDOW: Duration = Duration::from_secs(8);

/// 各下載中模型最近的（取樣時間, 已下載量）。
#[derive(Default)]
pub struct Rates {
    seen: BTreeMap<String, VecDeque<(Instant, u64)>>,
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
        for (id, done) in running {
            let samples = self.seen.entry(id).or_default();
            // 已下載量倒退代表重新開始，舊取樣作廢
            if samples.back().is_some_and(|&(_, d)| done < d) {
                samples.clear();
            }
            if samples
                .back()
                .is_some_and(|&(at, _)| now.saturating_duration_since(at) < SAMPLE_EVERY)
            {
                continue;
            }
            samples.push_back((now, done));
            while samples
                .front()
                .is_some_and(|&(at, _)| now.saturating_duration_since(at) > WINDOW)
            {
                samples.pop_front();
            }
        }
    }

    /// 目前的速度；還沒有可用取樣或速度為 0 時為 `None`。
    pub fn speed(&self, id: &str) -> Option<f64> {
        let samples = self.seen.get(id)?;
        let (&(t0, d0), &(t1, d1)) = (samples.front()?, samples.back()?);
        let dt = t1.saturating_duration_since(t0).as_secs_f64();
        Some((d1 - d0) as f64 / dt).filter(|r| dt > 0.0 && *r > 0.0)
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

/// 引擎安裝的狀態文字：下載中同 `transfer_text`，解壓中（或全部下載完）是解壓縮。
pub fn engine_text(done: u64, total: u64, extracting: bool, speed: Option<f64>) -> String {
    if extracting || (total > 0 && done >= total) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(done: u64) -> Snapshot {
        let mut s = Snapshot::default();
        s.downloads
            .insert("m".into(), DownloadState::Running { done, total: 1000 });
        s
    }

    /// 速度是窗口內的平均，一秒內的重複繪製不另記；下載量倒退就重新算。
    #[test]
    fn speed_averages_the_recent_window() {
        let mut rates = Rates::default();
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        rates.observe(at(0), &snapshot(0));
        assert_eq!(rates.speed("m"), None);
        rates.observe(at(1000), &snapshot(300));
        rates.observe(at(1500), &snapshot(900));
        rates.observe(at(2000), &snapshot(400));
        assert_eq!(rates.speed("m"), Some(200.0));
        rates.observe(at(9000), &snapshot(2000));
        assert_eq!(rates.speed("m"), Some(1700.0 / 8.0));
        rates.observe(at(10_000), &snapshot(10));
        assert_eq!(rates.speed("m"), None);
        rates.observe(at(11_000), &Snapshot::default());
        assert!(rates.seen.is_empty());
    }
}
