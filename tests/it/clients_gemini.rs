//! Gemini client 的 HTTP 協定測試：請求內容照 google-genai SDK、不重試、串流解析、取消（wiremock，不打真 API）。

use crate::common;
use crate::stream_server::{self, request};

use std::time::Duration;

use common::log_capture::{capture_logs, log_text};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::translation::Turn;
use wizard101_chat_translator::translation::clients::{
    GeminiClient, MAX_TOKENS_THINKING, TIMEOUT, ThinkingCaps,
};
use wizard101_chat_translator::translation::errors::TranslateError;

const STREAM_PATH: &str = "/v1beta/models/gemini-x:streamGenerateContent";

fn chunk(parts: Value, finish_reason: Option<&str>, usage: Option<Value>) -> Value {
    let mut candidate = json!({"content": {"parts": parts, "role": "model"}});
    if let Some(reason) = finish_reason {
        candidate["finishReason"] = json!(reason);
    }
    let mut out = json!({"candidates": [candidate]});
    if let Some(usage) = usage {
        out["usageMetadata"] = usage;
    }
    out
}

fn text(piece: &str) -> Value {
    json!([{"text": piece}])
}

fn stream_of(chunks: &[Value]) -> ResponseTemplate {
    let body: String = chunks
        .iter()
        .map(|c| format!("data: {c}\r\n\r\n"))
        .collect();
    ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

fn ok_stream() -> ResponseTemplate {
    stream_of(&[
        chunk(text("雙子"), None, None),
        chunk(text("譯文"), Some("STOP"), None),
    ])
}

fn error_body(status: u16, message: &str) -> ResponseTemplate {
    ResponseTemplate::new(status)
        .set_body_json(json!({"error": {"code": status, "message": message, "status": "X"}}))
}

fn client(server: &MockServer, effort: &str) -> GeminiClient {
    thinking_client(server, true, effort)
}

fn thinking_client(server: &MockServer, thinking: bool, effort: &str) -> GeminiClient {
    GeminiClient::new("AIza-test", "gemini-x", thinking, effort, &server.uri())
}

async fn sent_generation_config(server: &MockServer) -> Value {
    let received = server.received_requests().await.unwrap();
    serde_json::from_slice::<Value>(&received[0].body).unwrap()["generationConfig"].clone()
}

async fn chat(c: &GeminiClient) -> Result<String, TranslateError> {
    c.chat(request(&[Turn::user("[A] hi")]), None).await
}

async fn mount_stream(server: &MockServer, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path(STREAM_PATH))
        .and(query_param("alt", "sse"))
        .respond_with(response)
        .mount(server)
        .await;
}

#[tokio::test]
async fn gemini_maps_roles_and_system_instruction() {
    let server = MockServer::start().await;
    mount_stream(&server, ok_stream()).await;
    let turns = [Turn::user("u1"), Turn::assistant("a1"), Turn::user("u2")];
    let result = client(&server, "auto").chat(request(&turns), None).await;
    assert_eq!(result.unwrap(), "雙子譯文");

    let received = &server.received_requests().await.unwrap()[0];
    assert_eq!(received.url.query(), Some("alt=sse"));
    let header = |name: &str| received.headers.get(name).unwrap().to_str().unwrap();
    assert_eq!(header("x-goog-api-key"), "AIza-test");
    assert_eq!(header("content-type"), "application/json");
    assert!(!received.url.as_str().contains("AIza"));
    // 與 google-genai SDK 2.31.0 實際送出的 body 相同（含鍵的順序）；不送 temperature
    let expected = json!({
        "contents": [{"parts": [{"text": "u1"}], "role": "user"},
                     {"parts": [{"text": "a1"}], "role": "model"},
                     {"parts": [{"text": "u2"}], "role": "user"}],
        "systemInstruction": {"parts": [{"text": "sys"}], "role": "user"},
        "generationConfig": {"maxOutputTokens": MAX_TOKENS_THINKING},
    });
    assert_eq!(
        std::str::from_utf8(&received.body).unwrap(),
        expected.to_string()
    );
}

#[tokio::test]
async fn gemini_empty_key_sent_as_empty_header() {
    // Python 把空金鑰換成 " " 繞過 SDK 的檢查，SDK 再 strip 成空字串送出
    let server = MockServer::start().await;
    mount_stream(&server, error_body(400, "API key not valid")).await;
    for key in ["", "   ", "  AIza-k \t"] {
        let c = GeminiClient::new(key, "gemini-x", true, "auto", &server.uri());
        let error = chat(&c).await.unwrap_err();
        assert_eq!(
            error,
            TranslateError::Config {
                status: Some(400),
                detail: "API key not valid".into()
            }
        );
    }
    let sent: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| {
            r.headers
                .get("x-goog-api-key")
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(sent, ["", "", "AIza-k"]);
}

#[tokio::test]
async fn gemini_thinking_off_sends_zero_budget() {
    let server = MockServer::start().await;
    mount_stream(&server, ok_stream()).await;
    chat(&thinking_client(&server, false, "low")).await.unwrap();
    let config = sent_generation_config(&server).await;
    assert_eq!(config["thinkingConfig"], json!({"thinkingBudget": 0}));
}

#[tokio::test]
async fn gemini_thinking_on_auto_sends_no_config() {
    let server = MockServer::start().await;
    mount_stream(&server, ok_stream()).await;
    chat(&thinking_client(&server, true, "auto")).await.unwrap();
    let config = sent_generation_config(&server).await;
    assert!(config.get("thinkingConfig").is_none());
}

#[tokio::test]
async fn gemini_thinking_on_level_sends_upper_level() {
    let server = MockServer::start().await;
    mount_stream(&server, ok_stream()).await;
    chat(&thinking_client(&server, true, "medium"))
        .await
        .unwrap();
    let config = sent_generation_config(&server).await;
    assert_eq!(
        config["thinkingConfig"],
        json!({"thinking_level": "MEDIUM"})
    );
}

#[tokio::test]
async fn gemini_thinking_level_only_when_not_auto() {
    let server = MockServer::start().await;
    mount_stream(&server, ok_stream()).await;
    chat(&client(&server, "auto")).await.unwrap();
    chat(&client(&server, "low")).await.unwrap();
    let received = server.received_requests().await.unwrap();
    let config = |i: usize| {
        serde_json::from_slice::<Value>(&received[i].body).unwrap()["generationConfig"].clone()
    };
    assert_eq!(config(0), json!({"maxOutputTokens": MAX_TOKENS_THINKING}));
    assert_eq!(
        config(1),
        json!({"maxOutputTokens": MAX_TOKENS_THINKING, "thinkingConfig": {"thinking_level": "LOW"}})
    );
}

#[tokio::test]
async fn gemini_skips_thought_parts() {
    let server = MockServer::start().await;
    mount_stream(
        &server,
        stream_of(&[
            chunk(json!([{"text": "推理中", "thought": true}]), None, None),
            chunk(
                json!([{"text": "[A] 譯"}, {"text": "思考", "thought": true}]),
                None,
                None,
            ),
            chunk(json!([{"text": "文 "}]), Some("STOP"), None),
        ]),
    )
    .await;
    assert_eq!(chat(&client(&server, "auto")).await.unwrap(), "[A] 譯文");
}

#[tokio::test]
async fn gemini_models_filtered_and_prefix_stripped() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models"))
        .and(query_param("pageToken", "TOK"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"name": "models/gemini-a-pro", "supportedGenerationMethods": ["generateContent"]},
            {"name": "models/no-actions"}]})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"name": "models/gemini-x-flash",
             "supportedGenerationMethods": ["generateContent", "countTokens"]},
            {"name": "models/text-embedding-x", "supportedGenerationMethods": ["embedContent"]}],
            "nextPageToken": "TOK"})))
        .mount(&server)
        .await;
    assert_eq!(
        common::model_ids(client(&server, "auto").list_models().await.unwrap()),
        ["gemini-a-pro", "gemini-x-flash"]
    );
    let received = server.received_requests().await.unwrap();
    let queries: Vec<_> = received.iter().map(|r| r.url.query()).collect();
    assert_eq!(queries, [None, Some("pageToken=TOK")]);
    assert_eq!(
        received[0].headers.get("x-goog-api-key").unwrap(),
        "AIza-test"
    );
}

#[tokio::test]
async fn gemini_models_error_mapping() {
    let cases = [
        (
            403,
            TranslateError::Config {
                status: Some(403),
                detail: "denied".into(),
            },
        ),
        (404, TranslateError::NoModelList),
        (
            429,
            TranslateError::Offline {
                status: Some(429),
                detail: "denied".into(),
                retry_after: None,
            },
        ),
    ];
    for (status, expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1beta/models"))
            .respond_with(error_body(status, "denied"))
            .mount(&server)
            .await;
        assert_eq!(client(&server, "auto").list_models().await, Err(expected));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn gemini_retry_delay_in_the_error_body_is_carried() {
    let server = MockServer::start().await;
    let body = json!({"error": {"code": 429, "message": "quota", "status": "RESOURCE_EXHAUSTED",
        "details": [{"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "37s"}]}});
    mount_stream(&server, ResponseTemplate::new(429).set_body_json(body)).await;
    assert_eq!(
        chat(&client(&server, "auto")).await,
        Err(TranslateError::Offline {
            status: Some(429),
            detail: "quota".into(),
            retry_after: Some(Duration::from_secs(37)),
        })
    );
}

#[tokio::test]
async fn gemini_list_models_connection_error_maps_to_offline() {
    let (url, accepted) = stream_server::dropping_server().await;
    let c = GeminiClient::new("AIza-secret", "gemini-x", true, "auto", &url);
    let error = c.list_models().await.unwrap_err();
    assert!(
        matches!(&error, TranslateError::Offline { status: None, .. }),
        "{error:?}"
    );
    assert!(!error.to_string().contains("AIza-secret"));
    assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn gemini_retry_behavior_matches_sdk() {
    // genai 未設 retry_options 時 stop_after_attempt(1)：429／5xx 都只送一次
    for (status, message) in [(429, "quota"), (503, "overloaded"), (500, "internal")] {
        let server = MockServer::start().await;
        mount_stream(
            &server,
            error_body(status, message).insert_header("retry-after", "0"),
        )
        .await;
        assert_eq!(
            chat(&client(&server, "auto")).await,
            Err(TranslateError::Offline {
                status: Some(status),
                detail: message.into(),
                retry_after: None,
            })
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn gemini_html_error_page_is_stripped_to_text() {
    let server = MockServer::start().await;
    let page = ResponseTemplate::new(502).set_body_string(
        "<html><body><h1>502 Bad Gateway</h1><center>nginx</center></body></html>",
    );
    mount_stream(&server, page).await;
    assert_eq!(
        chat(&client(&server, "auto")).await,
        Err(TranslateError::Offline {
            status: Some(502),
            detail: "502 Bad Gateway nginx".into(),
            retry_after: None,
        })
    );
}

#[tokio::test]
async fn gemini_unwraps_a_single_element_error_array() {
    let server = MockServer::start().await;
    let body = json!([{"error": {"code": 400, "message": "API key not valid"}}]);
    mount_stream(&server, ResponseTemplate::new(400).set_body_json(body)).await;
    assert_eq!(
        chat(&client(&server, "auto")).await,
        Err(TranslateError::Config {
            status: Some(400),
            detail: "API key not valid".into()
        })
    );
}

#[tokio::test]
async fn gemini_error_mapping() {
    let server = MockServer::start().await;
    mount_stream(&server, error_body(400, "API key not valid")).await;
    assert_eq!(
        chat(&client(&server, "auto")).await,
        Err(TranslateError::Config {
            status: Some(400),
            detail: "API key not valid".into()
        })
    );

    let url = stream_server::closed_url().await;
    let c = GeminiClient::new("AIza-secret", "gemini-x", true, "auto", &url);
    let error = chat(&c).await.unwrap_err();
    assert!(
        matches!(&error, TranslateError::Offline { status: None, .. }),
        "{error:?}"
    );
    assert!(!error.to_string().contains("AIza-secret"));
}

#[tokio::test]
async fn gemini_error_chunk_mid_stream_maps_by_its_code() {
    let server = MockServer::start().await;
    mount_stream(
        &server,
        stream_of(&[
            chunk(text("半"), None, None),
            json!({"error": {"code": 503, "message": "overloaded", "status": "UNAVAILABLE"}}),
        ]),
    )
    .await;
    assert_eq!(
        chat(&client(&server, "auto")).await,
        Err(TranslateError::Offline {
            status: Some(503),
            detail: "overloaded".into(),
            retry_after: None,
        })
    );
}

#[tokio::test]
async fn gemini_max_tokens_finish_is_bad_output() {
    let server = MockServer::start().await;
    let usage = json!({"promptTokenCount": 30, "candidatesTokenCount": 4096});
    mount_stream(
        &server,
        stream_of(&[chunk(text("am chick"), Some("MAX_TOKENS"), Some(usage))]),
    )
    .await;
    let result = chat(&client(&server, "auto")).await;
    let Err(TranslateError::BadOutput(message)) = result else {
        panic!("{result:?}");
    };
    assert!(message.contains("max_tokens=4096"), "{message}");
    assert!(message.contains("completion_tokens=4096"), "{message}");
    assert!(message.contains("am chick"), "{message}");
}

#[tokio::test]
async fn gemini_key_with_invalid_header_characters_is_a_key_error() {
    let server = MockServer::start().await;
    mount_stream(&server, ok_stream()).await;
    let c = GeminiClient::new("AIza-bad\nkey", "gemini-x", true, "auto", &server.uri());
    assert_eq!(chat(&c).await, Err(TranslateError::invalid_key()));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn gemini_invalid_model_is_unexpected_and_never_sent() {
    let server = MockServer::start().await;
    mount_stream(&server, ok_stream()).await;
    for model in ["", "a?b", "a&b", "../x"] {
        let c = GeminiClient::new("k", model, true, "auto", &server.uri());
        assert!(
            matches!(chat(&c).await, Err(TranslateError::Unexpected(_))),
            "{model:?}"
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn gemini_model_prefix_is_kept() {
    let server = MockServer::start().await;
    mount_stream(&server, ok_stream()).await;
    let c = GeminiClient::new("k", "models/gemini-x", true, "auto", &server.uri());
    assert_eq!(chat(&c).await.unwrap(), "雙子譯文");
    assert_eq!(
        server.received_requests().await.unwrap()[0].url.path(),
        STREAM_PATH
    );
}

#[tokio::test]
async fn gemini_logs_prompt_cache_usage() {
    capture_logs();
    let server = MockServer::start().await;
    let usage = json!({"promptTokenCount": 3000, "cachedContentTokenCount": 2048,
        "candidatesTokenCount": 9, "thoughtsTokenCount": 37});
    Mock::given(method("POST"))
        .and(path(
            "/v1beta/models/gemini-cache-marker:streamGenerateContent",
        ))
        .respond_with(stream_of(&[chunk(text("譯文"), Some("STOP"), Some(usage))]))
        .mount(&server)
        .await;
    let c = GeminiClient::new("k", "gemini-cache-marker", true, "auto", &server.uri());
    chat(&c).await.unwrap();
    let text = log_text();
    assert!(
        text.contains(
            "[translate] prompt cache: provider=gemini model=gemini-cache-marker \
             prompt=3000 cached=2048 reasoning_tokens=37"
        ),
        "{text}"
    );
}

#[tokio::test]
async fn cancel_mid_stream_returns_cancelled() {
    let first = format!("data: {}\r\n\r\n", chunk(text("半"), None, None));
    let token = CancellationToken::new();
    let (url, server) = stream_server::stalled_stream(first, token.clone()).await;
    let c = GeminiClient::new("k", "gemini-x", true, "auto", &url);
    let result = c.chat(request(&[Turn::user("hi")]), Some(&token)).await;
    assert_eq!(result, Err(TranslateError::Cancelled));
    assert!(
        server.await.unwrap(),
        "server never saw the connection close"
    );
}

#[tokio::test]
async fn gemini_blocked_finish_reasons_are_bad_output_with_sample() {
    for reason in ["SAFETY", "RECITATION", "PROHIBITED_CONTENT", "OTHER"] {
        let server = MockServer::start().await;
        mount_stream(
            &server,
            stream_of(&[
                chunk(text("半句"), None, None),
                chunk(json!([]), Some(reason), None),
            ]),
        )
        .await;
        let result = chat(&client(&server, "auto")).await;
        let Err(TranslateError::BadOutput(message)) = result else {
            panic!("{reason}: {result:?}");
        };
        assert!(
            message.contains(&format!("finish_reason={reason}")),
            "{message}"
        );
        assert!(message.contains("半句"), "{message}");
    }
}

#[tokio::test]
async fn gemini_prompt_block_reason_is_bad_output() {
    let server = MockServer::start().await;
    let blocked = json!({"promptFeedback": {"blockReason": "PROHIBITED_CONTENT"}});
    mount_stream(&server, stream_of(&[blocked])).await;
    let result = chat(&client(&server, "auto")).await;
    assert!(
        matches!(&result, Err(TranslateError::BadOutput(m)) if m.contains("block_reason=PROHIBITED_CONTENT")),
        "{result:?}"
    );
}

#[tokio::test]
async fn gemini_stream_ended_without_finish_reason_is_offline() {
    let body = format!("data: {}\r\n\r\n", chunk(text("半句"), None, None));
    let url = stream_server::ended_stream(body).await;
    let c = GeminiClient::new("k", "gemini-x", true, "auto", &url);
    let error = chat(&c).await.unwrap_err();
    assert!(
        matches!(&error, TranslateError::Offline { status: None, detail, .. } if detail.contains("before completion")),
        "{error:?}"
    );
}

#[tokio::test]
async fn gemini_keep_alive_comments_past_the_total_timeout_are_offline() {
    capture_logs();
    let comment = ": keep-alive\r\n\r\n".to_owned();
    let url =
        stream_server::keep_alive_stream(comment.clone(), comment, Duration::from_millis(50)).await;
    let c = GeminiClient::new("k", "gemini-total-timeout", true, "auto", &url)
        .with_timeouts(TIMEOUT, Duration::from_millis(300));
    let result = tokio::time::timeout(common::WAIT, chat(&c))
        .await
        .expect("request never gave up");
    assert!(
        matches!(&result, Err(TranslateError::Offline { status: None, .. })),
        "{result:?}"
    );
    let text = log_text();
    assert!(
        text.contains("[translate] request timed out: provider=gemini model=gemini-total-timeout"),
        "{text}"
    );
}

#[tokio::test]
async fn gemini_models_carry_thinking_flag() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1beta/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"name": "models/no-think", "thinking": false, "supportedGenerationMethods": ["generateContent"]},
            {"name": "models/thinks", "thinking": true, "supportedGenerationMethods": ["generateContent"]},
            {"name": "models/silent", "supportedGenerationMethods": ["generateContent"]}]})))
        .mount(&server)
        .await;
    let models = client(&server, "auto").list_models().await.unwrap();
    let caps = |id: &str| models.iter().find(|m| m.id == id).unwrap().thinking.clone();
    assert_eq!(
        caps("no-think"),
        ThinkingCaps {
            supported: Some(false),
            ..Default::default()
        }
    );
    assert_eq!(
        caps("thinks"),
        ThinkingCaps {
            supported: Some(true),
            ..Default::default()
        }
    );
    assert_eq!(caps("silent"), ThinkingCaps::default());
}
