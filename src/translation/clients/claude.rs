//! Claude 官方 API：串流的 `/v1/messages`（結構化輸出）與 `/v1/models`；請求內容與重試策略照 anthropic SDK。

use std::ops::ControlFlow;
use std::time::Duration;

use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::sse::for_each_data;
use super::{
    ChatRequest, MAX_TOKENS_THINKING, ModelInfo, REQUEST_TIMEOUT, TIMEOUT, ThinkingCaps, blocked,
    cancellable, connection_error, count_of, error_type_status, finish_text, http_client,
    incomplete_stream, parse_object, request_error, response_failure, retry_after_within_cap,
    sample_of, secret_header, stream_failure, truncated, with_query,
};
use crate::config::services::{EFFORT_AUTO, provider};
use crate::logs::log;
use crate::py::pyrepr::repr_str;
use crate::translation::Turn;
use crate::translation::errors::{TranslateError, model_list_error, status_error};
use crate::util::random_u64;

/// Claude 官方端點。
pub const CLAUDE_BASE_URL: &str = "https://api.anthropic.com";

const API_VERSION: &str = "2023-06-01";
const MAX_RETRIES: u32 = 2;
const INITIAL_RETRY_DELAY: f64 = 0.5;
const MAX_RETRY_DELAY: f64 = 8.0;
const NO_AUTH: &str = "Could not resolve authentication method. Expected one of api_key, \
    auth_token, or credentials to be set.";

/// 回應限定成單一 JSON 字串欄位：Claude（尤其 Haiku）不思考時常在正文裡自我修正，限定後實測不再發生。
fn output_format() -> Value {
    json!({"type": "json_schema", "schema": {
        "type": "object", "properties": {"translation": {"type": "string"}},
        "required": ["translation"], "additionalProperties": false}})
}

/// 串流累積出的訊息（SDK `MessageStream.get_final_message` 的對應）。
#[derive(Default)]
struct Message {
    started: bool,
    /// 收到 `message_stop`。
    stopped: bool,
    /// 各內容區塊的 (type, text)。
    blocks: Vec<(String, String)>,
    stop_reason: Option<String>,
    usage: Option<Map<String, Value>>,
}

/// Claude 官方 API 的 client。
///
/// 不送 temperature（新模型不接受）。關閉思考送 `thinking: disabled`，能不能關由設定介面依模型清單判斷；
/// 深度由 effort 控制，`auto` 時連 `effort` 都不帶。
pub struct ClaudeClient {
    pub(super) http: reqwest::Client,
    pub(super) base_url: String,
    api_key: Option<HeaderValue>,
    /// 金鑰非空卻含 HTTP 標頭不允許的字元。
    key_malformed: bool,
    model: String,
    thinking: bool,
    effort: String,
    request_timeout: Duration,
}

fn default_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert("anthropic-version", HeaderValue::from_static(API_VERSION));
    headers
}

/// 從 `/v1/models` 的一筆讀出思考能力；欄位缺或型別不對就當成清單沒說。
fn thinking_caps(model: &Value) -> ThinkingCaps {
    let supported = |v: &Value| v.get("supported").and_then(Value::as_bool);
    let caps = &model["capabilities"];
    let effort = &caps["effort"];
    let levels = provider("claude").map_or(&[][..], |p| p.efforts);
    ThinkingCaps {
        supported: supported(&caps["thinking"]),
        can_disable: supported(&caps["thinking"]["types"]["disabled"]),
        efforts: supported(effort).map(|any| {
            levels
                .iter()
                .filter(|level| any && supported(&effort[**level]) == Some(true))
                .map(|level| (*level).to_owned())
                .collect()
        }),
    }
}

impl ClaudeClient {
    /// 建立 client；`endpoint` 正式環境為 [`CLAUDE_BASE_URL`]。
    pub fn new(api_key: &str, model: &str, thinking: bool, effort: &str, endpoint: &str) -> Self {
        let key = secret_header(api_key);
        Self {
            http: http_client(default_headers(), endpoint, TIMEOUT),
            base_url: endpoint.trim_end_matches('/').to_owned(),
            // SDK 遇到空金鑰在送出前就拋錯，含非法字元的金鑰也送不出去
            api_key: key.clone().filter(|_| !api_key.is_empty()),
            key_malformed: key.is_none() && !api_key.is_empty(),
            model: model.to_owned(),
            thinking,
            effort: effort.to_owned(),
            request_timeout: REQUEST_TIMEOUT,
        }
    }

    /// 測試用：縮短讀取逾時與聊天請求的總期限。
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_timeouts(mut self, read: Duration, total: Duration) -> Self {
        self.http = http_client(default_headers(), &self.base_url, read);
        self.request_timeout = total;
        self
    }

    /// 目前使用的模型 ID。
    pub fn model(&self) -> &str {
        &self.model
    }

    fn label(&self) -> String {
        format!("provider=claude model={}", self.model)
    }

    fn request_body(&self, system: &str, turns: &[Turn], max_tokens: u32) -> Value {
        let mut output_config = json!({"format": output_format()});
        if self.thinking && self.effort != EFFORT_AUTO {
            output_config["effort"] = json!(self.effort);
        }
        let messages: Vec<Value> = turns.iter().map(Turn::to_message).collect();
        // 斷點放在 system 結尾：之後的上下文每句都變，放在最後只會一直寫入、讀不到
        let mut body = json!({
            "max_tokens": max_tokens,
            "messages": messages,
            "model": self.model,
            "output_config": output_config,
            "system": [{"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}],
        });
        if !self.thinking {
            body["thinking"] = json!({"type": "disabled"});
        }
        body["stream"] = json!(true);
        body
    }

    /// 送出串流請求並解開結構化輸出的譯文；`cancel` 觸發即斷線。
    pub async fn chat(
        &self,
        req: ChatRequest<'_>,
        cancel: Option<&CancellationToken>,
    ) -> Result<String, TranslateError> {
        cancellable(cancel, self.chat_once(req)).await
    }

    async fn chat_once(&self, req: ChatRequest<'_>) -> Result<String, TranslateError> {
        // Claude 沒有「不思考」模式，未指定上限時沿用思考模式的預算
        let limit = req.max_tokens.unwrap_or(MAX_TOKENS_THINKING);
        let body = self.request_body(req.system, req.turns, limit);
        let url = format!("{}/v1/messages", self.base_url);
        let resp = self
            .send(|| {
                self.http
                    .post(&url)
                    .json(&body)
                    .timeout(self.request_timeout)
            })
            .await?;
        if !resp.status().is_success() {
            return Err(response_failure(resp, status_error).await);
        }
        let message = read_message(resp, &self.label()).await?;
        let stop_reason = message.stop_reason.as_deref().unwrap_or("unknown");
        if let Some(usage) = &message.usage {
            log(format!(
                // Claude 不分開列思考 token，思考算在 output_tokens 裡：遠多於譯文的長度就是有思考
                "[translate] prompt cache: provider=claude model={} read={} write={} uncached={} \
                 output_tokens={}",
                self.model,
                count_of(usage.get("cache_read_input_tokens")),
                count_of(usage.get("cache_creation_input_tokens")),
                count_of(usage.get("input_tokens")),
                count_of(usage.get("output_tokens")),
            ));
        }
        // 文件只保證第一個 text 區塊是合法 JSON；實測此模式下也只會有一個 text 區塊
        let content = message
            .blocks
            .iter()
            .find(|(kind, _)| kind == "text")
            .map_or("", |(_, text)| text.as_str());
        match stop_reason {
            "max_tokens" => {
                let output_tokens = message.usage.as_ref().and_then(|u| u.get("output_tokens"));
                return Err(truncated(limit, output_tokens, content));
            }
            "refusal" => return Err(blocked(stop_reason, content)),
            _ => {}
        }
        let translation = serde_json::from_str::<Value>(content)
            .ok()
            .and_then(|v| v.get("translation").cloned());
        let Some(translation) = translation else {
            log(format!(
                "[translate] claude output is not the expected JSON: model={} \
                 stop_reason={stop_reason} sample={}",
                self.model,
                repr_str(&sample_of(content))
            ));
            return Err(TranslateError::BadOutput(format!(
                "unexpected output (stop_reason={stop_reason})"
            )));
        };
        let text = translation.as_str().ok_or_else(|| {
            TranslateError::Unexpected(format!("translation is not a string: {translation}"))
        })?;
        Ok(finish_text(text))
    }

    /// 列出模型 ID（排序後），依 `has_more`／`last_id` 自動翻頁。
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, TranslateError> {
        let url = format!("{}/v1/models", self.base_url);
        let mut models = Vec::new();
        let mut after_id: Option<String> = None;
        loop {
            let page = self.models_page(&url, after_id.as_deref()).await?;
            let data = page.get("data").and_then(Value::as_array);
            models.extend(data.into_iter().flatten().filter_map(|m| {
                Some(ModelInfo {
                    id: m.get("id")?.as_str()?.to_owned(),
                    thinking: thinking_caps(m),
                })
            }));
            let last_id = page.get("last_id").and_then(Value::as_str);
            let more = page.get("has_more") != Some(&Value::Bool(false));
            match last_id {
                Some(id) if more && data.is_some_and(|d| !d.is_empty()) && !id.is_empty() => {
                    after_id = Some(id.to_owned());
                }
                _ => break,
            }
        }
        models.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(models)
    }

    async fn models_page(
        &self,
        url: &str,
        after_id: Option<&str>,
    ) -> Result<Value, TranslateError> {
        let resp = self
            .send(|| match after_id {
                Some(id) => self.http.get(with_query(url, "after_id", id)),
                None => self.http.get(url),
            })
            .await?;
        if !resp.status().is_success() {
            return Err(response_failure(resp, model_list_error).await);
        }
        let text = resp.text().await.map_err(connection_error)?;
        parse_object(&text, "models response")
    }

    /// 帶金鑰送出請求並照 SDK 預設重試：連線失敗與 [`should_retry`] 的狀態碼最多重試兩次。
    /// 只重試到拿到回應標頭為止，串流讀到一半斷線不重送。
    async fn send(
        &self,
        request: impl Fn() -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, TranslateError> {
        let Some(key) = self.api_key.clone() else {
            if self.key_malformed {
                log("[translate] claude api key has characters invalid in an HTTP header");
                return Err(TranslateError::invalid_key());
            }
            return Err(TranslateError::Unexpected(NO_AUTH.into()));
        };
        let mut retries_taken = 0;
        loop {
            let (reason, delay) = match request().header("x-api-key", key.clone()).send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if resp.status().is_success()
                        || !should_retry(status, resp.headers())
                        || retries_taken == MAX_RETRIES
                    {
                        return Ok(resp);
                    }
                    let delay = retry_delay(retries_taken, resp.headers(), random_unit());
                    (format!("HTTP {status}"), delay)
                }
                Err(error) => {
                    let error = request_error(&self.label(), error);
                    if retries_taken == MAX_RETRIES {
                        log(format!(
                            "[translate] claude connection failed after retries: {error}"
                        ));
                        return Err(error);
                    }
                    let reason = error.to_string();
                    let delay = retry_delay(retries_taken, &HeaderMap::new(), random_unit());
                    (reason, delay)
                }
            };
            retries_taken += 1;
            log(format!(
                "[translate] claude request failed ({reason}, model={}); \
                 retry {retries_taken}/{MAX_RETRIES} in {:.2}s",
                self.model,
                delay.as_secs_f64()
            ));
            tokio::time::sleep(delay).await;
        }
    }
}

/// SDK 的重試判定：`x-should-retry` 優先，其次 408／409／429／5xx。
fn should_retry(status: u16, headers: &HeaderMap) -> bool {
    match headers.get("x-should-retry").map(HeaderValue::as_bytes) {
        Some(b"true") => true,
        Some(b"false") => false,
        _ => matches!(status, 408 | 409 | 429 | 500..),
    }
}

/// 第 `retries_taken` 次失敗後的等待：照伺服器要求，否則 0.5 秒起指數退避（上限 8 秒）乘上 0.75–1 的抖動。
fn retry_delay(retries_taken: u32, headers: &HeaderMap, unit: f64) -> Duration {
    if let Some(wait) = retry_after_within_cap(headers) {
        return wait;
    }
    let backoff = (INITIAL_RETRY_DELAY * 2f64.powi(retries_taken as i32)).min(MAX_RETRY_DELAY);
    Duration::from_secs_f64((backoff * (1.0 - 0.25 * unit)).max(0.0))
}

/// `[0, 1)` 的亂數，給重試抖動用。
fn random_unit() -> f64 {
    (random_u64() >> 11) as f64 / (1u64 << 53) as f64
}

/// 串流中途的 `error` 事件：依 `error.type` 對應 HTTP 狀態碼分類，其餘類型為未分類的例外。
fn stream_error(event: &Value) -> TranslateError {
    let error = &event["error"];
    let status = error["type"].as_str().and_then(error_type_status);
    stream_failure(status, error["message"].as_str().unwrap_or(""), event)
}

/// 讀完 Messages 串流並累積成完整訊息。`error` 事件依類型分類，格式異常為未分類的例外。
async fn read_message(resp: reqwest::Response, label: &str) -> Result<Message, TranslateError> {
    let mut message = Message::default();
    for_each_data(resp, label, |data| {
        let event = parse_object(data, "stream event")?;
        accumulate(&mut message, &event)
    })
    .await?;
    if !message.stopped && message.stop_reason.is_none() {
        let received: String = message
            .blocks
            .iter()
            .filter(|(kind, _)| kind == "text")
            .map(|(_, text)| text.as_str())
            .collect();
        return Err(incomplete_stream(
            label,
            "no message_stop or stop_reason",
            &received,
        ));
    }
    Ok(message)
}

fn accumulate(message: &mut Message, event: &Value) -> Result<ControlFlow<()>, TranslateError> {
    let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
    let text_of = |block: &Value| {
        let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
        let text = block.get("text").and_then(Value::as_str).unwrap_or("");
        (kind.to_owned(), text.to_owned())
    };
    match kind {
        "error" => return Err(stream_error(event)),
        "message_start" if !message.started => {
            let start = &event["message"];
            message.started = true;
            message.blocks = start["content"]
                .as_array()
                .into_iter()
                .flatten()
                .map(text_of)
                .collect();
            message.stop_reason = start["stop_reason"].as_str().map(str::to_owned);
            message.usage = start["usage"].as_object().cloned();
        }
        "content_block_start"
        | "content_block_delta"
        | "content_block_stop"
        | "message_delta"
        | "message_stop"
            if !message.started =>
        {
            return Err(TranslateError::Unexpected(format!(
                "Unexpected event order, got {kind} before \"message_start\""
            )));
        }
        "message_stop" => {
            message.stopped = true;
            return Ok(ControlFlow::Break(()));
        }
        "content_block_start" => message.blocks.push(text_of(&event["content_block"])),
        "content_block_delta" if event["delta"]["type"] == "text_delta" => {
            let index = event["index"].as_u64().unwrap_or(u64::MAX) as usize;
            let (block_kind, text) = message.blocks.get_mut(index).ok_or_else(|| {
                TranslateError::Unexpected(format!("delta for unknown content block {index}"))
            })?;
            if block_kind == "text" {
                text.push_str(event["delta"]["text"].as_str().unwrap_or(""));
            }
        }
        "message_delta" => {
            message.stop_reason = event["delta"]["stop_reason"].as_str().map(str::to_owned);
            let usage = message.usage.get_or_insert_default();
            usage.insert(
                "output_tokens".into(),
                event["usage"]["output_tokens"].clone(),
            );
            // message_delta 的用量是累計值，缺席的欄位保留 message_start 的數字
            for name in [
                "input_tokens",
                "cache_creation_input_tokens",
                "cache_read_input_tokens",
            ] {
                if let Some(value) = event["usage"].get(name).filter(|v| !v.is_null()) {
                    usage.insert(name.into(), value.clone());
                }
            }
        }
        _ => {}
    }
    Ok(ControlFlow::Continue(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn backoff_doubles_from_half_a_second_with_quarter_jitter() {
        let none = HeaderMap::new();
        assert_eq!(retry_delay(0, &none, 0.0), Duration::from_millis(500));
        assert_eq!(retry_delay(1, &none, 0.0), Duration::from_secs(1));
        assert_eq!(retry_delay(0, &none, 1.0), Duration::from_millis(375));
        assert_eq!(retry_delay(1, &none, 0.5), Duration::from_millis(875));
        assert_eq!(retry_delay(10, &none, 0.0), Duration::from_secs(8));
    }

    #[test]
    fn retry_after_headers_override_backoff() {
        let delay = |pairs| retry_delay(0, &headers(pairs), 0.0);
        assert_eq!(delay(&[("retry-after", "3")]), Duration::from_secs(3));
        assert_eq!(delay(&[("retry-after", "0.2")]), Duration::from_millis(200));
        assert_eq!(
            delay(&[("retry-after-ms", "150"), ("retry-after", "5")]),
            Duration::from_millis(150)
        );
        assert_eq!(delay(&[("retry-after", "60")]), Duration::from_secs(60));
        // 非正數或認不出的值退回指數退避；retry-after-ms 解析成功就不再看 retry-after
        for pairs in [
            &[("retry-after", "0")][..],
            &[("retry-after", "-1")],
            &[("retry-after", "soon")],
            &[("retry-after", "61")],
            &[("retry-after", "1e20")],
            &[("retry-after-ms", "99999999")],
            &[("retry-after-ms", "0"), ("retry-after", "5")],
        ] {
            assert_eq!(delay(pairs), Duration::from_millis(500), "{pairs:?}");
        }
    }

    #[test]
    fn retry_after_accepts_an_http_date() {
        let date = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc2822();
        let seconds = retry_delay(0, &headers(&[("retry-after", &date)]), 0.0).as_secs_f64();
        assert!((28.0..=30.0).contains(&seconds), "{seconds}");
        let past = "Sun, 06 Nov 1994 08:49:37 GMT";
        assert_eq!(
            retry_delay(1, &headers(&[("retry-after", past)]), 0.0),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn should_retry_matches_the_sdk() {
        let none = HeaderMap::new();
        for status in [408, 409, 429, 500, 502, 529] {
            assert!(should_retry(status, &none), "{status}");
        }
        for status in [400, 401, 403, 404, 413, 422] {
            assert!(!should_retry(status, &none), "{status}");
        }
        assert!(!should_retry(503, &headers(&[("x-should-retry", "false")])));
        assert!(should_retry(400, &headers(&[("x-should-retry", "true")])));
    }

    #[test]
    fn random_unit_stays_in_range() {
        for _ in 0..1000 {
            let unit = random_unit();
            assert!((0.0..1.0).contains(&unit));
        }
    }
}
