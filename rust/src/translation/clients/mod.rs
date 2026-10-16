//! 各服務商的 HTTP client：統一的 [`Client`] 入口、請求逾時與 token 上限、可取消的請求。

mod openai;
pub mod sse;

use std::error::Error as _;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

pub use openai::OpenAiCompatClient;

use super::errors::{TranslateError, one_line};
use super::{BoxFuture, Turn};
use crate::config::ServiceSpec;

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

    /// 列出端點的模型 ID（排序後）。
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<String>, TranslateError>>;

    /// 目前使用的模型 ID（診斷 log 用）。
    fn model(&self) -> &str;
}

/// 依服務商建立的 client。
pub enum Client {
    OpenAiCompat(OpenAiCompatClient),
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
        }
    }

    /// 列出端點的模型 ID（排序後）。
    pub async fn list_models(&self) -> Result<Vec<String>, TranslateError> {
        match self {
            Self::OpenAiCompat(c) => c.list_models().await,
        }
    }

    /// 目前使用的模型 ID。
    pub fn model(&self) -> &str {
        match self {
            Self::OpenAiCompat(c) => c.model(),
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

    fn list_models(&self) -> BoxFuture<'_, Result<Vec<String>, TranslateError>> {
        Box::pin(Client::list_models(self))
    }

    fn model(&self) -> &str {
        Client::model(self)
    }
}

/// 依服務設定建立 client；`claude`／`gemini` 尚未移植，回 `None`。
pub fn build_client(spec: &ServiceSpec) -> Option<Client> {
    let (base_url, official) = match spec.provider.as_str() {
        "claude" | "gemini" => return None,
        "openai" => (OPENAI_BASE_URL, true),
        _ => (spec.base_url.as_str(), false),
    };
    Some(Client::OpenAiCompat(OpenAiCompatClient::new(
        base_url,
        &spec.api_key,
        &spec.model,
        spec.thinking,
        official,
    )))
}

/// 連線層失敗一律視為離線；去掉 URL 與金鑰可能出現的位置，只留原因鏈。
pub(crate) fn connection_error(error: reqwest::Error) -> TranslateError {
    let error = error.without_url();
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    TranslateError::offline(one_line(&parts.join(": ")))
}

/// 共用的 HTTP client：連線與讀取逾時各 [`TIMEOUT`]，不跟隨轉址（與 httpx 預設一致）。
pub(crate) fn http_client(headers: reqwest::header::HeaderMap) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(TIMEOUT)
        .read_timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .default_headers(headers)
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
