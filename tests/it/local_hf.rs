//! Hugging Face repo 解析、GGUF 分類與檔案列表（wiremock，不打真 API）。

use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::local::hf::{
    RepoFile, classify, hf_client, list_files, parse_quant, parse_repo,
};

#[test]
fn parse_repo_accepts_urls_and_bare_ids() {
    for input in [
        "unsloth/Qwen3.5-4B-GGUF",
        " https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/tree/main ",
        "huggingface.co/unsloth/Qwen3.5-4B-GGUF/",
        "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF",
    ] {
        assert_eq!(
            parse_repo(input).as_deref(),
            Some("unsloth/Qwen3.5-4B-GGUF"),
            "{input}"
        );
    }
    for bad in ["", "Qwen", "https://example.com/a/b", "a/b c", "a/b/c"] {
        assert_eq!(parse_repo(bad), None, "{bad}");
    }
}

#[test]
fn parse_quant_covers_unsloth_and_google_names() {
    let q = |n| {
        let q = parse_quant(n);
        (q.label, q.bits)
    };
    assert_eq!(
        q("Qwen3.5-4B-UD-Q4_K_XL.gguf"),
        ("UD-Q4_K_XL".into(), Some(4))
    );
    assert_eq!(q("Qwen3.5-4B-IQ4_NL.gguf"), ("IQ4_NL".into(), Some(4)));
    assert_eq!(
        q("gemma-4-26B-A4B-it-MXFP4_MOE.gguf"),
        ("MXFP4_MOE".into(), Some(4))
    );
    assert_eq!(q("gemma-4-12b-it-qat-q4_0.gguf"), ("q4_0".into(), Some(4)));
    assert_eq!(q("Qwen3.5-4B-BF16.gguf"), ("BF16".into(), Some(16)));
    assert_eq!(
        q("big-UD-Q2_K_XL-00002-of-00003.gguf"),
        ("UD-Q2_K_XL".into(), Some(2))
    );
    assert_eq!(q("weird.gguf"), ("weird.gguf".into(), None));
}

#[test]
fn classify_drops_mmproj_and_imatrix_and_splits_mtp_and_shards() {
    let f = |p: &str, s| RepoFile {
        path: p.into(),
        size: s,
        sha256: None,
    };
    let cat = classify(&[
        f("mmproj-F16.gguf", 1),
        f("gemma-4-26B-it-mmproj.gguf", 1),
        f("imatrix_unsloth.gguf", 1),
        f("README.md", 1),
        f("MTP/mtp-X-Q4_K_M.gguf", 2),
        f("MTP/mtp-X-shared-Q8_0.gguf", 3),
        f("Q2/X-Q2_K-00001-of-00002.gguf", 10),
        f("Q2/X-Q2_K-00002-of-00002.gguf", 5),
        f("X-Q4_K_M.gguf", 20),
        f("UP-Q8_0.GGUF", 30),
        f("Part/Y-Q3_K_M-00001-of-00003.gguf", 1),
        f("Part/Y-Q3_K_M-00003-of-00003.gguf", 1),
    ]);
    assert_eq!(
        cat.choices
            .iter()
            .map(|c| (c.quant.label.as_str(), c.files.len()))
            .collect::<Vec<_>>(),
        [("Q2_K", 2), ("Q4_K_M", 1), ("Q8_0", 1)]
    );
    assert_eq!(
        cat.mtp
            .iter()
            .map(|m| (m.quant.label.as_str(), m.shared))
            .collect::<Vec<_>>(),
        [("Q4_K_M", false), ("Q8_0", true)]
    );
}

#[tokio::test]
async fn list_files_reads_the_recursive_tree() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/models/o/r/tree/main"))
        .and(query_param("recursive", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"type": "directory", "path": "MTP"},
            {"type": "file", "path": "a.gguf", "size": 5, "lfs": {"oid": "ab", "size": 5}}
        ])))
        .mount(&server)
        .await;
    let files = list_files(&hf_client(&server.uri()), &server.uri(), "o/r")
        .await
        .unwrap();
    assert_eq!(
        files,
        [RepoFile {
            path: "a.gguf".into(),
            size: 5,
            sha256: Some("ab".into())
        }]
    );
}

#[tokio::test]
async fn list_files_reports_http_errors_verbatim() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Repository Not Found"))
        .mount(&server)
        .await;
    let err = list_files(&hf_client(&server.uri()), &server.uri(), "o/r")
        .await
        .unwrap_err();
    assert!(
        err.contains("HTTP 404") && err.contains("Repository Not Found"),
        "{err}"
    );
}

/// 很長的錯誤頁只帶開頭一段（錯誤訊息與 log 都不會塞進整頁 HTML）。
#[tokio::test]
async fn list_files_truncates_a_long_error_body() {
    let server = MockServer::start().await;
    let page = format!("<html>{}</html>", "x".repeat(5000));
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503).set_body_string(page))
        .mount(&server)
        .await;
    let err = list_files(&hf_client(&server.uri()), &server.uri(), "o/r")
        .await
        .unwrap_err();
    assert!(err.starts_with("HTTP 503: <html>xxx"), "{err}");
    assert!(err.len() < 300, "{} chars", err.len());
}

#[tokio::test]
async fn list_files_follows_the_next_page_link() {
    let server = MockServer::start().await;
    let next = format!("<{}/page2>; rel=\"next\"", server.uri());
    Mock::given(method("GET"))
        .and(path("/api/models/o/r/tree/main"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Link", next.as_str())
                .set_body_json(json!([{"type": "file", "path": "a.gguf", "size": 1}])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/page2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"type": "file", "path": "b.gguf", "size": 2}])),
        )
        .mount(&server)
        .await;
    let files = list_files(&hf_client(&server.uri()), &server.uri(), "o/r")
        .await
        .unwrap();
    assert_eq!(
        files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
        ["a.gguf", "b.gguf"]
    );
}

const CHILD_MARKER: &str = "W101_HF_PROXY_TEST_CHILD";
const CHILD_TEST: &str = "local_hf::child_lists_files_despite_an_unreachable_env_proxy";

#[tokio::test(flavor = "multi_thread")]
async fn loopback_base_ignores_the_env_proxy() {
    let exe = std::env::current_exe().unwrap();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(exe)
            .args([CHILD_TEST, "--exact", "--ignored", "--nocapture"])
            .env(CHILD_MARKER, "1")
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("NO_PROXY", "no-proxy.test")
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}
{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "run in a child process by loopback_base_ignores_the_env_proxy"]
async fn child_lists_files_despite_an_unreachable_env_proxy() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    let files = list_files(&hf_client(&server.uri()), &server.uri(), "o/r").await;
    assert_eq!(files, Ok(vec![]));
}
