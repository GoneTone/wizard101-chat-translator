//! Gemini 官方 API：串流的 `streamGenerateContent?alt=sse` 與模型清單；請求內容照 google-genai SDK（預設不重試）。

use std::ops::ControlFlow;

use reqwest::header::HeaderValue;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::sse::for_each_data;
use super::{
    ChatRequest, MAX_TOKENS_THINKING, Streamed, cancellable, chunk_error, connection_error,
    count_of, finish_text, http_client, parse_object, response_failure, secret_header, truncated,
    with_query,
};
use crate::config::services::EFFORT_AUTO;
use crate::logs::log;
use crate::py::pystr;
use crate::translation::errors::{TranslateError, model_list_error, status_error};
use crate::translation::{Role, Turn};

/// Gemini 官方端點。
pub const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";

const API_VERSION: &str = "v1beta";

/// Gemini 官方 API 的 client。
///
/// 不送 temperature —— Google 建議 Gemini 3 維持預設，調低反而容易重複輸出。3 系列與 2.5 Pro
/// 關不掉思考，故用 effort：`auto` 不帶 `thinkingConfig`，其餘送 `thinking_level`。
pub struct GeminiClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<HeaderValue>,
    model: String,
    effort: String,
}

impl GeminiClient {
    /// 建立 client；`endpoint` 正式環境為 [`GEMINI_BASE_URL`]。
    ///
    /// 金鑰照 SDK 去頭尾空白後放 `x-goog-api-key` 標頭（空金鑰送空值，讓伺服器回 4xx 照設定錯誤處理）。
    pub fn new(api_key: &str, model: &str, effort: &str, endpoint: &str) -> Self {
        Self {
            http: http_client(Default::default(), endpoint),
            base_url: endpoint.trim_end_matches('/').to_owned(),
            api_key: secret_header(pystr::strip(api_key)),
            model: model.to_owned(),
            effort: effort.to_owned(),
        }
    }

    /// 目前使用的模型 ID。
    pub fn model(&self) -> &str {
        &self.model
    }

    fn request_body(&self, system: &str, turns: &[Turn], max_tokens: u32) -> Value {
        let contents: Vec<Value> = turns
            .iter()
            .map(|t| {
                let role = if t.role == Role::Assistant {
                    "model"
                } else {
                    "user"
                };
                json!({"parts": [{"text": t.content}], "role": role})
            })
            .collect();
        let mut config = json!({"maxOutputTokens": max_tokens});
        if self.effort != EFFORT_AUTO {
            // SDK 把 thinking_level 轉成大寫列舉值，鍵名維持 snake_case
            config["thinkingConfig"] = json!({"thinking_level": self.effort.to_ascii_uppercase()});
        }
        json!({
            "contents": contents,
            "systemInstruction": {"parts": [{"text": system}], "role": "user"},
            "generationConfig": config,
        })
    }

    /// 送出串流請求並把各塊文字接回整段；`cancel` 觸發即斷線。
    pub async fn chat(
        &self,
        req: ChatRequest<'_>,
        cancel: Option<&CancellationToken>,
    ) -> Result<String, TranslateError> {
        cancellable(cancel, self.chat_once(req)).await
    }

    async fn chat_once(&self, req: ChatRequest<'_>) -> Result<String, TranslateError> {
        let limit = req.max_tokens.unwrap_or(MAX_TOKENS_THINKING);
        let url = format!(
            "{}/{API_VERSION}/{}:streamGenerateContent?alt=sse",
            self.base_url,
            model_path(&self.model)?
        );
        let body = self.request_body(req.system, req.turns, limit);
        let resp = self.send(self.http.post(url).json(&body)).await?;
        let mut out = Streamed::default();
        for_each_data(resp, |data| {
            let chunk = parse_object(data, "stream chunk")?;
            if let Some(error) = chunk.get("error") {
                return Err(chunk_error(error));
            }
            let candidate = chunk.pointer("/candidates/0");
            let parts = candidate.and_then(|c| c.pointer("/content/parts"));
            for part in parts.and_then(Value::as_array).into_iter().flatten() {
                if part.get("thought") != Some(&Value::Bool(true))
                    && let Some(text) = part.get("text").and_then(Value::as_str)
                {
                    out.content.push_str(text);
                }
            }
            if let Some(reason) = candidate
                .and_then(|c| c.get("finishReason"))
                .and_then(Value::as_str)
                .filter(|r| !r.is_empty())
            {
                out.finish_reason = Some(reason.to_owned());
            }
            if let Some(usage) = chunk.get("usageMetadata").filter(|u| !u.is_null()) {
                out.usage = Some(usage.clone());
            }
            Ok(ControlFlow::Continue(()))
        })
        .await?;
        if let Some(usage) = &out.usage {
            log(format!(
                "[translate] prompt cache: provider=gemini model={} prompt={} cached={}",
                self.model,
                count_of(usage.get("promptTokenCount")),
                count_of(usage.get("cachedContentTokenCount")),
            ));
        }
        if out.finish_reason.as_deref() == Some("MAX_TOKENS") {
            let output_tokens = out
                .usage
                .as_ref()
                .and_then(|u| u.get("candidatesTokenCount"));
            return Err(truncated(limit, output_tokens, &out.content));
        }
        if out.content.is_empty() {
            let reason = out.finish_reason.unwrap_or_else(|| "unknown".to_owned());
            log(format!(
                "[translate] gemini returned no text: model={} finish_reason={reason}",
                self.model
            ));
        }
        Ok(finish_text(&out.content))
    }

    /// 列出能拿來翻譯（支援 `generateContent`）的模型 ID，去掉 `models/` 前綴並排序；自動翻頁。
    pub async fn list_models(&self) -> Result<Vec<String>, TranslateError> {
        let url = format!("{}/{API_VERSION}/models", self.base_url);
        let mut ids = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let request = match &page_token {
                Some(token) => self.http.get(with_query(&url, "pageToken", token)),
                None => self.http.get(&url),
            };
            let resp = self.send_with(request, model_list_error).await?;
            let text = resp.text().await.map_err(connection_error)?;
            let page = parse_object(&text, "models response")?;
            for model in page["models"].as_array().into_iter().flatten() {
                let actions = model["supportedGenerationMethods"].as_array();
                let generates = actions.is_some_and(|a| a.iter().any(|m| m == "generateContent"));
                if let (true, Some(name)) = (generates, model["name"].as_str()) {
                    ids.push(name.strip_prefix("models/").unwrap_or(name).to_owned());
                }
            }
            match page["nextPageToken"].as_str() {
                Some(token) if !token.is_empty() => page_token = Some(token.to_owned()),
                _ => break,
            }
        }
        ids.sort();
        Ok(ids)
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, TranslateError> {
        self.send_with(request, status_error).await
    }

    /// 帶金鑰送出一次（SDK 預設不重試）；200 以外的狀態碼一律依 `map` 分類成錯誤。
    async fn send_with(
        &self,
        request: reqwest::RequestBuilder,
        map: fn(u16, &str) -> Option<TranslateError>,
    ) -> Result<reqwest::Response, TranslateError> {
        let Some(key) = self.api_key.clone() else {
            log("[translate] gemini api key has characters invalid in an HTTP header");
            return Err(TranslateError::invalid_key());
        };
        let resp = request
            .header("x-goog-api-key", key)
            .send()
            .await
            .map_err(connection_error)?;
        let status = resp.status().as_u16();
        if status == 200 {
            return Ok(resp);
        }
        Err(response_failure(resp, map).await)
    }
}

/// SDK 的模型路徑規則：空值或含 `..`／`?`／`&` 直接拒絕，沒有前綴的補上 `models/`。
fn model_path(model: &str) -> Result<String, TranslateError> {
    if model.is_empty() {
        return Err(TranslateError::Unexpected("model is required.".into()));
    }
    if model.contains("..") || model.contains('?') || model.contains('&') {
        return Err(TranslateError::Unexpected(
            "invalid model parameter.".into(),
        ));
    }
    if model.starts_with("models/") || model.starts_with("tunedModels/") {
        return Ok(model.to_owned());
    }
    Ok(format!("models/{model}"))
}
