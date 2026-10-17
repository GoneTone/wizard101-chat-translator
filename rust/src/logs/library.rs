//! 第三方套件經 `log` crate 發出的警告轉進 app.log：依來源套件加模組前綴，同一則重複時限流。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use super::log;

/// 同一則訊息重複時，至多每隔這麼久寫出一次（附上期間略過的次數）。
const REPEAT_WINDOW: Duration = Duration::from_secs(60);
/// 記住的不同訊息上限；超過就整份重來。
const MAX_TRACKED: usize = 256;

const UI_CRATES: [&str; 6] = ["wgpu", "naga", "egui", "eframe", "winit", "accesskit"];
const NETWORK_CRATES: [&str; 4] = ["reqwest", "hyper", "rustls", "h2"];

/// 依 log target 的套件名決定前綴：繪圖與視窗歸 `[ui]`、HTTP 歸 `[translate]`、其餘 `[app]`。
pub fn prefix_for(target: &str) -> &'static str {
    let krate = target.split("::").next().unwrap_or_default();
    if UI_CRATES.iter().any(|c| krate.starts_with(c)) {
        "[ui]"
    } else if NETWORK_CRATES.iter().any(|c| krate.starts_with(c)) {
        "[translate]"
    } else {
        "[app]"
    }
}

/// 重複訊息的限流：wgpu 之類每幀都可能發同一則警告，不能把 app.log 灌爆。
#[derive(Default)]
pub struct RepeatLimiter {
    /// 訊息 → （上次寫出的時刻, 之後略過的次數）。
    seen: HashMap<String, (Instant, u32)>,
}

impl RepeatLimiter {
    /// 第一次照寫；之後一分鐘內的重複只計數，過了一分鐘的下一次再寫出並附上略過的次數。
    pub fn admit(&mut self, line: &str, now: Instant) -> Option<String> {
        match self.seen.get_mut(line) {
            Some((since, skipped)) if now.duration_since(*since) < REPEAT_WINDOW => {
                *skipped += 1;
                None
            }
            Some((since, skipped)) => {
                *since = now;
                Some(match std::mem::take(skipped) {
                    0 => line.to_owned(),
                    n => format!("{line} (repeated {n} times since last logged)"),
                })
            }
            None => {
                // shortcut: 超過上限就全部忘掉，極端情況下會多寫幾行；真的遇到再改成 LRU
                if self.seen.len() >= MAX_TRACKED {
                    self.seen.clear();
                }
                self.seen.insert(line.to_owned(), (now, 0));
                Some(line.to_owned())
            }
        }
    }
}

struct LibraryLogs(Mutex<RepeatLimiter>);

impl ::log::Log for LibraryLogs {
    fn enabled(&self, metadata: &::log::Metadata) -> bool {
        metadata.level() <= ::log::Level::Warn
    }

    fn log(&self, record: &::log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let target = record.target();
        let line = format!(
            "{} {} {target}: {}",
            prefix_for(target),
            record.level(),
            record.args()
        );
        let admitted = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .admit(&line, Instant::now());
        if let Some(line) = admitted {
            log(line);
        }
    }

    fn flush(&self) {}
}

static LIBRARY_LOGS: LazyLock<LibraryLogs> =
    LazyLock::new(|| LibraryLogs(Mutex::new(RepeatLimiter::default())));

/// 把第三方套件（wgpu、egui、reqwest…）的警告與錯誤轉進 app.log。
pub fn forward_library_logs() {
    if ::log::set_logger(&*LIBRARY_LOGS).is_ok() {
        ::log::set_max_level(::log::LevelFilter::Warn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_follows_the_source_crate() {
        assert_eq!(prefix_for("wgpu_hal::dx12::adapter"), "[ui]");
        assert_eq!(prefix_for("egui_wgpu"), "[ui]");
        assert_eq!(prefix_for("winit::platform_impl"), "[ui]");
        assert_eq!(prefix_for("reqwest::connect"), "[translate]");
        assert_eq!(prefix_for("hyper_util::client"), "[translate]");
        assert_eq!(prefix_for("rustls::conn"), "[translate]");
        assert_eq!(prefix_for("tokio::runtime"), "[app]");
    }

    #[test]
    fn repeats_are_counted_and_summarized_at_most_once_a_minute() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut limiter = RepeatLimiter::default();
        assert_eq!(limiter.admit("w", at(0)).as_deref(), Some("w"));
        assert_eq!(limiter.admit("w", at(1)), None);
        assert_eq!(limiter.admit("w", at(59)), None);
        assert_eq!(
            limiter.admit("other", at(2)).as_deref(),
            Some("other"),
            "different messages are independent"
        );
        assert_eq!(
            limiter.admit("w", at(60)).as_deref(),
            Some("w (repeated 2 times since last logged)")
        );
        assert_eq!(limiter.admit("w", at(61)), None);
        assert_eq!(
            limiter.admit("other", at(200)).as_deref(),
            Some("other"),
            "no summary when nothing was skipped"
        );
    }

    #[test]
    fn tracked_messages_are_bounded() {
        let now = Instant::now();
        let mut limiter = RepeatLimiter::default();
        for i in 0..MAX_TRACKED * 2 {
            limiter.admit(&format!("m{i}"), now);
        }
        assert!(limiter.seen.len() <= MAX_TRACKED);
    }
}
