//! 各服務商的 HTTP client：統一的 [`Client`] 入口、請求逾時與 token 上限、可取消的請求。

mod claude;
mod gemini;
mod openai;
mod sse;

use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub use claude::{CLAUDE_BASE_URL, ClaudeClient};
pub use gemini::{GEMINI_BASE_URL, GeminiClient};
pub use openai::OpenAiCompatClient;

use super::errors::{MAX_RETRY_AFTER, TranslateError, error_detail, status_error};
use super::postprocess::strip_think;
use super::{BoxFuture, Turn};
use crate::config::ServiceSpec;
use crate::config::services::{EFFORT_AUTO, known_effort, provider};
use crate::logs::log;
use crate::net::{bypass_proxy_for_loopback, error_chain, install_crypto_provider};
use crate::py::pyrepr::repr_str;
use crate::py::pystr;

/// 連線逾時與讀取逾時：位元組停住這麼久就斷線；閘道的保持連線註解與 Claude 的 `ping` 都會重新計時。
pub const TIMEOUT: Duration = Duration::from_secs(60);

/// HTTP/2 閒置時的 PING 間隔：偵測電腦睡眠、換網路後的死連線；逾 [`KEEPALIVE_TIMEOUT`] 沒回應就丟掉這條連線。
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(10);

/// 一次聊天請求從連線到讀完的總期限：擋住永不收尾的串流，又夠本機慢模型寫完長篇的區域翻譯。
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

/// 譯文長度上限，用意是防 repetition loop 吃穿逾時而非省 token。
pub const MAX_TOKENS: u32 = 512;

/// 思考模式下推理本身就吃掉大量 token，上限需放寬才不會砍在譯文之前。
pub const MAX_TOKENS_THINKING: u32 = 4096;

/// 區域翻譯一次可能送整頁任務書，固定用放寬的上限。
pub const MAX_TOKENS_REGION: u32 = 4096;

/// ChatGPT preset 固定的官方端點。
pub const OPENAI_BASE_URL: &str = "https://api.openai.com";

const TRUNCATED_SAMPLE_CHARS: usize = 80;

/// 一次聊天請求的內容。
pub struct ChatRequest<'a> {
    pub system: &'a str,
    pub turns: &'a [Turn],
    /// 譯文長度上限；`None` 依思考模式取預設。
    pub max_tokens: Option<u32>,
    /// `false` 時自架端點不送 temperature（生成範例的重試要靠隨機性換結果）。
    pub deterministic: bool,
}

/// 翻譯後端的抽象，讓上層以假的 backend 測試。
pub trait ChatBackend: Send + Sync {
    /// 送出聊天請求並回傳整段譯文；`cancel` 觸發時關閉連線並回 `Cancelled`。
    fn chat<'a>(
        &'a self,
        req: ChatRequest<'a>,
        cancel: Option<&'a CancellationToken>,
    ) -> BoxFuture<'a, Result<String, TranslateError>>;

    /// 目前使用的模型 ID（診斷 log 用）。
    fn model(&self) -> &str;

    /// 先連上端點放進連線池，下一個請求免交握；`label` 標在 log 上。
    fn warm_up<'a>(&'a self, _label: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// 依服務商建立的 client。
pub enum Client {
    OpenAiCompat(OpenAiCompatClient),
    Claude(ClaudeClient),
    Gemini(GeminiClient),
}

impl Client {
    /// 列出端點的模型 ID（排序後）。
    pub async fn list_models(&self) -> Result<Vec<String>, TranslateError> {
        match self {
            Self::OpenAiCompat(c) => c.list_models().await,
            Self::Claude(c) => c.list_models().await,
            Self::Gemini(c) => c.list_models().await,
        }
    }
}

impl ChatBackend for Client {
    fn chat<'a>(
        &'a self,
        req: ChatRequest<'a>,
        cancel: Option<&'a CancellationToken>,
    ) -> BoxFuture<'a, Result<String, TranslateError>> {
        Box::pin(async move {
            match self {
                Self::OpenAiCompat(c) => c.chat(req, cancel).await,
                Self::Claude(c) => c.chat(req, cancel).await,
                Self::Gemini(c) => c.chat(req, cancel).await,
            }
        })
    }

    fn model(&self) -> &str {
        match self {
            Self::OpenAiCompat(c) => c.model(),
            Self::Claude(c) => c.model(),
            Self::Gemini(c) => c.model(),
        }
    }

    fn warm_up<'a>(&'a self, label: &'a str) -> BoxFuture<'a, ()> {
        let (http, base_url) = match self {
            Self::OpenAiCompat(c) => (&c.http, &c.base_url),
            Self::Claude(c) => (&c.http, &c.base_url),
            Self::Gemini(c) => (&c.http, &c.base_url),
        };
        Box::pin(warm_up(http, base_url, label))
    }
}

/// 送一個不帶金鑰的 `HEAD` 把連線建好留在池裡；回什麼狀態碼都無妨，失敗只記 log。
async fn warm_up(http: &reqwest::Client, base_url: &str, label: &str) {
    let started = Instant::now();
    match http.head(base_url).timeout(TIMEOUT).send().await {
        Ok(resp) => log(format!(
            "[translate] warm-up done in {:.2}s: {label} http={:?} status={}",
            started.elapsed().as_secs_f64(),
            resp.version(),
            resp.status().as_u16()
        )),
        Err(error) => log(format!(
            "[translate] warm-up failed: {label} ({})",
            error_chain(error)
        )),
    }
}

/// 依服務設定建立 client：`claude`／`gemini`／`openai` 走各自官方端點，其餘為自訂端點。
pub fn build_client(spec: &ServiceSpec) -> Client {
    let (key, model) = (&spec.api_key, &spec.model);
    let effort = provider(&spec.provider).map_or(EFFORT_AUTO, |p| known_effort(p, &spec.effort));
    match spec.provider.as_str() {
        "claude" => Client::Claude(ClaudeClient::new(key, model, effort, CLAUDE_BASE_URL)),
        "gemini" => Client::Gemini(GeminiClient::new(key, model, effort, GEMINI_BASE_URL)),
        "openai" => Client::OpenAiCompat(OpenAiCompatClient::new(
            OPENAI_BASE_URL,
            key,
            model,
            spec.thinking,
            true,
        )),
        _ => Client::OpenAiCompat(OpenAiCompatClient::new(
            &spec.base_url,
            key,
            model,
            spec.thinking,
            false,
        )),
    }
}

/// 金鑰標頭值（標成敏感，不進 Debug 輸出）；含 HTTP 標頭不允許的字元時回 `None`。
pub(crate) fn secret_header(value: &str) -> Option<HeaderValue> {
    let mut value = HeaderValue::from_str(value).ok()?;
    value.set_sensitive(true);
    Some(value)
}

/// 錯誤狀態碼依 `map` 分類；分類不了（1xx／3xx 等）的歸為 `Unexpected`。
pub(crate) fn status_failure(
    status: u16,
    detail: String,
    map: fn(u16, &str) -> Option<TranslateError>,
) -> TranslateError {
    map(status, &detail)
        .unwrap_or_else(|| TranslateError::Unexpected(format!("HTTP {status}: {detail}")))
}

/// 非成功回應：讀完 body 後依 `map` 分類成錯誤；可重試的錯誤帶上伺服器要求的等待時間。
pub(crate) async fn response_failure(
    resp: reqwest::Response,
    map: fn(u16, &str) -> Option<TranslateError>,
) -> TranslateError {
    let status = resp.status().as_u16();
    let header_wait = retry_after(resp.headers());
    let text = match resp.text().await {
        Ok(text) => text,
        Err(error) => return connection_error(error),
    };
    let mut error = status_failure(status, error_detail(&text), map);
    if let TranslateError::Offline {
        retry_after: wait, ..
    } = &mut error
    {
        *wait = header_wait.or_else(|| body_retry_delay(&text));
    }
    error
}

/// 伺服器要求的等待時間，夾在 [`MAX_RETRY_AFTER`]；認不出或不是正數為 `None`。
pub(crate) fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    requested_seconds(headers).and_then(capped)
}

/// 同 [`retry_after`]，但要求超過 [`MAX_RETRY_AFTER`] 時為 `None`（anthropic SDK 此時改走自己的退避）。
pub(crate) fn retry_after_within_cap(headers: &HeaderMap) -> Option<Duration> {
    requested_seconds(headers)
        .filter(|seconds| *seconds <= MAX_RETRY_AFTER.as_secs_f64())
        .and_then(capped)
}

/// 依序看 `retry-after-ms`、`retry-after`（秒數或 HTTP 日期）要求的秒數，未夾上限。
fn requested_seconds(headers: &HeaderMap) -> Option<f64> {
    let header = |name: &str| headers.get(name)?.to_str().ok().map(pystr::strip);
    let seconds = if let Some(ms) = header("retry-after-ms").and_then(|v| v.parse::<f64>().ok()) {
        ms / 1000.0
    } else {
        let value = header("retry-after")?;
        match value.parse::<f64>() {
            Ok(seconds) => seconds,
            Err(_) => {
                let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
                (date.timestamp_millis() - chrono::Utc::now().timestamp_millis()) as f64 / 1000.0
            }
        }
    };
    positive_seconds(seconds)
}

/// Gemini 錯誤 body 裡 `google.rpc.RetryInfo` 的 `retryDelay`（如 `"37s"`）。
fn body_retry_delay(text: &str) -> Option<Duration> {
    let body: Value = serde_json::from_str(text).ok()?;
    let body = match &body {
        Value::Array(items) if items.len() == 1 => &items[0],
        body => body,
    };
    let seconds = body
        .pointer("/error/details")?
        .as_array()?
        .iter()
        .find_map(|d| {
            d.get("retryDelay")?
                .as_str()?
                .strip_suffix('s')?
                .parse()
                .ok()
        })?;
    positive_seconds(seconds).and_then(capped)
}

fn positive_seconds(seconds: f64) -> Option<f64> {
    (seconds.is_finite() && seconds > 0.0).then_some(seconds)
}

/// 先夾上限再轉：極大的有限值（如 `1e20`）超出 `Duration` 範圍，但意思是「等很久」而非「沒說」。
fn capped(seconds: f64) -> Option<Duration> {
    let wait = Duration::from_secs_f64(seconds.min(MAX_RETRY_AFTER.as_secs_f64()));
    (!wait.is_zero()).then_some(wait)
}

/// 串流中途的錯誤：有狀態碼就照 [`status_error`] 分類，否則以 `raw` 對應未分類的例外。
pub(crate) fn stream_failure(code: Option<u16>, message: &str, raw: &Value) -> TranslateError {
    match code {
        Some(code) => status_failure(code, pystr::one_line(message), status_error),
        None => TranslateError::Unexpected(pystr::one_line(&raw.to_string())),
    }
}

/// 錯誤類型名稱（Claude 的 `error.type`、OpenAI 的 `type`／字串 `code`）對應的 HTTP 狀態碼。
pub(crate) fn error_type_status(kind: &str) -> Option<u16> {
    match kind {
        "rate_limit_error" | "rate_limit_exceeded" => Some(429),
        "api_error" | "server_error" => Some(500),
        "timeout_error" => Some(504),
        "overloaded_error" => Some(529),
        _ => None,
    }
}

/// 串流中途的 `{"error": …}` 塊：`code` 是 HTTP 狀態碼範圍內的數字（或數字字串）就照它分類，否則以 `code`／`type` 的名稱對照狀態碼。
pub(crate) fn chunk_error(error: &Value) -> TranslateError {
    let code = error.get("code");
    // 有些服務商的 code 是業務代碼（如 1301），不能當 HTTP 狀態碼分類
    let numeric = code
        .and_then(|c| c.as_u64().or_else(|| c.as_str()?.parse().ok()))
        .filter(|c| (100..=599).contains(c))
        .map(|c| c as u16);
    let named = || {
        [code, error.get("type")]
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .find_map(error_type_status)
    };
    let message = error.get("message").and_then(Value::as_str).unwrap_or("");
    stream_failure(numeric.or_else(named), message, error)
}

/// 組出帶診斷資訊的截斷錯誤：token 數與樣本讓使用者從 app.log 分辨 repetition loop 與上限誤砍。
pub(crate) fn truncated(
    max_tokens: u32,
    completion_tokens: Option<&Value>,
    sample: &str,
) -> TranslateError {
    TranslateError::BadOutput(format!(
        "output truncated at max_tokens={max_tokens}, completion_tokens={}, sample={}",
        count_of(completion_tokens),
        quoted_sample(sample)
    ))
}

/// 服務商擋下的回應（安全過濾等）：已收到的半句不當譯文，帶結束原因與樣本。
pub(crate) fn blocked(finish_reason: &str, sample: &str) -> TranslateError {
    TranslateError::BadOutput(format!(
        "response blocked with finish_reason={finish_reason}, sample={}",
        quoted_sample(sample)
    ))
}

/// 串流沒收完的錯誤說明；會直接顯示在介面上，服務商、模型等診斷資訊只進 log。
pub(crate) const INCOMPLETE_STREAM: &str = "stream ended before completion";

/// 串流沒收到結束訊號就收尾了（代理／閘道提前關線），屬暫時性中斷；`missing` 說明缺了什麼。
pub(crate) fn incomplete_stream(label: &str, missing: &str, received: &str) -> TranslateError {
    log(format!(
        "[translate] stream ended before completion ({missing}): {label} received_chars={}",
        received.chars().count()
    ));
    TranslateError::offline(INCOMPLETE_STREAM)
}

fn quoted_sample(text: &str) -> String {
    repr_str(&sample_of(text).replace('\n', " "))
}

/// 在網址後加上一個編碼過的查詢參數（翻頁游標）。
pub(crate) fn with_query(url: &str, name: &str, value: &str) -> reqwest::Url {
    reqwest::Url::parse_with_params(url, &[(name, value)]).expect("valid endpoint URL")
}

/// 解析成 JSON 物件；其他形狀（含非 JSON）回 `Unexpected`，`what` 是被解析的東西，供訊息辨認。
pub(crate) fn parse_object(text: &str, what: &str) -> Result<Value, TranslateError> {
    serde_json::from_str(text)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| {
            TranslateError::Unexpected(format!(
                "{what} is not a JSON object: {}",
                repr_str(&sample_of(text))
            ))
        })
}

/// 模型原始輸出 → 最終譯文：去掉思考區塊與頭尾空白。
pub(crate) fn finish_text(raw: &str) -> String {
    pystr::strip(&strip_think(raw)).to_owned()
}

/// 一次串流讀完的結果（OpenAI／Gemini 共用）。
#[derive(Default)]
pub(crate) struct Streamed {
    pub content: String,
    pub finish_reason: Option<String>,
    pub usage: Option<Value>,
}

/// 診斷用樣本：開頭 [`TRUNCATED_SAMPLE_CHARS`] 個字元，足以看出是不是同一個字重複。
pub(crate) fn sample_of(text: &str) -> String {
    text.chars().take(TRUNCATED_SAMPLE_CHARS).collect()
}

/// API 回應裡的整數欄位轉成 log 用文字；缺欄位或 null 印成 `unknown`。
pub(crate) fn count_of(value: Option<&Value>) -> String {
    value
        .filter(|v| !v.is_null())
        .map_or("unknown".to_owned(), Value::to_string)
}

/// 連線層失敗視為離線；網址組不出請求（builder 錯誤）是設定錯誤。去掉 URL 與金鑰可能出現的位置，只留原因鏈。
pub(crate) fn connection_error(error: reqwest::Error) -> TranslateError {
    if error.is_builder() {
        return TranslateError::Config {
            status: None,
            detail: error_chain(error),
        };
    }
    TranslateError::offline(error_chain(error))
}

/// 送出或讀取請求時的連線層失敗（[`connection_error`]）；逾時另記一行帶服務商與模型（`label`）的 log。
pub(crate) fn request_error(label: &str, error: reqwest::Error) -> TranslateError {
    let timed_out = error.is_timeout();
    let error = connection_error(error);
    if timed_out {
        log(format!("[translate] request timed out: {label} ({error})"));
    }
    error
}

/// 共用的 HTTP client：連線逾時 [`TIMEOUT`]、讀取逾時 `read`，不跟隨轉址（與 httpx 預設一致）。
///
/// 閒置連線不設期限：死連線由 HTTP/2 PING 與 reqwest 預設的 TCP keepalive 偵測丟棄。
pub(crate) fn http_client(
    headers: reqwest::header::HeaderMap,
    base_url: &str,
    read: Duration,
) -> reqwest::Client {
    install_crypto_provider();
    let builder = reqwest::Client::builder()
        .connect_timeout(TIMEOUT)
        .read_timeout(read)
        .redirect(reqwest::redirect::Policy::none())
        .pool_idle_timeout(None)
        .http2_keep_alive_interval(KEEPALIVE_INTERVAL)
        .http2_keep_alive_timeout(KEEPALIVE_TIMEOUT)
        .http2_keep_alive_while_idle(true)
        .default_headers(headers);
    bypass_proxy_for_loopback(builder, base_url)
        .build()
        .expect("build HTTP client")
}

/// 跑 `work`；`cancel` 觸發時丟掉它（連線隨之關閉，伺服器停止生成）並回 `Cancelled`。
pub(crate) async fn cancellable<T>(
    cancel: Option<&CancellationToken>,
    work: impl Future<Output = Result<T, TranslateError>>,
) -> Result<T, TranslateError> {
    let Some(cancel) = cancel else {
        return work.await;
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(TranslateError::Cancelled),
        result = work => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_client_builds_with_the_ring_provider() {
        // 沒裝 crypto provider 的話 reqwest 在 build 時直接 panic
        let _client = http_client(HeaderMap::new(), "https://example.com", TIMEOUT);
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }

    #[test]
    fn truncated_error_quotes_the_sample_like_python_repr() {
        let error = truncated(64, None, "it's\na loop");
        assert_eq!(
            error,
            TranslateError::BadOutput(
                "output truncated at max_tokens=64, completion_tokens=unknown, sample=\"it's a loop\""
                    .into()
            )
        );
    }

    fn headers(name: &'static str, value: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(name, HeaderValue::from_str(value).unwrap());
        map
    }

    fn gemini_body(delay: &str) -> String {
        serde_json::json!({"error": {"code": 429, "details": [
            {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": delay}]}})
        .to_string()
    }

    #[test]
    fn abnormal_retry_after_values_are_no_hint() {
        let past = "Sun, 06 Nov 1994 08:49:37 GMT";
        for value in ["inf", "NaN", "1e400", "-5", "abc", "0", past] {
            assert_eq!(retry_after(&headers("retry-after", value)), None, "{value}");
        }
        for value in ["inf", "NaN", "-5", "abc", "0"] {
            assert_eq!(
                retry_after(&headers("retry-after-ms", value)),
                None,
                "{value}"
            );
        }
        for delay in ["-5s", "abc", "infs", "NaNs", "0s", "5"] {
            assert_eq!(body_retry_delay(&gemini_body(delay)), None, "{delay}");
        }
    }

    #[test]
    fn huge_retry_after_values_wait_the_cap() {
        assert_eq!(
            retry_after(&headers("retry-after", "1e20")),
            Some(MAX_RETRY_AFTER)
        );
        assert_eq!(
            retry_after(&headers("retry-after-ms", "1e25")),
            Some(MAX_RETRY_AFTER)
        );
        assert_eq!(
            body_retry_delay(&gemini_body("1e20s")),
            Some(MAX_RETRY_AFTER)
        );
        assert_eq!(
            body_retry_delay(&gemini_body("37s")),
            Some(Duration::from_secs(37))
        );
    }
}
