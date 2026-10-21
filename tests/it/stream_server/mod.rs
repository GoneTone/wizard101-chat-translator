//! client 測試共用：假伺服器（串流停住不收尾、正常收尾、分段慢送、只送保持連線、不回應、中途斷線、接受連線就斷線）與請求／回應的小輔助。

// 每個測試檔只用到其中幾個
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use wiremock::MockServer;
use wizard101_chat_translator::translation::Turn;
use wizard101_chat_translator::translation::clients::ChatRequest;

async fn bind() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    (listener, url)
}

/// 靜態 system 與給定對話的確定性請求。
pub fn request<'a>(turns: &'a [Turn]) -> ChatRequest<'a> {
    ChatRequest {
        system: "sys",
        turns,
        max_tokens: None,
        deterministic: true,
    }
}

/// `server` 收到、打在 `request_path` 的所有請求本文（JSON）。
pub async fn bodies_at(server: &MockServer, request_path: &str) -> Vec<serde_json::Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == request_path)
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

/// 剛關掉的本機埠網址：連線必被拒絕。
pub async fn closed_url() -> String {
    let (listener, url) = bind().await;
    drop(listener);
    url
}

/// 讀完請求標頭後回 200 與 `first_event` 這段 SSE（chunked，尚未收尾）。
async fn start_stream(listener: TcpListener, first_event: &str) -> TcpStream {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut buf = vec![0u8; 8192];
    let mut seen = Vec::new();
    while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = socket.read(&mut buf).await.unwrap();
        assert!(n > 0, "client closed before sending the request headers");
        seen.extend_from_slice(&buf[..n]);
    }
    let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                transfer-encoding: chunked\r\n\r\n";
    socket.write_all(head.as_bytes()).await.unwrap();
    socket.write_all(&chunk(first_event)).await.unwrap();
    socket
}

/// `text` 包成一個 HTTP chunk。
fn chunk(text: &str) -> Vec<u8> {
    format!("{:x}\r\n{text}\r\n", text.len()).into_bytes()
}

/// 送出 chunked 結尾 `0\r\n\r\n` 後等客戶端先關：伺服器在請求本文沒讀完時關線會送 RST，客戶端會讀成斷線。
async fn finish_stream(mut socket: TcpStream) {
    socket.write_all(b"0\r\n\r\n").await.unwrap();
    let mut buf = vec![0u8; 8192];
    let _ = tokio::time::timeout(crate::common::WAIT, async {
        while let Ok(n) = socket.read(&mut buf).await
            && n > 0
        {}
    })
    .await;
}

/// 串流停住不收尾，送出第一段後觸發 `cancel`；handle 的結果為「[`crate::common::WAIT`] 內看到客戶端斷線」。
///
/// 等請求確實送達才取消，取消時機不受機器負載影響。
pub async fn stalled_stream(
    first_event: String,
    cancel: CancellationToken,
) -> (String, JoinHandle<bool>) {
    let (listener, url) = bind().await;
    let server = tokio::spawn(async move {
        let mut socket = start_stream(listener, &first_event).await;
        cancel.cancel();
        let mut buf = vec![0u8; 8192];
        // 客戶端斷線後 read 會讀到 EOF 或連線重置
        tokio::time::timeout(crate::common::WAIT, async {
            loop {
                match socket.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        })
        .await
        .is_ok()
    });
    (url, server)
}

/// 送出 `body` 後以 chunked 結尾 `0\r\n\r\n` 正常收尾：傳輸層完整，內容可能缺結束訊號。
pub async fn ended_stream(body: String) -> String {
    let (listener, url) = bind().await;
    tokio::spawn(async move {
        finish_stream(start_stream(listener, &body).await).await;
    });
    url
}

/// 依序送出 `parts`、每段之間隔 `interval`，最後正常收尾。
pub async fn paced_stream(parts: Vec<String>, interval: Duration) -> String {
    let (listener, url) = bind().await;
    tokio::spawn(async move {
        let mut socket = start_stream(listener, &parts[0]).await;
        for part in &parts[1..] {
            tokio::time::sleep(interval).await;
            if socket.write_all(&chunk(part)).await.is_err() {
                return;
            }
        }
        finish_stream(socket).await;
    });
    url
}

/// 送出 `first_event` 後每隔 `interval` 送一次 `keep_alive`、從不收尾，直到客戶端斷線。
pub async fn keep_alive_stream(
    first_event: String,
    keep_alive: String,
    interval: Duration,
) -> String {
    let (listener, url) = bind().await;
    tokio::spawn(async move {
        let mut socket = start_stream(listener, &first_event).await;
        let keep_alive = chunk(&keep_alive);
        while socket.write_all(&keep_alive).await.is_ok() {
            tokio::time::sleep(interval).await;
        }
    });
    url
}

/// 送出 `first_event` 後直接關掉連線，chunked 串流沒有收尾。
pub async fn dropped_stream(first_event: String) -> String {
    let (listener, url) = bind().await;
    tokio::spawn(async move {
        drop(start_stream(listener, &first_event).await);
    });
    url
}

/// 接受連線、讀請求，但從不回應，直到客戶端斷線。
pub async fn silent_server() -> String {
    let (listener, url) = bind().await;
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 8192];
        while let Ok(n) = socket.read(&mut buf).await
            && n > 0
        {}
    });
    url
}

/// 每條連線讀一次就關掉、不回任何回應；回傳網址與累計接受的連線數。
pub async fn dropping_server() -> (String, Arc<AtomicUsize>) {
    let (listener, url) = bind().await;
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            counter.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
        }
    });
    (url, accepted)
}
