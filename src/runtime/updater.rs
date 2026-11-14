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
pub const RELEASES_API: &str =
    "https://api.github.com/repos/GoneTone/wizard101-chat-translator/releases";
/// release 附的 exe 檔名，更新時就是下載這個 asset。
pub const EXE_ASSET: &str = "Wizard101ChatTranslator.exe";

/// 自動檢查更新的間隔。
pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(3600);
/// 單次請求的逾時，免得手動檢查的按鈕卡住不放。
pub const TIMEOUT: Duration = Duration::from_secs(10);

// 只取前三段數字：pre-release 由 API 的旗標排除，容忍後綴只是不讓 tag 寫法害整個解析失敗。
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

/// release 附的 exe：下載網址、位元組數與 SHA-256（小寫十六進位，已去 `sha256:` 前綴）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

/// GitHub 上的一個 release；`asset` 為 `None` 代表沒有 exe 或缺 digest，無法驗證下載。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Release {
    /// 已去 `v` 前綴。
    pub version: String,
    /// release 的網頁網址。
    pub url: String,
    /// API 原字串（RFC 3339）。
    pub published_at: String,
    /// Markdown 版本說明。
    pub body: String,
    pub asset: Option<Asset>,
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

fn parse_asset(release: &Value) -> Option<Asset> {
    let asset = release["assets"]
        .as_array()?
        .iter()
        .find(|a| a["name"] == EXE_ASSET)?;
    Some(Asset {
        url: asset["browser_download_url"].as_str()?.to_owned(),
        size: asset["size"].as_u64()?,
        sha256: asset["digest"]
            .as_str()?
            .strip_prefix("sha256:")?
            .to_ascii_lowercase(),
    })
}

/// 解析單一 release 物件；缺 `tag_name` 回 `None`。
fn parse_release(item: &Value) -> Option<Release> {
    let version = item["tag_name"].as_str()?.trim_start_matches(['v', 'V']);
    let text = |key: &str| item[key].as_str().unwrap_or_default().to_owned();
    Some(Release {
        version: version.to_owned(),
        url: item["html_url"]
            .as_str()
            .filter(|u| !u.is_empty())
            .unwrap_or(RELEASES_URL)
            .to_owned(),
        published_at: text("published_at"),
        body: text("body"),
        asset: parse_asset(item),
    })
}

/// 從 `/releases` 清單挑出比 `current` 新的正式 release，由新到舊；草稿與 pre-release 不算。
pub fn parse_releases(json: &Value, current: &str) -> Result<Vec<Release>, UpdateCheckError> {
    let items = json.as_array().ok_or_else(|| {
        let shown: String = json.to_string().chars().take(80).collect();
        UpdateCheckError(format!("unexpected response: {shown}"))
    })?;
    let mut releases = Vec::new();
    for item in items {
        if item["draft"] == true || item["prerelease"] == true {
            continue;
        }
        let Some(release) = parse_release(item) else {
            log("[update] skipped a release without tag_name");
            continue;
        };
        if is_newer(&release.version, current) {
            releases.push(release);
        }
    }
    releases.sort_by_key(|r| std::cmp::Reverse(precedence_key(&r.version)));
    Ok(releases)
}

/// 取 `url` 上所有比 `current` 新的 release（新到舊）；空 vec 是沒有新版（含尚未發版），查詢失敗回 `Err`。
pub async fn check_for_update(
    client: &reqwest::Client,
    url: &str,
    current: &str,
) -> Result<Vec<Release>, UpdateCheckError> {
    log("[update] checking releases");
    let Some(data) = get_json(client, url).await? else {
        log("[update] no release published yet (404)");
        return Ok(Vec::new());
    };
    let releases = parse_releases(&data, current)?;
    log(format!(
        "[update] newer releases: count={} latest={}",
        releases.len(),
        releases.first().map_or("-", |r| r.version.as_str())
    ));
    Ok(releases)
}

/// GET `url` 並解析成 JSON；404 回 `Ok(None)`，其他非 200 與壞掉的 JSON 回 `Err`。
async fn get_json(client: &reqwest::Client, url: &str) -> Result<Option<Value>, UpdateCheckError> {
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
        return Ok(None);
    }
    if status.as_u16() != 200 {
        return Err(UpdateCheckError(format!("HTTP {}", status.as_u16())));
    }
    response
        .json()
        .await
        .map(Some)
        .map_err(|e| UpdateCheckError(format!("unexpected response: {}", error_chain(e))))
}

/// 指定版本在 `api`（releases API 根網址）上的單一 release 網址。
pub fn release_tag_url(api: &str, version: &str) -> String {
    format!("{api}/tags/v{version}")
}

/// 取 `url` 上的單一 release；404 回 `Ok(None)`，其他失敗回 `Err`。
pub async fn fetch_release(
    client: &reqwest::Client,
    url: &str,
) -> Result<Option<Release>, UpdateCheckError> {
    log(format!("[update] fetching release notes: {url}"));
    let Some(data) = get_json(client, url).await? else {
        log("[update] release notes not found (404)");
        return Ok(None);
    };
    let release = parse_release(&data).ok_or_else(|| {
        let shown: String = data.to_string().chars().take(80).collect();
        UpdateCheckError(format!("unexpected response: {shown}"))
    })?;
    log(format!(
        "[update] release notes loaded: version={} published={}",
        release.version, release.published_at
    ));
    Ok(Some(release))
}

/// 立刻查一次更新，之後每隔 `interval` 再查，直到 `stop` 觸發；有新版就把清單交給 `found`。
///
/// 失敗只留 log：更新檢查是附加功能，不能影響啟動與收訊，下一輪照常再查。
pub fn schedule_update_checks(
    rt: &Handle,
    stop: CancellationToken,
    url: String,
    interval: Duration,
    found: Arc<dyn Fn(Vec<Release>) + Send + Sync>,
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
                Ok(releases) if !releases.is_empty() => {
                    let announce = std::panic::AssertUnwindSafe(|| found(releases));
                    if let Err(panic) = std::panic::catch_unwind(announce) {
                        log(format!(
                            "[update] announce callback panicked: {}",
                            panic_message(panic.as_ref())
                        ));
                    }
                }
                Ok(_) => {}
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
