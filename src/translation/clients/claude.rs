//! Claude 官方 API：串流的 `/v1/messages`（結構化輸出）與 `/v1/models`；請求內容與重試策略照 anthropic SDK。

use std::ops::ControlFlow;
use std::time::Duration;

use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::sse::for_each_data;
use super::{
    ChatRequest, MAX_TOKENS_THINKING, cancellable, connection_error, count_of, finish_text,
    http_client, parse_object, sample_of, secret_header, status_failure, truncated, with_query,
};
use crate::config::services::EFFORT_AUTO;
use crate::logs::log;
use crate::py::{pyrepr::repr_str, pystr};
use crate::translation::Turn;
use crate::translation::errors::{
    TranslateError, error_detail, model_list_error, one_line, status_error,
};
use crate::util::random_u64;

/// Claude 官方端點。
pub const CLAUDE_BASE_URL: &str = "https://api.anthropic.com";

const API_VERSION: &str = "2023-06-01";
const MAX_RETRIES: u32 = 2;
const INITIAL_RETRY_DELAY: f64 = 0.5;
const MAX_RETRY_DELAY: f64 = 8.0;
// 伺服器要求等更久就不理它，改走指數退避，交給池子的退避重試
const MAX_RETRY_AFTER: f64 = 60.0;
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
    /// 各內容區塊的 (type, text)。
    blocks: Vec<(String, String)>,
    stop_reason: Option<String>,
    usage: Option<Map<String, Value>>,
}

/// Claude 官方 API 的 client。
///
/// Claude 5 系不接受 temperature，也沒有「完全不思考」：深度由 effort 控制，`auto` 時連
/// `effort` 都不帶。刻意不送 `thinking: disabled` —— 那在 Opus 5 會把 `<thinking>` 漏進回應。
pub struct ClaudeClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<HeaderValue>,
    /// 金鑰非空卻含 HTTP 標頭不允許的字元。
    key_malformed: bool,
    model: String,
    effort: String,
}

impl ClaudeClient {
    /// 建立 client；`endpoint` 正式環境為 [`CLAUDE_BASE_URL`]。
    pub fn new(api_key: &str, model: &str, effort: &str, endpoint: &str) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert("anthropic-version", HeaderValue::from_static(API_VERSION));
        Self {
            http: http_client(headers, endpoint),
            base_url: endpoint.trim_end_matches('/').to_owned(),
            // SDK 遇到空金鑰在送出前就拋錯，含非法字元的金鑰也送不出去
            api_key: secret_header(api_key).filter(|_| !api_key.is_empty()),
            key_malformed: !api_key.is_empty() && secret_header(api_key).is_none(),
            model: model.to_owned(),
            effort: effort.to_owned(),
        }
    }

    /// 目前使用的模型 ID。
    pub fn model(&self) -> &str {
        &self.model
    }

    fn request_body(&self, system: &str, turns: &[Turn], max_tokens: u32) -> Value {
        let mut output_config = json!({"format": output_format()});
        if self.effort != EFFORT_AUTO {
            output_config["effort"] = json!(self.effort);
        }
        let messages: Vec<Value> = turns
            .iter()
            .map(|t| json!({"role": t.role.as_str(), "content": t.content}))
            .collect();
        // 斷點放在 system 結尾：之後的上下文每句都變，放在最後只會一直寫入、讀不到
        json!({
            "max_tokens": max_tokens,
            "messages": messages,
            "model": self.model,
            "output_config": output_config,
            "system": [{"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}],
            "stream": true,
        })
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
        let resp = self.send(|| self.http.post(&url).json(&body)).await?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let text = resp.text().await.map_err(connection_error)?;
            return Err(status_failure(status, error_detail(&text), status_error));
        }
        let message = read_message(resp).await?;
        let stop_reason = message.stop_reason.as_deref().unwrap_or("None");
        if let Some(usage) = &message.usage {
            log(format!(
                "[translate] prompt cache: provider=claude model={} read={} write={} uncached={}",
                self.model,
                count_of(usage.get("cache_read_input_tokens")),
                count_of(usage.get("cache_creation_input_tokens")),
                count_of(usage.get("input_tokens")),
            ));
        }
        // 文件只保證第一個 text 區塊是合法 JSON；實測此模式下也只會有一個 text 區塊
        let content = message
            .blocks
            .iter()
            .find(|(kind, _)| kind == "text")
            .map_or("", |(_, text)| text.as_str());
        if stop_reason == "max_tokens" {
            let output_tokens = message.usage.as_ref().and_then(|u| u.get("output_tokens"));
            return Err(truncated(limit, output_tokens, content));
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
    pub async fn list_models(&self) -> Result<Vec<String>, TranslateError> {
        let url = format!("{}/v1/models", self.base_url);
        let mut ids = Vec::new();
        let mut after_id: Option<String> = None;
        loop {
            let page = self.models_page(&url, after_id.as_deref()).await?;
            let data = page.get("data").and_then(Value::as_array);
            ids.extend(
                data.into_iter()
                    .flatten()
                    .filter_map(|m| m.get("id")?.as_str().map(str::to_owned)),
            );
            let last_id = page.get("last_id").and_then(Value::as_str);
            let more = page.get("has_more") != Some(&Value::Bool(false));
            match last_id {
                Some(id) if more && data.is_some_and(|d| !d.is_empty()) && !id.is_empty() => {
                    after_id = Some(id.to_owned());
                }
                _ => break,
            }
        }
        ids.sort();
        Ok(ids)
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
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(connection_error)?;
        if !(200..300).contains(&status) {
            return Err(status_failure(
                status,
                error_detail(&text),
                model_list_error,
            ));
        }
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
                return Err(TranslateError::Config {
                    status: Some(401),
                    detail: String::new(),
                });
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
                    if retries_taken == MAX_RETRIES {
                        let error = connection_error(error);
                        log(format!(
                            "[translate] claude connection failed after retries: {error}"
                        ));
                        return Err(error);
                    }
                    let reason = connection_error(error).to_string();
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

/// 伺服器要求的等待秒數：依序看 `retry-after-ms`、`retry-after`（秒數或 HTTP 日期）。
fn retry_after(headers: &HeaderMap) -> Option<f64> {
    let header = |name: &str| headers.get(name)?.to_str().ok().map(pystr::strip);
    if let Some(ms) = header("retry-after-ms").and_then(|v| v.parse::<f64>().ok()) {
        return Some(ms / 1000.0);
    }
    let value = header("retry-after")?;
    if let Ok(seconds) = value.parse::<f64>() {
        return Some(seconds);
    }
    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    Some((date.timestamp_millis() - chrono::Utc::now().timestamp_millis()) as f64 / 1000.0)
}

/// 第 `retries_taken` 次失敗後的等待：照伺服器要求，否則 0.5 秒起指數退避（上限 8 秒）乘上 0.75–1 的抖動。
fn retry_delay(retries_taken: u32, headers: &HeaderMap, unit: f64) -> Duration {
    if let Some(seconds) = retry_after(headers).filter(|s| *s > 0.0 && *s <= MAX_RETRY_AFTER) {
        return Duration::from_secs_f64(seconds);
    }
    let backoff = (INITIAL_RETRY_DELAY * 2f64.powi(retries_taken as i32)).min(MAX_RETRY_DELAY);
    Duration::from_secs_f64((backoff * (1.0 - 0.25 * unit)).max(0.0))
}

/// `[0, 1)` 的亂數，給重試抖動用。
fn random_unit() -> f64 {
    (random_u64() >> 11) as f64 / (1u64 << 53) as f64
}

/// 讀完 Messages 串流並累積成完整訊息。`error` 事件與格式異常都對應 Python 未分類的例外。
async fn read_message(resp: reqwest::Response) -> Result<Message, TranslateError> {
    let mut message = Message::default();
    for_each_data(resp, |data| {
        let event = parse_object(data, "stream event")?;
        accumulate(&mut message, &event)?;
        Ok(ControlFlow::Continue(()))
    })
    .await?;
    if !message.started {
        return Err(TranslateError::Unexpected(
            "stream ended without a message".into(),
        ));
    }
    Ok(message)
}

fn accumulate(message: &mut Message, event: &Value) -> Result<(), TranslateError> {
    let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
    let text_of = |block: &Value| {
        let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
        let text = block.get("text").and_then(Value::as_str).unwrap_or("");
        (kind.to_owned(), text.to_owned())
    };
    match kind {
        "error" => return Err(TranslateError::Unexpected(one_line(&event.to_string()))),
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
    Ok(())
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
            &[("retry-after-ms", "99999999")],
            &[("retry-after-ms", "0"), ("retry-after", "5")],
        ] {
            assert_eq!(delay(pairs), Duration::from_millis(500), "{pairs:?}");
        }
    }

    #[test]
    fn retry_after_accepts_an_http_date() {
        let date = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc2822();
        let seconds = retry_after(&headers(&[("retry-after", &date)])).unwrap();
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
