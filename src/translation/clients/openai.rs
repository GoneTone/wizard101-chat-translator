//! OpenAI 相容端點（ChatGPT 官方與自訂伺服器共用）：串流的 `/v1/chat/completions`。

use std::collections::BTreeSet;
use std::ops::ControlFlow;
use std::sync::Mutex;

use reqwest::header::{AUTHORIZATION, HeaderMap};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::sse::for_each_data;
use super::{
    ChatRequest, MAX_TOKENS, MAX_TOKENS_THINKING, Streamed, cancellable, connection_error,
    count_of, finish_text, http_client, parse_object, secret_header, status_failure, truncated,
};
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
pub struct OpenAiCompatClient {
    http: reqwest::Client,
    base_url: String,
    model: String,
    thinking: bool,
    official: bool,
    learned: Mutex<Learned>,
}

impl OpenAiCompatClient {
    /// 建立 client；`api_key` 為空時不帶 Authorization。
    pub fn new(base_url: &str, api_key: &str, model: &str, thinking: bool, official: bool) -> Self {
        let mut headers = HeaderMap::new();
        if !api_key.is_empty() {
            match secret_header(&format!("Bearer {api_key}")) {
                Some(value) => {
                    headers.insert(AUTHORIZATION, value);
                }
                None => log(
                    "[translate] api key has characters invalid in an HTTP header; \
                     sending without Authorization",
                ),
            }
        }
        Self {
            http: http_client(headers, base_url),
            base_url: base_url.trim_end_matches('/').to_owned(),
            model: model.to_owned(),
            thinking,
            official,
            learned: Mutex::new(Learned {
                token_param: if official {
                    "max_completion_tokens"
                } else {
                    "max_tokens"
                },
                dropped: BTreeSet::new(),
                rejected_token_params: BTreeSet::new(),
            }),
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
        messages.extend(
            turns
                .iter()
                .map(|t| json!({"role": t.role.as_str(), "content": t.content})),
        );
        let mut body = Map::new();
        body.insert("model".into(), json!(self.model));
        body.insert("messages".into(), Value::Array(messages));
        body.insert(learned.token_param.into(), json!(max_tokens));
        body.insert("stream".into(), json!(true));
        if self.official {
            // 串流預設不回 usage；只對官方端點帶，自架後端不一定認得而冒 400 的險
            body.insert("stream_options".into(), json!({"include_usage": true}));
        } else if deterministic {
            body.insert("temperature".into(), json!(0));
        }
        if !self.thinking {
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

    /// 記住端點拒絕的參數，回傳下一次請求是否還有變化可試（長度上限兩個名字都被拒就沒招了）。
    fn learn_rejection(&self, param: &str) -> bool {
        let mut learned = lock(&self.learned);
        if let Some(other) = other_token_param(param) {
            learned.rejected_token_params.insert(param.to_owned());
            if learned.rejected_token_params.contains(other) {
                return false;
            }
            learned.token_param = other;
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
            let param = match &error {
                TranslateError::Config {
                    status: Some(400),
                    detail,
                } => rejected_parameter(detail),
                _ => None,
            };
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
            log(format!(
                "[translate] prompt cache: provider=openai-compatible model={} \
                 prompt={prompt} cached={cached}",
                self.model
            ));
        }
        // 部分後端不回 finish_reason，缺欄位一律視為正常結束、不誤判成截斷
        if streamed.finish_reason.as_deref() == Some("length") {
            return Err(truncated(
                max_tokens,
                usage.get("completion_tokens"),
                &streamed.content,
            ));
        }
        Ok(finish_text(&streamed.content))
    }

    async fn stream_once(&self, body: &Value) -> Result<Streamed, TranslateError> {
        let resp = self
            .http
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(body)
            .send()
            .await
            .map_err(connection_error)?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let text = resp.text().await.map_err(connection_error)?;
            return Err(status_failure(status, error_detail(&text), status_error));
        }
        let mut out = Streamed::default();
        for_each_data(resp, |data| {
            if data == "[DONE]" {
                return Ok(ControlFlow::Break(()));
            }
            let chunk = parse_object(data, "stream chunk")?;
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
        Ok(out)
    }

    /// 取 `/v1/models` 的模型 ID（排序後）；404／405 或缺 `data` 陣列回 `NoModelList`。
    pub async fn list_models(&self) -> Result<Vec<String>, TranslateError> {
        let resp = self
            .http
            .get(format!("{}/v1/models", self.base_url))
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
        let mut ids: Vec<String> = data
            .iter()
            .filter_map(|m| m.get("id")?.as_str().filter(|id| !id.is_empty()))
            .map(str::to_owned)
            .collect();
        ids.sort();
        Ok(ids)
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
