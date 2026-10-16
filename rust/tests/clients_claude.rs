//! Claude client 的 HTTP 協定測試：請求內容照 anthropic SDK、重試策略、串流解析、取消（wiremock，不打真 API）。

mod stream_server;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::logs::{self, Sink};
use wizard101_chat_translator::translation::Turn;
use wizard101_chat_translator::translation::clients::{
    ChatRequest, ClaudeClient, MAX_TOKENS_REGION, MAX_TOKENS_THINKING,
};
use wizard101_chat_translator::translation::errors::TranslateError;

/// 整個測試檔共用一個 log 檔（全域 sink 只有一個，並行測試各自 init 會互搶）；以各自的模型名稱篩選。
fn log_file() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let path = std::env::temp_dir().join(format!("clients_claude_{}.log", std::process::id()));
        logs::init(Sink::File(std::fs::File::create(&path).unwrap()));
        path
    })
}

fn output_format() -> Value {
    json!({"type": "json_schema", "schema": {
        "type": "object", "properties": {"translation": {"type": "string"}},
        "required": ["translation"], "additionalProperties": false}})
}

fn events(list: &[Value]) -> String {
    list.iter()
        .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap()))
        .collect()
}

/// 一則完整的 Messages 串流：`blocks` 為 (type, text)，text 依 delta 分兩段送。
fn message_stream(blocks: &[(&str, &str)], stop_reason: &str) -> ResponseTemplate {
    let mut list = vec![
        json!({"type": "message_start", "message": {"id": "msg", "type": "message",
            "role": "assistant", "model": "claude-x", "content": [], "stop_reason": null,
            "usage": {"input_tokens": 250, "output_tokens": 1,
                "cache_read_input_tokens": 1200, "cache_creation_input_tokens": 0}}}),
        json!({"type": "ping"}),
    ];
    for (index, (kind, text)) in blocks.iter().enumerate() {
        let (head, tail) = text.split_at(text.len() / 2);
        let start = if *kind == "text" {
            json!({"type": "text", "text": ""})
        } else {
            json!({"type": kind, "thinking": ""})
        };
        list.push(json!({"type": "content_block_start", "index": index, "content_block": start}));
        for piece in [head, tail] {
            list.push(json!({"type": "content_block_delta", "index": index,
                "delta": {"type": "text_delta", "text": piece}}));
        }
        list.push(json!({"type": "content_block_stop", "index": index}));
    }
    list.push(json!({"type": "message_delta",
        "delta": {"stop_reason": stop_reason, "stop_sequence": null},
        "usage": {"output_tokens": 9}}));
    list.push(json!({"type": "message_stop"}));
    ResponseTemplate::new(200).set_body_raw(events(&list), "text/event-stream")
}

fn translation(text: &str) -> String {
    json!({"translation": text}).to_string()
}

fn ok_stream(text: &str) -> ResponseTemplate {
    message_stream(&[("text", &translation(text))], "end_turn")
}

fn error_body(status: u16, message: &str) -> ResponseTemplate {
    ResponseTemplate::new(status)
        .set_body_json(json!({"type": "error", "error": {"type": "x", "message": message}}))
}

fn client(server: &MockServer, effort: &str) -> ClaudeClient {
    ClaudeClient::new("sk-test", "claude-x", effort, &server.uri())
}

fn request<'a>(turns: &'a [Turn]) -> ChatRequest<'a> {
    ChatRequest {
        system: "sys",
        turns,
        max_tokens: None,
        deterministic: true,
    }
}

async fn chat(c: &ClaudeClient) -> Result<String, TranslateError> {
    c.chat(request(&[Turn::user("[A] hi")]), None).await
}

async fn mount_messages(server: &MockServer, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(response)
        .mount(server)
        .await;
}

async fn bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

#[tokio::test]
async fn claude_sends_structured_output_and_parses_translation() {
    let server = MockServer::start().await;
    mount_messages(&server, ok_stream(" <think>x</think>[A] 譯文 ")).await;
    let turns = [Turn::user("u1"), Turn::assistant("a1"), Turn::user("u2")];
    let result = client(&server, "auto").chat(request(&turns), None).await;
    assert_eq!(result.unwrap(), "[A] 譯文");

    let received = &server.received_requests().await.unwrap()[0];
    let header = |name: &str| received.headers.get(name).unwrap().to_str().unwrap();
    assert_eq!(header("x-api-key"), "sk-test");
    assert_eq!(header("anthropic-version"), "2023-06-01");
    assert_eq!(header("accept"), "application/json");
    assert_eq!(header("content-type"), "application/json");
    assert!(received.headers.get("authorization").is_none());
    // 與 anthropic SDK 1.11.0 實際送出的 body 逐欄相同（含鍵的順序）
    let expected = json!({
        "max_tokens": MAX_TOKENS_THINKING,
        "messages": [{"role": "user", "content": "u1"},
                     {"role": "assistant", "content": "a1"},
                     {"role": "user", "content": "u2"}],
        "model": "claude-x",
        "output_config": {"format": output_format()},
        "system": [{"type": "text", "text": "sys", "cache_control": {"type": "ephemeral"}}],
        "stream": true,
    });
    assert_eq!(
        std::str::from_utf8(&received.body).unwrap(),
        expected.to_string()
    );
}

#[tokio::test]
async fn claude_effort_only_when_not_auto() {
    let server = MockServer::start().await;
    mount_messages(&server, ok_stream("x")).await;
    chat(&client(&server, "auto")).await.unwrap();
    chat(&client(&server, "low")).await.unwrap();
    let sent = bodies(&server).await;
    assert_eq!(sent[0]["output_config"], json!({"format": output_format()}));
    assert_eq!(
        sent[1]["output_config"],
        json!({"format": output_format(), "effort": "low"})
    );
    // 思考深度壓低不代表關閉思考：Claude 沒有 thinking 開關可送
    assert!(sent.iter().all(|b| b.get("thinking").is_none()));
}

#[tokio::test]
async fn claude_no_temperature() {
    let server = MockServer::start().await;
    mount_messages(&server, ok_stream("x")).await;
    let c = client(&server, "auto");
    for deterministic in [true, false] {
        let turns = [Turn::user("hi")];
        let req = ChatRequest {
            deterministic,
            ..request(&turns)
        };
        c.chat(req, None).await.unwrap();
    }
    assert!(
        bodies(&server)
            .await
            .iter()
            .all(|b| b.get("temperature").is_none())
    );
}

#[tokio::test]
async fn claude_system_has_cache_control_at_end() {
    let server = MockServer::start().await;
    mount_messages(&server, ok_stream("x")).await;
    let turns = [
        Turn::user("[B] earlier"),
        Turn::assistant("x"),
        Turn::user("[A] hi"),
    ];
    client(&server, "auto")
        .chat(request(&turns), None)
        .await
        .unwrap();
    let body = &bodies(&server).await[0];
    assert_eq!(
        body["system"],
        json!([{"type": "text", "text": "sys", "cache_control": {"type": "ephemeral"}}])
    );
    // 會變的上下文不可放在斷點之前，也不再另設斷點
    assert!(!body["messages"].to_string().contains("cache_control"));
}

#[tokio::test]
async fn claude_region_limit_is_sent_and_used_for_truncation() {
    let server = MockServer::start().await;
    mount_messages(
        &server,
        message_stream(&[("text", "{\"transl")], "max_tokens"),
    )
    .await;
    let turns = [Turn::user("hi")];
    let req = ChatRequest {
        max_tokens: Some(MAX_TOKENS_REGION),
        ..request(&turns)
    };
    let result = client(&server, "auto").chat(req, None).await;
    assert!(
        matches!(&result, Err(TranslateError::BadOutput(m)) if m.contains("max_tokens=4096")),
        "{result:?}"
    );
    assert_eq!(bodies(&server).await[0]["max_tokens"], MAX_TOKENS_REGION);
}

#[tokio::test]
async fn claude_retries_429_twice_then_offline() {
    let log_path = log_file();
    let server = MockServer::start().await;
    mount_messages(
        &server,
        error_body(429, "slow down").insert_header("retry-after", "0.2"),
    )
    .await;
    let c = ClaudeClient::new("k", "claude-retry-marker", "auto", &server.uri());
    let started = Instant::now();
    let result = chat(&c).await;
    let elapsed = started.elapsed();
    assert_eq!(
        result,
        Err(TranslateError::Offline {
            status: Some(429),
            detail: "slow down".into()
        })
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
    // 兩次等待各照 retry-after 的 0.2 秒；指數退避至少要 0.375＋0.75 秒
    assert!(
        elapsed >= Duration::from_millis(400) && elapsed < Duration::from_millis(1100),
        "{elapsed:?}"
    );
    let text = std::fs::read_to_string(log_path).unwrap();
    for n in 1..=2 {
        let line = format!(
            "[translate] claude request failed (HTTP 429, model=claude-retry-marker); \
             retry {n}/2 in 0.20s\r\n"
        );
        assert!(text.contains(&line), "{text}");
    }
}

#[tokio::test]
async fn cancel_during_backoff_returns_cancelled_without_retrying() {
    let server = MockServer::start().await;
    mount_messages(
        &server,
        error_body(429, "slow down").insert_header("retry-after-ms", "2000"),
    )
    .await;
    let token = CancellationToken::new();
    let canceller = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        canceller.cancel();
    });
    let started = Instant::now();
    let result = client(&server, "auto")
        .chat(request(&[Turn::user("hi")]), Some(&token))
        .await;
    assert_eq!(result, Err(TranslateError::Cancelled));
    assert!(started.elapsed() < Duration::from_millis(1500));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn claude_retry_recovers_when_a_later_attempt_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(error_body(529, "overloaded").insert_header("retry-after-ms", "1"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    mount_messages(&server, ok_stream("[A] ok")).await;
    assert_eq!(chat(&client(&server, "auto")).await.unwrap(), "[A] ok");
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn claude_retry_conditions_follow_the_sdk() {
    let cases = [
        // (status, x-should-retry, expected attempts, expected error)
        (408, None, 3, "config"),
        (409, None, 3, "config"),
        (500, None, 3, "offline"),
        (529, None, 3, "offline"),
        (400, None, 1, "config"),
        (401, None, 1, "config"),
        (404, None, 1, "config"),
        (503, Some("false"), 1, "offline"),
        (400, Some("true"), 3, "config"),
    ];
    for (status, should_retry, attempts, kind) in cases {
        let server = MockServer::start().await;
        let mut response = error_body(status, "nope").insert_header("retry-after-ms", "1");
        if let Some(value) = should_retry {
            response = response.insert_header("x-should-retry", value);
        }
        mount_messages(&server, response).await;
        let error = chat(&client(&server, "auto")).await.unwrap_err();
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), attempts, "{status} {should_retry:?}");
        assert_eq!(error.status(), Some(status));
        match kind {
            "config" => assert!(matches!(error, TranslateError::Config { .. }), "{status}"),
            _ => assert!(matches!(error, TranslateError::Offline { .. }), "{status}"),
        }
    }
}

#[tokio::test]
async fn claude_retries_connection_failures_then_offline() {
    let (url, accepted) = stream_server::dropping_server().await;
    let c = ClaudeClient::new("sk-secret", "claude-x", "auto", &url);
    let error = chat(&c).await.unwrap_err();
    assert_eq!(error, TranslateError::offline("Connection error."));
    assert_eq!(accepted.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn claude_list_models_connection_error_maps_to_offline() {
    let (url, accepted) = stream_server::dropping_server().await;
    let c = ClaudeClient::new("sk-secret", "claude-x", "auto", &url);
    assert_eq!(
        c.list_models().await,
        Err(TranslateError::offline("Connection error."))
    );
    assert_eq!(accepted.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn claude_stream_dropped_mid_response_is_offline() {
    // 裁示 R10：串流讀到一半斷線視為離線、不重送（Python 因 httpx2 漏接而未分類）
    let start = json!({"type": "message_start", "message": {"content": [], "usage": {}}});
    let url = stream_server::dropped_stream(events(&[start])).await;
    let c = ClaudeClient::new("k", "claude-x", "auto", &url);
    let error = chat(&c).await.unwrap_err();
    assert!(
        matches!(&error, TranslateError::Offline { status: None, detail } if !detail.is_empty()),
        "{error:?}"
    );
}

#[tokio::test]
async fn claude_max_tokens_stop_is_bad_output() {
    let server = MockServer::start().await;
    mount_messages(
        &server,
        message_stream(
            &[("text", "{\"translation\": \"am chick um chick")],
            "max_tokens",
        ),
    )
    .await;
    let result = chat(&client(&server, "auto")).await;
    let Err(TranslateError::BadOutput(message)) = result else {
        panic!("{result:?}");
    };
    assert!(message.contains("max_tokens=4096"), "{message}");
    assert!(message.contains("completion_tokens=9"), "{message}");
    assert!(message.contains("am chick"), "{message}");
}

#[tokio::test]
async fn claude_reads_the_first_text_block() {
    let server = MockServer::start().await;
    let first = translation("[A] 第一版");
    let second = translation("[A] 第二版");
    mount_messages(
        &server,
        message_stream(
            &[("text", &first), ("thinking", ""), ("text", &second)],
            "end_turn",
        ),
    )
    .await;
    assert_eq!(chat(&client(&server, "auto")).await.unwrap(), "[A] 第一版");
}

#[tokio::test]
async fn claude_unexpected_output_is_bad_output() {
    let cases: [&[(&str, &str)]; 4] = [
        &[("text", "")],
        &[("text", "[A] 不是 JSON")],
        &[("text", "{\"other\": \"x\"}")],
        &[("thinking", "")],
    ];
    for blocks in cases {
        let server = MockServer::start().await;
        mount_messages(&server, message_stream(blocks, "end_turn")).await;
        assert_eq!(
            chat(&client(&server, "auto")).await,
            Err(TranslateError::BadOutput(
                "unexpected output (stop_reason=end_turn)".into()
            )),
            "{blocks:?}"
        );
    }
}

#[tokio::test]
async fn claude_status_error_carries_api_message() {
    let server = MockServer::start().await;
    mount_messages(&server, error_body(401, "invalid x-api-key")).await;
    assert_eq!(
        chat(&client(&server, "auto")).await,
        Err(TranslateError::Config {
            status: Some(401),
            detail: "invalid x-api-key".into()
        })
    );
}

#[tokio::test]
async fn claude_error_without_message_falls_back_to_sdk_wording() {
    let server = MockServer::start().await;
    mount_messages(&server, ResponseTemplate::new(403)).await;
    assert_eq!(
        chat(&client(&server, "auto")).await,
        Err(TranslateError::Config {
            status: Some(403),
            detail: "Error code: 403".into()
        })
    );
}

#[tokio::test]
async fn claude_stream_error_event_is_unexpected() {
    let server = MockServer::start().await;
    let body = events(&[
        json!({"type": "message_start", "message": {"content": [], "usage": {}}}),
        json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}),
    ]);
    mount_messages(
        &server,
        ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"),
    )
    .await;
    let result = chat(&client(&server, "auto")).await;
    assert!(
        matches!(&result, Err(TranslateError::Unexpected(m)) if m.contains("Overloaded")),
        "{result:?}"
    );
}

#[tokio::test]
async fn claude_empty_key_is_unexpected_and_never_sent() {
    let server = MockServer::start().await;
    mount_messages(&server, ok_stream("x")).await;
    let c = ClaudeClient::new("", "claude-x", "auto", &server.uri());
    assert!(matches!(chat(&c).await, Err(TranslateError::Unexpected(_))));
    assert!(matches!(
        c.list_models().await,
        Err(TranslateError::Unexpected(_))
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn claude_models_paginate() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(wiremock::matchers::query_param("after_id", "a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "c", "type": "model"}],
            "has_more": false, "first_id": "c", "last_id": "c"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "b", "type": "model"}, {"id": "a", "type": "model"}],
            "has_more": true, "first_id": "b", "last_id": "a"})))
        .mount(&server)
        .await;
    assert_eq!(
        client(&server, "auto").list_models().await.unwrap(),
        ["a", "b", "c"]
    );
    let received = server.received_requests().await.unwrap();
    let queries: Vec<_> = received.iter().map(|r| r.url.query()).collect();
    assert_eq!(queries, [None, Some("after_id=a")]);
    let header = |name: &str| received[0].headers.get(name).unwrap().to_str().unwrap();
    assert_eq!(header("x-api-key"), "sk-test");
    assert_eq!(header("anthropic-version"), "2023-06-01");
}

#[tokio::test]
async fn claude_models_status_mapping() {
    let cases = [
        (
            401,
            TranslateError::Config {
                status: Some(401),
                detail: "nope".into(),
            },
        ),
        (404, TranslateError::NoModelList),
        (
            400,
            TranslateError::Config {
                status: Some(400),
                detail: "nope".into(),
            },
        ),
        (
            429,
            TranslateError::Offline {
                status: Some(429),
                detail: "nope".into(),
            },
        ),
        (
            500,
            TranslateError::Offline {
                status: Some(500),
                detail: "nope".into(),
            },
        ),
    ];
    for (status, expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(error_body(status, "nope").insert_header("retry-after-ms", "1"))
            .mount(&server)
            .await;
        assert_eq!(client(&server, "auto").list_models().await, Err(expected));
        let attempts = if matches!(status, 429 | 500) { 3 } else { 1 };
        assert_eq!(server.received_requests().await.unwrap().len(), attempts);
    }
}

#[tokio::test]
async fn claude_logs_cache_read_write_uncached() {
    let log_path = log_file();
    let server = MockServer::start().await;
    mount_messages(&server, ok_stream("x")).await;
    let c = ClaudeClient::new("k", "claude-cache-marker", "auto", &server.uri());
    chat(&c).await.unwrap();
    let text = std::fs::read_to_string(log_path).unwrap();
    assert!(
        text.contains(
            "[translate] prompt cache: provider=claude model=claude-cache-marker \
             read=1200 write=0 uncached=250"
        ),
        "{text}"
    );
}

#[tokio::test]
async fn cancel_mid_stream_returns_cancelled() {
    let start = json!({"type": "message_start", "message": {"content": [], "usage": {}}});
    let (url, server) = stream_server::stalled_stream(events(&[start])).await;
    let c = ClaudeClient::new("k", "claude-x", "auto", &url);
    let token = CancellationToken::new();
    let canceller = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        canceller.cancel();
    });
    let result = c.chat(request(&[Turn::user("hi")]), Some(&token)).await;
    assert_eq!(result, Err(TranslateError::Cancelled));
    assert!(
        server.await.unwrap(),
        "server never saw the connection close"
    );
}
