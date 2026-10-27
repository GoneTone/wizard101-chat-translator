//! 下載速度與剩餘時間：取最近幾秒已下載量的平均速度推算。

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crate::i18n::t;
use crate::local::{DownloadState, EngineState, Snapshot};
use crate::ui::settings::quant_table::gb;

/// 引擎安裝在 `Rates` 裡的鍵；模型 id 是資料夾名，不會含 `:`。
pub const ENGINE: &str = ":engine";

/// 兩次取樣至少相隔這麼久才算一筆（同一幀多次繪製不重複取樣）。
const MIN_INTERVAL: Duration = Duration::from_millis(200);
/// 速度取最近這段時間的平均：資料分段送達，短時間內量到的忽快忽慢。
const WINDOW: Duration = Duration::from_secs(8);
/// 第一筆速度一有就顯示，之後每隔這麼久才換：數字不會一直跳。
const SHOW_EVERY: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Track {
    /// 最近的（取樣時間, 已下載量）。
    samples: VecDeque<(Instant, u64)>,
    /// 顯示中的速度與換上的時間。
    shown: Option<(Instant, f64)>,
}

/// 各下載中模型的速度（位元組／秒）。
#[derive(Default)]
pub struct Rates {
    seen: BTreeMap<String, Track>,
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
            let track = self.seen.entry(id).or_default();
            // 已下載量倒退代表重新開始，舊速度作廢
            if track.samples.back().is_some_and(|&(_, d)| done < d) {
                *track = Track::default();
            }
            let since = |at: Instant| now.saturating_duration_since(at);
            if track
                .samples
                .back()
                .is_some_and(|&(at, _)| since(at) < MIN_INTERVAL)
            {
                continue;
            }
            track.samples.push_back((now, done));
            while track
                .samples
                .front()
                .is_some_and(|&(at, _)| since(at) > WINDOW)
            {
                track.samples.pop_front();
            }
            let due = track.shown.is_none_or(|(at, _)| since(at) >= SHOW_EVERY);
            if let (true, Some(rate)) = (due, average(&track.samples)) {
                track.shown = Some((now, rate));
            }
        }
    }

    /// 目前的速度；還沒有可用取樣或速度為 0 時為 `None`。
    pub fn speed(&self, id: &str) -> Option<f64> {
        self.seen
            .get(id)?
            .shown
            .map(|(_, r)| r)
            .filter(|r| *r > 0.0)
    }
}

/// 第一筆到最後一筆之間的平均速度；不到兩筆為 `None`。
fn average(samples: &VecDeque<(Instant, u64)>) -> Option<f64> {
    let (&(t0, d0), &(t1, d1)) = (samples.front()?, samples.back()?);
    let dt = t1.saturating_duration_since(t0).as_secs_f64();
    (dt > 0.0).then(|| (d1 - d0) as f64 / dt)
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

    /// 第一筆速度一有就顯示，之後一秒才換一次；速度是窗口內的平均，下載量倒退就重新算。
    #[test]
    fn speed_averages_the_recent_window() {
        let mut rates = Rates::default();
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        rates.observe(at(0), &snapshot(0));
        assert_eq!(rates.speed("m"), None);
        rates.observe(at(250), &snapshot(100));
        assert_eq!(rates.speed("m"), Some(400.0));
        rates.observe(at(500), &snapshot(300));
        assert_eq!(rates.speed("m"), Some(400.0));
        rates.observe(at(1250), &snapshot(1000));
        assert_eq!(rates.speed("m"), Some(800.0));
        rates.observe(at(9000), &snapshot(2000));
        assert_eq!(rates.speed("m"), Some(1000.0 / 7.75));
        rates.observe(at(9250), &snapshot(10));
        assert_eq!(rates.speed("m"), None);
        rates.observe(at(9500), &Snapshot::default());
        assert!(rates.seen.is_empty());
    }
}
