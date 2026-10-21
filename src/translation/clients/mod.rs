//! 各服務商的 HTTP client：統一的 [`Client`] 入口、請求逾時與 token 上限、可取消的請求。

mod claude;
mod gemini;
mod openai;
pub mod sse;

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub use claude::{CLAUDE_BASE_URL, ClaudeClient};
pub use gemini::{GEMINI_BASE_URL, GeminiClient};
pub use openai::OpenAiCompatClient;

use super::errors::{TranslateError, error_detail, status_error};
use super::postprocess::strip_think;
use super::{BoxFuture, Turn};
use crate::config::ServiceSpec;
use crate::net::{bypass_proxy_for_loopback, error_chain, install_crypto_provider};
use crate::py::pyrepr::repr_str;
use crate::py::pystr;

/// 請求逾時：連線與每次讀取各自計時，串流不受總長度限制。
pub const TIMEOUT: Duration = Duration::from_secs(60);

/// 譯文長度上限，用意是防 repetition loop 吃穿逾時而非省 token。
pub const MAX_TOKENS: u32 = 512;

/// 思考模式下推理本身就吃掉大量 token，上限需放寬才不會砍在譯文之前。
pub const MAX_TOKENS_THINKING: u32 = 4096;

/// 區域翻譯一次可能送整頁任務書，固定用放寬的上限。
pub const MAX_TOKENS_REGION: u32 = 4096;

/// ChatGPT preset 固定的官方端點。
pub const OPENAI_BASE_URL: &str = "https://api.openai.com";

/// 伺服器要求的等待上限：超過時 Claude 改走指數退避，翻譯池則夾在這個值。
pub(crate) const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

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
}

/// 依服務設定建立 client：`claude`／`gemini`／`openai` 走各自官方端點，其餘為自訂端點。
pub fn build_client(spec: &ServiceSpec) -> Client {
    let (key, model, effort) = (&spec.api_key, &spec.model, &spec.effort);
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

/// 錯誤狀態碼依 `map` 分類；分類不了（1xx／3xx 等）的對應 Python 未分類的例外。
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

/// 伺服器要求的等待時間：依序看 `retry-after-ms`、`retry-after`（秒數或 HTTP 日期）；非正數為 `None`。
pub(crate) fn retry_after(headers: &HeaderMap) -> Option<Duration> {
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
    positive_seconds(seconds)
}

fn positive_seconds(seconds: f64) -> Option<Duration> {
    Duration::try_from_secs_f64(seconds)
        .ok()
        .filter(|wait| !wait.is_zero())
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

/// 串流中途的 `{"error": …}` 塊：`code` 是數字（或數字字串）就照它分類，否則以 `code`／`type` 的名稱對照狀態碼。
pub(crate) fn chunk_error(error: &Value) -> TranslateError {
    let code = error.get("code");
    let numeric = code
        .and_then(|c| c.as_u64().or_else(|| c.as_str()?.parse().ok()))
        .and_then(|c| u16::try_from(c).ok());
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
        repr_str(&sample_of(sample).replace('\n', " "))
    ))
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

/// 共用的 HTTP client：連線與讀取逾時各 [`TIMEOUT`]，不跟隨轉址（與 httpx 預設一致）。
pub(crate) fn http_client(headers: reqwest::header::HeaderMap, base_url: &str) -> reqwest::Client {
    install_crypto_provider();
    let builder = reqwest::Client::builder()
        .connect_timeout(TIMEOUT)
        .read_timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
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
        let _client = http_client(reqwest::header::HeaderMap::new(), "https://example.com");
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
}
