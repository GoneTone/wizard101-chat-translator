//! GGUF 檔頭解析與 HTTP Range 讀取（wiremock，不打真 API）。

use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};
use wizard101_chat_translator::local::gguf::{ParseError, fetch, fetch_with, parse};
use wizard101_chat_translator::local::hf::hf_client;

enum Val {
    U32(u32),
    U64(u64),
    Arr(Vec<u32>),
    Str(String),
    Strs(usize),
}
use Val::*;

fn s(v: &str) -> Val {
    Str(v.into())
}

fn put_str(out: &mut Vec<u8>, v: &str) {
    out.extend((v.len() as u64).to_le_bytes());
    out.extend(v.as_bytes());
}

/// 手寫 GGUF v3 檔頭（無張量資料）。
fn gguf_bytes(kv: &[(&str, Val)], tensors: &[&str]) -> Vec<u8> {
    let mut out = b"GGUF".to_vec();
    out.extend(3u32.to_le_bytes());
    out.extend((tensors.len() as u64).to_le_bytes());
    out.extend((kv.len() as u64).to_le_bytes());
    for (k, v) in kv {
        put_str(&mut out, k);
        match v {
            U32(n) => {
                out.extend(4u32.to_le_bytes());
                out.extend(n.to_le_bytes());
            }
            U64(n) => {
                out.extend(10u32.to_le_bytes());
                out.extend(n.to_le_bytes());
            }
            Arr(v) => {
                out.extend(9u32.to_le_bytes());
                out.extend(4u32.to_le_bytes());
                out.extend((v.len() as u64).to_le_bytes());
                for n in v {
                    out.extend(n.to_le_bytes());
                }
            }
            Str(t) => {
                out.extend(8u32.to_le_bytes());
                put_str(&mut out, t);
            }
            Strs(n) => {
                out.extend(9u32.to_le_bytes());
                out.extend(8u32.to_le_bytes());
                out.extend((*n as u64).to_le_bytes());
                for i in 0..*n {
                    put_str(&mut out, &format!("tok{i:08}"));
                }
            }
        }
    }
    for t in tensors {
        put_str(&mut out, t);
        out.extend(1u32.to_le_bytes());
        out.extend(1u64.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(0u64.to_le_bytes());
    }
    out
}

#[test]
fn reads_attention_shape_and_mtp() {
    let bytes = gguf_bytes(
        &[
            ("general.architecture", s("qwen35")),
            ("qwen35.block_count", U32(4)),
            ("qwen35.attention.head_count", U32(16)),
            ("qwen35.attention.head_count_kv", U32(4)),
            ("qwen35.embedding_length", U32(2048)),
        ],
        &["blk.3.nextn.eh_proj.weight"],
    );
    let m = parse(&bytes).unwrap();
    assert_eq!(
        (
            m.arch.as_str(),
            m.block_count,
            m.head_count_kv,
            m.key_length,
            m.value_length,
            m.expert_count,
            m.has_mtp
        ),
        ("qwen35", 4, 4, 128, 128, 0, true)
    );
}

#[test]
fn kv_heads_fall_back_to_head_count_and_no_mtp() {
    let bytes = gguf_bytes(
        &[
            ("general.architecture", s("x")),
            ("x.block_count", U32(2)),
            ("x.attention.head_count", U32(8)),
            ("x.embedding_length", U32(512)),
        ],
        &["blk.0.attn_q.weight"],
    );
    let m = parse(&bytes).unwrap();
    assert_eq!((m.head_count_kv, m.key_length, m.has_mtp), (8, 64, false));
}

#[test]
fn missing_architecture_is_invalid() {
    let bytes = gguf_bytes(&[("x.block_count", U32(2))], &[]);
    assert!(matches!(parse(&bytes), Err(ParseError::Invalid(_))));
}

#[test]
fn truncated_header_asks_for_more() {
    let bytes = gguf_bytes(
        &[
            ("general.architecture", s("x")),
            ("tokenizer.ggml.tokens", Strs(10_000)),
        ],
        &[],
    );
    assert_eq!(parse(&bytes[..bytes.len() / 2]), Err(ParseError::NeedMore));
}

#[test]
fn non_gguf_bytes_are_invalid() {
    assert!(matches!(
        parse(b"PK\x03\x04...."),
        Err(ParseError::Invalid(_))
    ));
}

struct RangeFile(Vec<u8>);

impl Respond for RangeFile {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let range = req.headers.get("range").unwrap().to_str().unwrap();
        let end: usize = range.trim_start_matches("bytes=0-").parse().unwrap();
        let end = end.min(self.0.len() - 1);
        ResponseTemplate::new(206)
            .insert_header(
                "Content-Range",
                format!("bytes 0-{end}/{}", self.0.len()).as_str(),
            )
            .set_body_bytes(self.0[..=end].to_vec())
    }
}

#[tokio::test]
async fn fetch_grows_the_range_until_the_header_fits() {
    // 約 6 MiB 的檔頭（大詞表）加填充：4 MiB 不夠、8 MiB 才讀得完
    let mut file = gguf_bytes(
        &[
            ("general.architecture", s("x")),
            ("x.block_count", U32(2)),
            ("tokenizer.ggml.tokens", Strs(300_000)),
        ],
        &[],
    );
    assert!(file.len() > 4 << 20 && file.len() < 8 << 20);
    file.resize(10 << 20, 0);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(RangeFile(file))
        .mount(&server)
        .await;
    let meta = fetch(&hf_client(&server.uri()), &server.uri())
        .await
        .unwrap();
    assert_eq!((meta.arch.as_str(), meta.block_count), ("x", 2));

    let ranges: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.headers.get("range").unwrap().to_str().unwrap().into())
        .collect();
    assert_eq!(ranges, ["bytes=0-4194303", "bytes=0-8388607"]);
}

#[test]
fn per_layer_array_takes_the_maximum() {
    let bytes = gguf_bytes(
        &[
            ("general.architecture", s("x")),
            ("x.block_count", U32(3)),
            ("x.attention.head_count", Arr(vec![16, 16, 16])),
            ("x.attention.head_count_kv", Arr(vec![0, 4, 8])),
            ("x.embedding_length", U32(1024)),
        ],
        &[],
    );
    let m = parse(&bytes).unwrap();
    assert_eq!((m.head_count, m.head_count_kv, m.key_length), (16, 8, 64));
}

#[test]
fn u64_fits_and_out_of_range_counts_as_missing() {
    let ok = gguf_bytes(
        &[("general.architecture", s("x")), ("x.block_count", U64(7))],
        &[],
    );
    assert_eq!(parse(&ok).unwrap().block_count, 7);
    let big = gguf_bytes(
        &[
            ("general.architecture", s("x")),
            ("x.block_count", U64(u64::MAX)),
        ],
        &[],
    );
    assert_eq!(parse(&big).unwrap().block_count, 0);
}

async fn serve(responder: impl Respond + 'static) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(responder)
        .mount(&server)
        .await;
    server
}

fn small_header() -> Vec<u8> {
    gguf_bytes(
        &[
            ("general.architecture", s("x")),
            ("tokenizer.ggml.tokens", Strs(1000)),
        ],
        &[],
    )
}

#[tokio::test]
async fn http_error_status_is_reported() {
    let server = serve(ResponseTemplate::new(404)).await;
    let err = fetch(&hf_client(&server.uri()), &server.uri())
        .await
        .unwrap_err();
    assert!(err.contains("404"), "{err}");
}

#[tokio::test]
async fn server_ignoring_range_still_works() {
    let mut file = small_header();
    file.resize(10 << 20, 0);
    let server = serve(ResponseTemplate::new(200).set_body_bytes(file)).await;
    let meta = fetch(&hf_client(&server.uri()), &server.uri())
        .await
        .unwrap();
    assert_eq!(meta.arch, "x");
}

#[tokio::test]
async fn file_ending_inside_the_header_fails_without_retrying() {
    let file = small_header();
    let server = serve(RangeFile(file[..file.len() / 2].to_vec())).await;
    let err = fetch(&hf_client(&server.uri()), &server.uri())
        .await
        .unwrap_err();
    assert!(err.contains("ended"), "{err}");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn range_cap_stops_the_growth() {
    let mut file = small_header();
    let header_len = file.len();
    file.resize(header_len * 4, 0);
    let server = serve(RangeFile(file)).await;
    let http = hf_client(&server.uri());
    let err = fetch_with(&http, &server.uri(), 256, 1024)
        .await
        .unwrap_err();
    assert!(err.contains("larger than 1024"), "{err}");
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}
