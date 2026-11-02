//! OpenAI 相容端點（ChatGPT 官方與自訂伺服器共用）：串流的 `/v1/chat/completions`。

use std::collections::BTreeSet;
use std::ops::ControlFlow;
use std::sync::Mutex;
use std::time::Duration;

use reqwest::RequestBuilder;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::sse::for_each_data;
use super::{
    ChatRequest, MAX_TOKENS, MAX_TOKENS_THINKING, ModelInfo, REQUEST_TIMEOUT, Streamed, TIMEOUT,
    blocked, cancellable, chunk_error, connection_error, count_of, finish_text, http_client,
    incomplete_stream, parse_object, request_error, response_failure, secret_header, truncated,
};
use crate::config::services::EFFORT_AUTO;
use crate::logs::log;
use crate::translation::Turn;
use crate::translation::errors::{
    TranslateError, error_detail, model_list_error, rejected_parameter, status_error,
};
use crate::util::lock;

/// 長度上限的兩個名字互換而非拿掉：它是防 repetition loop 的保險。
fn other_token_param(name: &str) -> Option<&'static str> {
    match name {
        "max_tokens" => Some("max_completion_tokens"),
        "max_completion_tokens" => Some("max_tokens"),
        _ => None,
    }
}

/// 從端點的 400 學到的請求參數規則；留在 client 實例上，之後的請求沿用。
struct Learned {
    token_param: &'static str,
    dropped: BTreeSet<String>,
    rejected_token_params: BTreeSet<String>,
}

/// OpenAI 相容端點的 client。
///
/// 哪些參數能帶因模型而異，改成聽伺服器的：400 指名某個參數就拿掉（或互換長度上限的名字）重送。
/// `official` 為 OpenAI 官方端點的起手式：用 `max_completion_tokens`、不帶 temperature、
/// 只帶官方認得的停用思考參數；自架後端維持 `max_tokens` 與 `temperature=0`。
/// 使用者選的思考深度、以及模型關不掉思考時被拒的 `none`，不在「聽伺服器的」範圍內：直接回錯誤。
pub struct OpenAiCompatClient {
    pub(super) http: reqwest::Client,
    auth: Option<HeaderValue>,
    pub(super) base_url: String,
    model: String,
    thinking: bool,
    effort: String,
    official: bool,
    /// 串流要求回 usage（官方端點一律要）。
    include_usage: bool,
    learned: Mutex<Learned>,
    request_timeout: Duration,
}

impl OpenAiCompatClient {
    /// 建立 client；`api_key` 為空時不帶 Authorization。
    pub fn new(
        base_url: &str,
        api_key: &str,
        model: &str,
        thinking: bool,
        effort: &str,
        official: bool,
    ) -> Self {
        let auth = secret_header(&format!("Bearer {api_key}")).filter(|_| !api_key.is_empty());
        if auth.is_none() && !api_key.is_empty() {
            log(
                "[translate] api key has characters invalid in an HTTP header; \
                 sending without Authorization",
            );
        }
        Self {
            http: http_client(HeaderMap::new(), base_url, TIMEOUT),
            auth,
            base_url: base_url.trim_end_matches('/').to_owned(),
            model: model.to_owned(),
            thinking,
            effort: effort.to_owned(),
            official,
            include_usage: official,
            learned: Mutex::new(Learned {
                token_param: if official {
                    "max_completion_tokens"
                } else {
                    "max_tokens"
                },
                dropped: BTreeSet::new(),
                rejected_token_params: BTreeSet::new(),
            }),
            request_timeout: REQUEST_TIMEOUT,
        }
    }

    /// 測試用：縮短讀取逾時與聊天請求的總期限。
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_timeouts(mut self, read: Duration, total: Duration) -> Self {
        self.http = http_client(HeaderMap::new(), &self.base_url, read);
        self.request_timeout = total;
        self
    }

    /// 串流要求回 usage（prompt cache 用量）；只給確定認得 `stream_options` 的後端用。
    pub fn with_usage(mut self) -> Self {
        self.include_usage = true;
        self
    }

    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        match &self.auth {
            Some(auth) => request.header(AUTHORIZATION, auth.clone()),
            None => request,
        }
    }

    /// 目前使用的模型 ID。
    pub fn model(&self) -> &str {
        &self.model
    }

    /// 組出請求 JSON，已套用從端點學到的參數規則。
    pub fn request_body(
        &self,
        system: &str,
        turns: &[Turn],
        max_tokens: u32,
        deterministic: bool,
    ) -> Value {
        let learned = lock(&self.learned);
        let mut messages = vec![json!({"role": "system", "content": system})];
        messages.extend(turns.iter().map(Turn::to_message));
        let mut body = Map::new();
        body.insert("model".into(), json!(self.model));
        body.insert("messages".into(), Value::Array(messages));
        body.insert(learned.token_param.into(), json!(max_tokens));
        body.insert("stream".into(), json!(true));
        // 串流預設不回 usage；自架後端不一定認得而冒 400 的險，只對確定認得的帶
        if self.include_usage {
            body.insert("stream_options".into(), json!({"include_usage": true}));
        }
        if !self.official && deterministic {
            body.insert("temperature".into(), json!(0));
        }
        if self.chose_effort() {
            body.insert("reasoning_effort".into(), json!(self.effort));
        } else if !self.thinking {
            body.insert("reasoning_effort".into(), json!("none"));
            if !self.official {
                // 涵蓋常見後端：vLLM／SGLang＋Qwen3、Ollama、部分吃頂層的伺服器
                body.insert(
                    "chat_template_kwargs".into(),
                    json!({"enable_thinking": false}),
                );
                body.insert("think".into(), json!(false));
                body.insert("enable_thinking".into(), json!(false));
            }
        }
        for name in &learned.dropped {
            body.remove(name);
        }
        Value::Object(body)
    }

    /// 開著思考且選了具體深度（不是自動）。
    fn chose_effort(&self) -> bool {
        self.thinking && self.effort != EFFORT_AUTO
    }

    /// 記住端點拒絕的參數，回傳下一次請求是否還有變化可試。
    ///
    /// 長度上限兩個名字都被拒、或被拒的是核心參數（model／messages／stream）時沒招了。
    fn learn_rejection(&self, param: &str) -> bool {
        let mut learned = lock(&self.learned);
        if let Some(other) = other_token_param(param) {
            learned.rejected_token_params.insert(param.to_owned());
            if learned.rejected_token_params.contains(other) {
                return false;
            }
            learned.token_param = other;
        } else if matches!(param, "model" | "messages" | "stream") {
            return false;
        } else {
            learned.dropped.insert(param.to_owned());
        }
        true
    }

    /// 送出串流聊天請求並接回整段譯文；`cancel` 觸發即斷線。
    pub async fn chat(
        &self,
        req: ChatRequest<'_>,
        cancel: Option<&CancellationToken>,
    ) -> Result<String, TranslateError> {
        let max_tokens = req.max_tokens.unwrap_or(if self.thinking {
            MAX_TOKENS_THINKING
        } else {
            MAX_TOKENS
        });
        let streamed = loop {
            let body = self.request_body(req.system, req.turns, max_tokens, req.deterministic);
            let error = match cancellable(cancel, self.stream_once(&body)).await {
                Ok(streamed) => break streamed,
                Err(error) => error,
            };
            let (param, bad_value) = match &error {
                TranslateError::Config {
                    status: Some(400),
                    detail,
                } => (
                    rejected_parameter(detail),
                    detail.contains("Unsupported value"),
                ),
                _ => (None, false),
            };
            // 認得參數卻拒收這個值（會思考但關不掉）就報錯；不認得參數代表模型本來就不思考，照常拿掉
            if param.as_deref() == Some("reasoning_effort") && (self.chose_effort() || bad_value) {
                let effort = if self.thinking { &*self.effort } else { "none" };
                log(format!(
                    "[translate] endpoint rejected the chosen reasoning_effort={effort} (model={}): {error}",
                    self.model
                ));
                return Err(error);
            }
            // 每輪都拿掉（或換掉）一個 body 裡確實有的參數，且同一個名字不會試第二次
            match param {
                Some(param) if body.get(&param).is_some() && self.learn_rejection(&param) => {
                    log(format!(
                        "[translate] endpoint rejected parameter {param} (model={}); \
                         retrying without it",
                        self.model
                    ));
                }
                _ => return Err(error),
            }
        };
        let usage = streamed.usage.unwrap_or(Value::Null);
        if let Some(prompt) = usage.get("prompt_tokens") {
            let cached = count_of(usage.pointer("/prompt_tokens_details/cached_tokens"));
            let reasoning = count_of(usage.pointer("/completion_tokens_details/reasoning_tokens"));
            log(format!(
                "[translate] prompt cache: provider=openai-compatible model={} \
                 prompt={prompt} cached={cached} reasoning_tokens={reasoning}",
                self.model
            ));
        }
        // 串流已確認收完（`[DONE]` 或 finish_reason），沒有 finish_reason 的後端照正常結束處理
        match streamed.finish_reason.as_deref() {
            Some("length") => {
                return Err(truncated(
                    max_tokens,
                    usage.get("completion_tokens"),
                    &streamed.content,
                ));
            }
            Some(reason @ "content_filter") => return Err(blocked(reason, &streamed.content)),
            _ => {}
        }
        Ok(finish_text(&streamed.content))
    }

    async fn stream_once(&self, body: &Value) -> Result<Streamed, TranslateError> {
        let label = format!("provider=openai-compatible model={}", self.model);
        let resp = self
            .authorized(
                self.http
                    .post(format!("{}/v1/chat/completions", self.base_url)),
            )
            .json(body)
            .timeout(self.request_timeout)
            .send()
            .await
            .map_err(|error| request_error(&label, error))?;
        if !resp.status().is_success() {
            return Err(response_failure(resp, status_error).await);
        }
        let mut out = Streamed::default();
        let mut done = false;
        for_each_data(resp, &label, |data| {
            if data == "[DONE]" {
                done = true;
                return Ok(ControlFlow::Break(()));
            }
            let chunk = parse_object(data, "stream chunk")?;
            // 200 之後的錯誤以 chunk 送出；有些後端固定帶 `"error": null`
            if let Some(error) = chunk.get("error").filter(|e| !e.is_null()) {
                return Err(chunk_error(error));
            }
            if let Some(choice) = chunk.pointer("/choices/0") {
                if let Some(piece) = choice.pointer("/delta/content").and_then(Value::as_str) {
                    out.content.push_str(piece);
                }
                if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str)
                    && !reason.is_empty()
                {
                    out.finish_reason = Some(reason.to_owned());
                }
            }
            if let Some(usage) = chunk
                .get("usage")
                .filter(|u| u.as_object().is_some_and(|o| !o.is_empty()))
            {
                out.usage = Some(usage.clone());
            }
            Ok(ControlFlow::Continue(()))
        })
        .await?;
        // 有些後端只送 `[DONE]`、有些只給 finish_reason，任一個就算完整
        if !done && out.finish_reason.is_none() {
            return Err(incomplete_stream(
                &label,
                "no [DONE] or finish_reason",
                &out.content,
            ));
        }
        Ok(out)
    }

    /// 取 `/v1/models` 的模型 ID（排序後）；404／405 或缺 `data` 陣列回 `NoModelList`。
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, TranslateError> {
        let resp = self
            .authorized(self.http.get(format!("{}/v1/models", self.base_url)))
            .send()
            .await
            .map_err(connection_error)?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(connection_error)?;
        if let Some(error) = model_list_error(status, &error_detail(&text)) {
            return Err(error);
        }
        if !(200..300).contains(&status) {
            return Err(TranslateError::Unexpected(format!("HTTP {status}")));
        }
        let body = parse_object(&text, "models response")?;
        let data = body
            .get("data")
            .and_then(Value::as_array)
            .ok_or(TranslateError::NoModelList)?;
        let mut models: Vec<ModelInfo> = data
            .iter()
            .filter_map(|m| m.get("id")?.as_str().filter(|id| !id.is_empty()))
            .map(ModelInfo::bare)
            .collect();
        models.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(models)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::ServiceSpec;
    use crate::translation::clients::{Client, OPENAI_BASE_URL, build_client};

    #[test]
    fn openai_provider_forces_official_base_url() {
        let spec = ServiceSpec {
            provider: "openai".into(),
            base_url: "http://evil.example".into(),
            model: "m".into(),
            api_key: "k".into(),
            ..ServiceSpec::default()
        };
        let Client::OpenAiCompat(client) = build_client(&spec) else {
            panic!("openai provider must use the OpenAI-compatible client");
        };
        assert_eq!(client.base_url, OPENAI_BASE_URL);
    }
}
