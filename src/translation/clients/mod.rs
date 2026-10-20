//! 各服務商的 HTTP client：統一的 [`Client`] 入口、請求逾時與 token 上限、可取消的請求。

mod claude;
mod gemini;
mod openai;
pub mod sse;

use std::error::Error as _;
use std::time::Duration;

use reqwest::header::HeaderValue;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub use claude::{CLAUDE_BASE_URL, ClaudeClient};
pub use gemini::{GEMINI_BASE_URL, GeminiClient};
pub use openai::OpenAiCompatClient;

use super::errors::{TranslateError, status_error};
use super::postprocess::strip_think;
use super::{BoxFuture, Turn};
use crate::config::ServiceSpec;
use crate::logs::log;
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
    /// 送出聊天請求並回傳整段譯文。
    pub async fn chat(
        &self,
        req: ChatRequest<'_>,
        cancel: Option<&CancellationToken>,
    ) -> Result<String, TranslateError> {
        match self {
            Self::OpenAiCompat(c) => c.chat(req, cancel).await,
            Self::Claude(c) => c.chat(req, cancel).await,
            Self::Gemini(c) => c.chat(req, cancel).await,
        }
    }

    /// 列出端點的模型 ID（排序後）。
    pub async fn list_models(&self) -> Result<Vec<String>, TranslateError> {
        match self {
            Self::OpenAiCompat(c) => c.list_models().await,
            Self::Claude(c) => c.list_models().await,
            Self::Gemini(c) => c.list_models().await,
        }
    }

    /// 目前使用的模型 ID。
    pub fn model(&self) -> &str {
        match self {
            Self::OpenAiCompat(c) => c.model(),
            Self::Claude(c) => c.model(),
            Self::Gemini(c) => c.model(),
        }
    }
}

impl ChatBackend for Client {
    fn chat<'a>(
        &'a self,
        req: ChatRequest<'a>,
        cancel: Option<&'a CancellationToken>,
    ) -> BoxFuture<'a, Result<String, TranslateError>> {
        Box::pin(Client::chat(self, req, cancel))
    }

    fn model(&self) -> &str {
        Client::model(self)
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

/// 串流中途的 `{"error": …}` 塊：依其中的 code 分類，沒有可用的 code 對應未分類的例外。
pub(crate) fn chunk_error(error: &Value) -> TranslateError {
    let message = error.get("message").and_then(Value::as_str).unwrap_or("");
    match error
        .get("code")
        .and_then(Value::as_u64)
        .and_then(|c| u16::try_from(c).ok())
    {
        Some(code) => status_failure(code, pystr::one_line(message), status_error),
        None => TranslateError::Unexpected(pystr::one_line(&error.to_string())),
    }
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

/// 連線層失敗一律視為離線；去掉 URL 與金鑰可能出現的位置，只留原因鏈。
pub(crate) fn connection_error(error: reqwest::Error) -> TranslateError {
    TranslateError::offline(error_chain(error))
}

/// reqwest 錯誤去掉 URL 後，連同整條原因鏈折成一行（頂層訊息常只有 "error sending request"）。
pub(crate) fn error_chain(error: reqwest::Error) -> String {
    let error = error.without_url();
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    pystr::one_line(&parts.join(": "))
}

/// 裝上 rustls 的 ring crypto provider；任何要建 HTTPS client 的地方都得先呼叫。
pub(crate) fn install_crypto_provider() {
    // 用 ring 而非 rustls 預設的 aws-lc-rs（建置要 cmake／NASM）；已裝過時回 Err，忽略即可
    let _ = rustls::crypto::ring::default_provider().install_default();
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

/// 連往本機（`localhost`、`127.0.0.0/8`、`::1`）時不用環境變數設定的 proxy：本機的 LLM 伺服器繞去 proxy 只會連不到。
pub(crate) fn bypass_proxy_for_loopback(
    builder: reqwest::ClientBuilder,
    url: &str,
) -> reqwest::ClientBuilder {
    if !is_loopback(url) {
        return builder;
    }
    let proxy_vars = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"];
    if proxy_vars.iter().any(|name| {
        std::env::var_os(name).is_some() || std::env::var_os(name.to_lowercase()).is_some()
    }) {
        log(format!(
            "[translate] {url} is a loopback address; not using the proxy from the environment"
        ));
    }
    builder.no_proxy()
}

fn is_loopback(url: &str) -> bool {
    let Some(host) = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
    else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
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
    fn loopback_hosts_are_recognised() {
        for url in [
            "http://localhost:11434/v1",
            "http://LOCALHOST:1234",
            "http://127.0.0.1:8080/v1",
            "http://127.5.6.7",
            "http://[::1]:11434/v1",
        ] {
            assert!(is_loopback(url), "{url}");
        }
        for url in [
            "https://api.anthropic.com",
            "http://192.168.1.10:11434/v1",
            "http://localhost.example.com",
            "not a url",
        ] {
            assert!(!is_loopback(url), "{url}");
        }
    }

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
