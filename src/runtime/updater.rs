//! 更新檢查：回答「GitHub 上有沒有比目前這版更新的 release」，只查詢與比較，不下載也不碰 UI。

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use regex::Regex;
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::logs::log;
use crate::net::{bypass_proxy_for_loopback, error_chain, install_crypto_provider};
use crate::util::panic_message;

pub const PROJECT_URL: &str = "https://github.com/GoneTone/wizard101-chat-translator";
pub const RELEASES_URL: &str =
    "https://github.com/GoneTone/wizard101-chat-translator/releases/latest";
pub const ISSUES_URL: &str = "https://github.com/GoneTone/wizard101-chat-translator/issues";
pub const AUTHOR_URL: &str = "https://github.com/GoneTone";
pub const CROWDIN_URL: &str = "https://crowdin.com/project/wizard101-chat-translator";
pub const LATEST_API: &str =
    "https://api.github.com/repos/GoneTone/wizard101-chat-translator/releases/latest";

/// 自動檢查更新的間隔。
pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(3600);
/// 單次請求的逾時，免得手動檢查的按鈕卡住不放。
pub const TIMEOUT: Duration = Duration::from_secs(10);

// 只取前三段數字：/releases/latest 已排除 pre-release，容忍後綴只是不讓 tag 寫法害整個解析失敗。
static VERSION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^v?([0-9]+)\.([0-9]+)\.([0-9]+)(?:-([0-9A-Za-z.-]+))?").expect("version regex")
});

/// 預發布識別碼：數字比數值、字母比 ASCII，且數字一律低於字母。
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Identifier {
    Numeric(u64),
    Alphanumeric(String),
}

/// 同版號下的優先序：有預發布後綴者低於正式版。
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Prerelease(Vec<Identifier>),
    Release,
}

/// GitHub 上的一個 release：版本號（已去 `v` 前綴）與該 release 的網頁網址。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
}

/// 檢查更新失敗（連線不通、配額用盡、回應無法解析）；「沒有新版」不算失敗。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheckError(pub String);

impl std::fmt::Display for UpdateCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UpdateCheckError {}

/// SemVer 優先序的可比較鍵；build metadata 不參與比較，正則不收它即可忽略。
fn precedence_key(text: &str) -> Option<((u64, u64, u64), Stage)> {
    let caps = VERSION_RE.captures(text.trim())?;
    let number = |i: usize| caps[i].parse::<u64>().ok();
    let numbers = (number(1)?, number(2)?, number(3)?);
    let Some(prerelease) = caps.get(4) else {
        return Some((numbers, Stage::Release));
    };
    let identifiers = prerelease
        .as_str()
        .split('.')
        .map(|part| {
            if !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) {
                part.parse().ok().map(Identifier::Numeric)
            } else {
                Some(Identifier::Alphanumeric(part.to_owned()))
            }
        })
        .collect::<Option<Vec<_>>>()?;
    Some((numbers, Stage::Prerelease(identifiers)))
}

/// `latest` 是否嚴格新於 `current`；任一邊解析不出來就回 `false`，寧可漏提醒也不誤報。
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (precedence_key(latest), precedence_key(current)) {
        (Some(newer), Some(mine)) => newer > mine,
        _ => {
            log(format!(
                "[update] version unparsable: latest={latest:?} current={current:?}"
            ));
            false
        }
    }
}

/// 檢查更新用、連往 `url` 的 HTTP client：逾時 [`TIMEOUT`]、跟隨轉址（repo 改名時 GitHub API 會回 301）。
pub fn update_client(url: &str) -> reqwest::Client {
    install_crypto_provider();
    let builder = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(10));
    bypass_proxy_for_loopback(builder, url)
        .build()
        .expect("build HTTP client")
}

/// 取 `url` 上最新的正式 release；尚未發過任何 release（404）時回 `Ok(None)`。
pub async fn fetch_latest_release(
    client: &reqwest::Client,
    url: &str,
) -> Result<Option<Release>, UpdateCheckError> {
    // GitHub API 要求帶 User-Agent，缺了會被拒（403）
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header(
            "User-Agent",
            format!("wizard101-chat-translator/{}", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .await
        .map_err(|e| UpdateCheckError(error_chain(e)))?;
    let status = response.status();
    if status.as_u16() == 404 {
        log("[update] no release published yet (404)");
        return Ok(None);
    }
    if status.as_u16() != 200 {
        return Err(UpdateCheckError(format!("HTTP {}", status.as_u16())));
    }
    let data: Value = response
        .json()
        .await
        .map_err(|e| UpdateCheckError(format!("unexpected response: {}", error_chain(e))))?;
    let tag = data["tag_name"]
        .as_str()
        .ok_or_else(|| UpdateCheckError("unexpected response: missing tag_name".into()))?;
    let html_url = data["html_url"]
        .as_str()
        .filter(|u| !u.is_empty())
        .unwrap_or(RELEASES_URL);
    Ok(Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_owned(),
        url: html_url.to_owned(),
    }))
}

/// 回傳「比 `current` 新的 release」；沒有新版（含尚未發版）回 `Ok(None)`，查詢失敗回 `Err`。
pub async fn check_for_update(
    client: &reqwest::Client,
    url: &str,
    current: &str,
) -> Result<Option<Release>, UpdateCheckError> {
    log("[update] checking latest release");
    let Some(release) = fetch_latest_release(client, url).await? else {
        return Ok(None);
    };
    let newer = is_newer(&release.version, current);
    log(format!(
        "[update] latest={} current={current} newer={newer}",
        release.version
    ));
    Ok(newer.then_some(release))
}

/// 立刻查一次更新，之後每隔 `interval` 再查，直到 `stop` 觸發；有新版就呼叫 `found`。
///
/// 失敗只留 log：更新檢查是附加功能，不能影響啟動與收訊，下一輪照常再查。
pub fn schedule_update_checks(
    rt: &Handle,
    stop: CancellationToken,
    url: String,
    interval: Duration,
    found: Arc<dyn Fn(Release) + Send + Sync>,
) -> JoinHandle<()> {
    rt.spawn(async move {
        let client = update_client(&url);
        loop {
            let checked = tokio::select! {
                biased;
                _ = stop.cancelled() => return,
                result = check_for_update(&client, &url, env!("CARGO_PKG_VERSION")) => result,
            };
            match checked {
                Ok(Some(release)) => {
                    let announce = std::panic::AssertUnwindSafe(|| found(release));
                    if let Err(panic) = std::panic::catch_unwind(announce) {
                        log(format!(
                            "[update] announce callback panicked: {}",
                            panic_message(panic.as_ref())
                        ));
                    }
                }
                Ok(None) => {}
                Err(e) => log(format!("[update] check failed: {e}")),
            }
            log(format!(
                "[update] next automatic check in {} min",
                interval.as_secs() / 60
            ));
            tokio::select! {
                biased;
                _ = stop.cancelled() => return,
                _ = tokio::time::sleep(interval) => {}
            }
        }
    })
}
