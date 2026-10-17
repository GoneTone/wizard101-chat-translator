//! OpenAI 相容 client 的 HTTP 協定測試：串流、參數學習、錯誤對應、取消（wiremock，不打真 API）。

#[allow(dead_code)]
mod common;
mod stream_server;

use common::log_capture::{capture_logs, log_text};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::config::ServiceSpec;
use wizard101_chat_translator::translation::Turn;
use wizard101_chat_translator::translation::clients::{
    ChatRequest, Client, MAX_TOKENS, MAX_TOKENS_THINKING, OpenAiCompatClient, build_client,
};
use wizard101_chat_translator::translation::errors::TranslateError;

fn sse(chunks: &[Value]) -> String {
    let mut out: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    out.push_str("data: [DONE]\n\n");
    out
}

fn delta(text: &str) -> Value {
    json!({"choices": [{"delta": {"content": text}}]})
}

fn stream_of(chunks: &[Value]) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(sse(chunks), "text/event-stream")
}

fn ok_stream(text: &str) -> ResponseTemplate {
    stream_of(&[delta(text)])
}

fn error_body(status: u16, message: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({"error": {"message": message}}))
}

fn rejects(param: &str) -> ResponseTemplate {
    error_body(
        400,
        &format!("Unsupported parameter: '{param}' is not supported with this model."),
    )
}

fn client(server: &MockServer, thinking: bool, official: bool) -> OpenAiCompatClient {
    OpenAiCompatClient::new(&server.uri(), "", "m", thinking, official)
}

fn request<'a>(turns: &'a [Turn]) -> ChatRequest<'a> {
    ChatRequest {
        system: "sys",
        turns,
        max_tokens: None,
        deterministic: true,
    }
}

async fn chat(c: &OpenAiCompatClient) -> Result<String, TranslateError> {
    c.chat(request(&[Turn::user("[A] hi")]), None).await
}

async fn bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/v1/chat/completions")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

async fn mount_chat(server: &MockServer, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(response)
        .mount(server)
        .await;
}

async fn mount_chat_once(server: &MockServer, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(response)
        .up_to_n_times(1)
        .mount(server)
        .await;
}

async fn mount_models(server: &MockServer, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(response)
        .mount(server)
        .await;
}

#[tokio::test]
async fn streams_and_joins_deltas() {
    let server = MockServer::start().await;
    mount_chat(
        &server,
        stream_of(&[delta("譯"), delta("文 "), json!({"choices": []})]),
    )
    .await;
    assert_eq!(chat(&client(&server, true, false)).await.unwrap(), "譯文");
    let body = &bodies(&server).await[0];
    assert_eq!(body["stream"], true);
    assert_eq!(
        body["messages"][0],
        json!({"role": "system", "content": "sys"})
    );
    assert_eq!(
        body["messages"][1],
        json!({"role": "user", "content": "[A] hi"})
    );
}

#[tokio::test]
async fn think_blocks_are_stripped() {
    let server = MockServer::start().await;
    mount_chat(&server, ok_stream("<think>hmm</think>\n譯文")).await;
    assert_eq!(chat(&client(&server, true, false)).await.unwrap(), "譯文");
}

#[tokio::test]
async fn sends_bearer_only_with_key() {
    let server = MockServer::start().await;
    mount_chat(&server, ok_stream("x")).await;
    let keyed = OpenAiCompatClient::new(&server.uri(), "sk-secret", "m", true, false);
    chat(&keyed).await.unwrap();
    chat(&client(&server, true, false)).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[0].headers["authorization"], "Bearer sk-secret");
    assert!(!requests[1].headers.contains_key("authorization"));
}

#[tokio::test]
async fn official_uses_max_completion_tokens_and_no_temperature() {
    let server = MockServer::start().await;
    mount_chat(&server, ok_stream("x")).await;
    chat(&client(&server, false, true)).await.unwrap();
    let body = &bodies(&server).await[0];
    assert_eq!(body["max_completion_tokens"], MAX_TOKENS);
    assert!(body.get("max_tokens").is_none() && body.get("temperature").is_none());
    assert_eq!(body["stream_options"], json!({"include_usage": true}));
    assert_eq!(body["reasoning_effort"], "none");
    assert!(body.get("think").is_none());
}

#[tokio::test]
async fn custom_keeps_max_tokens_and_temperature_by_thinking_mode() {
    let server = MockServer::start().await;
    mount_chat(&server, ok_stream("x")).await;
    chat(&client(&server, false, false)).await.unwrap();
    chat(&client(&server, true, false)).await.unwrap();
    let sent = bodies(&server).await;
    assert_eq!(sent[0]["max_tokens"], MAX_TOKENS);
    assert_eq!(sent[0]["temperature"], 0);
    assert_eq!(sent[0]["think"], false);
    assert_eq!(sent[1]["max_tokens"], MAX_TOKENS_THINKING);
    assert!(sent[1].get("think").is_none() && sent[1].get("stream_options").is_none());
}

#[tokio::test]
async fn length_finish_is_bad_output_with_sample() {
    let server = MockServer::start().await;
    mount_chat(
        &server,
        stream_of(&[
            delta("呃 呃 呃"),
            json!({"choices": [{"delta": {}, "finish_reason": "length"}],
                   "usage": {"completion_tokens": 7}}),
        ]),
    )
    .await;
    let Err(TranslateError::BadOutput(message)) = chat(&client(&server, false, false)).await else {
        panic!("expected BadOutput");
    };
    assert!(message.contains("completion_tokens=7"), "{message}");
    assert!(message.contains("max_tokens=512"), "{message}");
    assert!(message.contains("呃 呃 呃"), "{message}");
}

#[tokio::test]
async fn missing_finish_reason_is_accepted() {
    let server = MockServer::start().await;
    mount_chat(&server, ok_stream("譯文")).await;
    assert_eq!(chat(&client(&server, true, false)).await.unwrap(), "譯文");
}

#[tokio::test]
async fn rejected_400_naming_param_drops_it_and_retries_once() {
    let server = MockServer::start().await;
    mount_chat_once(&server, rejects("reasoning_effort")).await;
    mount_chat(&server, ok_stream("譯文")).await;
    assert_eq!(chat(&client(&server, false, true)).await.unwrap(), "譯文");
    let sent = bodies(&server).await;
    assert_eq!(sent.len(), 2);
    assert!(sent[0].get("reasoning_effort").is_some());
    assert!(sent[1].get("reasoning_effort").is_none());
}

#[tokio::test]
async fn learned_params_persist_on_instance() {
    let server = MockServer::start().await;
    mount_chat_once(&server, rejects("reasoning_effort")).await;
    mount_chat(&server, ok_stream("譯文")).await;
    let c = client(&server, false, true);
    chat(&c).await.unwrap();
    chat(&c).await.unwrap();
    let sent = bodies(&server).await;
    assert_eq!(sent.len(), 3);
    assert!(sent[2].get("reasoning_effort").is_none());
}

#[tokio::test]
async fn other_rejection_wordings_are_recognised() {
    let server = MockServer::start().await;
    mount_chat_once(
        &server,
        error_body(
            400,
            "Unsupported value: 'temperature' does not support 0 with this model.",
        ),
    )
    .await;
    mount_chat(&server, ok_stream("譯文")).await;
    assert_eq!(chat(&client(&server, true, false)).await.unwrap(), "譯文");
    assert!(bodies(&server).await[1].get("temperature").is_none());
}

#[tokio::test]
async fn rejected_token_limit_is_swapped_not_dropped() {
    let server = MockServer::start().await;
    mount_chat_once(&server, rejects("max_completion_tokens")).await;
    mount_chat(&server, ok_stream("譯文")).await;
    chat(&client(&server, true, true)).await.unwrap();
    let sent = bodies(&server).await;
    assert!(sent[1].get("max_completion_tokens").is_none());
    assert_eq!(sent[1]["max_tokens"], MAX_TOKENS_THINKING);
}

#[tokio::test]
async fn token_limit_rejected_under_both_names_stops() {
    let server = MockServer::start().await;
    mount_chat_once(&server, rejects("max_completion_tokens")).await;
    mount_chat_once(&server, rejects("max_tokens")).await;
    mount_chat(&server, rejects("max_completion_tokens")).await;
    let result = chat(&client(&server, true, true)).await;
    assert!(matches!(
        result,
        Err(TranslateError::Config {
            status: Some(400),
            ..
        })
    ));
    assert_eq!(bodies(&server).await.len(), 2);
}

#[tokio::test]
async fn rejected_param_not_sent_is_not_retried() {
    let server = MockServer::start().await;
    mount_chat(&server, rejects("frequency_penalty")).await;
    let result = chat(&client(&server, true, false)).await;
    assert!(matches!(result, Err(TranslateError::Config { .. })));
    assert_eq!(bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn bad_request_without_named_param_is_config_error_once() {
    let server = MockServer::start().await;
    mount_chat(&server, error_body(400, "bad request")).await;
    let error = chat(&client(&server, true, false)).await.unwrap_err();
    assert_eq!(
        error,
        TranslateError::Config {
            status: Some(400),
            detail: "bad request".into()
        }
    );
    assert_eq!(bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn status_429_and_5xx_are_offline() {
    for status in [429, 500, 503] {
        let server = MockServer::start().await;
        mount_chat(&server, error_body(status, "busy")).await;
        let error = chat(&client(&server, true, false)).await.unwrap_err();
        assert_eq!(
            error,
            TranslateError::Offline {
                status: Some(status),
                detail: "busy".into()
            }
        );
    }
}

#[tokio::test]
async fn status_4xx_is_config_with_detail() {
    for status in [401, 403, 404] {
        let server = MockServer::start().await;
        let message = "Incorrect API key provided: sk-abc***";
        mount_chat(&server, error_body(status, message)).await;
        let error = chat(&client(&server, true, false)).await.unwrap_err();
        assert_eq!(
            error,
            TranslateError::Config {
                status: Some(status),
                detail: message.into()
            }
        );
    }
}

#[tokio::test]
async fn connection_failure_is_offline_without_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let c = OpenAiCompatClient::new(&url, "sk-secret", "m", true, false);
    let error = chat(&c).await.unwrap_err();
    assert!(
        matches!(&error, TranslateError::Offline { status: None, detail } if !detail.is_empty())
    );
    assert!(!error.to_string().contains("sk-secret"));
}

#[tokio::test]
async fn cancel_mid_stream_returns_cancelled_and_closes_connection() {
    let token = CancellationToken::new();
    let (url, server) =
        stream_server::stalled_stream(format!("data: {}\n\n", delta("partial")), token.clone())
            .await;
    let c = OpenAiCompatClient::new(&url, "", "m", true, false);
    let result = c.chat(request(&[Turn::user("hi")]), Some(&token)).await;
    assert_eq!(result, Err(TranslateError::Cancelled));
    assert!(
        server.await.unwrap(),
        "server never saw the connection close"
    );
}

#[tokio::test]
async fn request_cancelled_before_start_is_never_sent() {
    let server = MockServer::start().await;
    mount_chat(&server, ok_stream("x")).await;
    let token = CancellationToken::new();
    token.cancel();
    let result = client(&server, true, false)
        .chat(request(&[Turn::user("hi")]), Some(&token))
        .await;
    assert_eq!(result, Err(TranslateError::Cancelled));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn models_sorted() {
    let server = MockServer::start().await;
    let data =
        json!({"data": [{"id": "b"}, {"id": "a"}, {"id": ""}, {"name": "x"}, "junk", {"id": "c"}]});
    mount_models(&server, ResponseTemplate::new(200).set_body_json(data)).await;
    assert_eq!(
        client(&server, true, false).list_models().await.unwrap(),
        ["a", "b", "c"]
    );
}

#[tokio::test]
async fn models_404_is_no_model_list() {
    for status in [404, 405] {
        let server = MockServer::start().await;
        mount_models(&server, ResponseTemplate::new(status)).await;
        assert_eq!(
            client(&server, true, false).list_models().await,
            Err(TranslateError::NoModelList)
        );
    }
}

#[tokio::test]
async fn models_without_data_array_is_no_model_list() {
    let server = MockServer::start().await;
    mount_models(
        &server,
        ResponseTemplate::new(200).set_body_json(json!({"object": "list"})),
    )
    .await;
    assert_eq!(
        client(&server, true, false).list_models().await,
        Err(TranslateError::NoModelList)
    );
}

#[tokio::test]
async fn models_error_statuses_map_like_chat() {
    let server = MockServer::start().await;
    mount_models(&server, error_body(403, "region not supported")).await;
    assert_eq!(
        client(&server, true, false).list_models().await,
        Err(TranslateError::Config {
            status: Some(403),
            detail: "region not supported".into()
        })
    );
}

#[tokio::test]
async fn logs_prompt_cache_usage() {
    capture_logs();
    let server = MockServer::start().await;
    mount_chat(
        &server,
        stream_of(&[
            delta("x"),
            json!({"choices": [], "usage": {"prompt_tokens": 1400, "completion_tokens": 5,
                "prompt_tokens_details": {"cached_tokens": 1024}}}),
        ]),
    )
    .await;
    let c = OpenAiCompatClient::new(&server.uri(), "", "cache-marker-model", true, true);
    chat(&c).await.unwrap();
    let text = log_text();
    assert!(
        text.contains(
            "[translate] prompt cache: provider=openai-compatible model=cache-marker-model \
             prompt=1400 cached=1024"
        ),
        "{text}"
    );
}

#[test]
fn build_client_picks_endpoint_by_provider() {
    let spec = |provider: &str| ServiceSpec {
        provider: provider.into(),
        model: "m".into(),
        base_url: "http://localhost:1".into(),
        ..Default::default()
    };
    for provider in ["custom", "openai", "something-else"] {
        let client = build_client(&spec(provider));
        assert!(matches!(client, Client::OpenAiCompat(_)));
        assert_eq!(client.model(), "m");
    }
    let claude = build_client(&spec("claude"));
    assert!(matches!(claude, Client::Claude(_)) && claude.model() == "m");
    let gemini = build_client(&spec("gemini"));
    assert!(matches!(gemini, Client::Gemini(_)) && gemini.model() == "m");
}

#[tokio::test]
async fn malformed_stream_chunk_is_unexpected() {
    for bad in ["data: not json\n\n", "data: 5\n\n"] {
        let server = MockServer::start().await;
        let response = ResponseTemplate::new(200).set_body_raw(bad, "text/event-stream");
        mount_chat(&server, response).await;
        let result = chat(&client(&server, true, false)).await;
        assert!(
            matches!(result, Err(TranslateError::Unexpected(_))),
            "{bad:?}"
        );
    }
}

#[tokio::test]
async fn models_200_with_html_body_is_unexpected() {
    for body in ["<html>oops</html>", "[1, 2]"] {
        let server = MockServer::start().await;
        mount_models(&server, ResponseTemplate::new(200).set_body_string(body)).await;
        let result = client(&server, true, false).list_models().await;
        assert!(
            matches!(result, Err(TranslateError::Unexpected(_))),
            "{body}"
        );
    }
}

#[tokio::test]
async fn models_auth_errors_are_config() {
    for status in [401, 403] {
        let server = MockServer::start().await;
        mount_models(&server, error_body(status, "bad key")).await;
        assert_eq!(
            client(&server, true, false).list_models().await,
            Err(TranslateError::Config {
                status: Some(status),
                detail: "bad key".into()
            })
        );
    }
}

#[tokio::test]
async fn models_retryable_statuses_are_offline() {
    for status in [429, 500, 503] {
        let server = MockServer::start().await;
        mount_models(&server, error_body(status, "busy")).await;
        assert_eq!(
            client(&server, true, false).list_models().await,
            Err(TranslateError::Offline {
                status: Some(status),
                detail: "busy".into()
            })
        );
    }
}

#[tokio::test]
async fn models_connection_failure_is_offline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let c = OpenAiCompatClient::new(&url, "", "m", true, false);
    assert!(matches!(
        c.list_models().await,
        Err(TranslateError::Offline { status: None, .. })
    ));
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let server = MockServer::start().await;
    let redirect = ResponseTemplate::new(302).insert_header("location", "/elsewhere");
    mount_chat(&server, redirect).await;
    // httpx 不跟隨轉址：<400 照串流讀，沒有 data 行就是空譯文
    assert_eq!(chat(&client(&server, true, false)).await.unwrap(), "");
    assert_eq!(bodies(&server).await.len(), 1);
}
