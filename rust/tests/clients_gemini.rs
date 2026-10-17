//! Gemini client 的 HTTP 協定測試：請求內容照 google-genai SDK、不重試、串流解析、取消（wiremock，不打真 API）。

mod stream_server;

use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::logs::{self, Sink};
use wizard101_chat_translator::translation::Turn;
use wizard101_chat_translator::translation::clients::{
    ChatRequest, GeminiClient, MAX_TOKENS_THINKING,
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
    GeminiClient::new("AIza-test", "gemini-x", effort, &server.uri())
}

fn request<'a>(turns: &'a [Turn]) -> ChatRequest<'a> {
    ChatRequest {
        system: "sys",
        turns,
        max_tokens: None,
        deterministic: true,
    }
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
        let c = GeminiClient::new(key, "gemini-x", "auto", &server.uri());
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
        client(&server, "auto").list_models().await.unwrap(),
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
async fn gemini_list_models_connection_error_maps_to_offline() {
    let (url, accepted) = stream_server::dropping_server().await;
    let c = GeminiClient::new("AIza-secret", "gemini-x", "auto", &url);
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
                detail: message.into()
            })
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
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

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let c = GeminiClient::new("AIza-secret", "gemini-x", "auto", &url);
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
            detail: "overloaded".into()
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
async fn gemini_invalid_model_is_unexpected_and_never_sent() {
    let server = MockServer::start().await;
    mount_stream(&server, ok_stream()).await;
    for model in ["", "a?b", "a&b", "../x"] {
        let c = GeminiClient::new("k", model, "auto", &server.uri());
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
    let c = GeminiClient::new("k", "models/gemini-x", "auto", &server.uri());
    assert_eq!(chat(&c).await.unwrap(), "雙子譯文");
    assert_eq!(
        server.received_requests().await.unwrap()[0].url.path(),
        STREAM_PATH
    );
}

#[tokio::test]
async fn gemini_logs_prompt_cache_usage() {
    let log_path = std::env::temp_dir().join(format!("clients_gemini_{}.log", std::process::id()));
    logs::init(Sink::File(std::fs::File::create(&log_path).unwrap()));
    let server = MockServer::start().await;
    let usage = json!({"promptTokenCount": 3000, "cachedContentTokenCount": 2048,
        "candidatesTokenCount": 9});
    Mock::given(method("POST"))
        .and(path(
            "/v1beta/models/gemini-cache-marker:streamGenerateContent",
        ))
        .respond_with(stream_of(&[chunk(text("譯文"), Some("STOP"), Some(usage))]))
        .mount(&server)
        .await;
    let c = GeminiClient::new("k", "gemini-cache-marker", "auto", &server.uri());
    chat(&c).await.unwrap();
    let text = std::fs::read_to_string(&log_path).unwrap();
    let _ = std::fs::remove_file(&log_path);
    assert!(
        text.contains(
            "[translate] prompt cache: provider=gemini model=gemini-cache-marker \
             prompt=3000 cached=2048"
        ),
        "{text}"
    );
}

#[tokio::test]
async fn cancel_mid_stream_returns_cancelled() {
    let first = format!("data: {}\r\n\r\n", chunk(text("半"), None, None));
    let token = CancellationToken::new();
    let (url, server) = stream_server::stalled_stream(first, token.clone()).await;
    let c = GeminiClient::new("k", "gemini-x", "auto", &url);
    let result = c.chat(request(&[Turn::user("hi")]), Some(&token)).await;
    assert_eq!(result, Err(TranslateError::Cancelled));
    assert!(
        server.await.unwrap(),
        "server never saw the connection close"
    );
}
