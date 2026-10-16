//! 取消測試共用的假伺服器：回 200 與一段 SSE 後停住不收尾，回報客戶端是否斷線。

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// 啟動伺服器並回傳網址；handle 的結果為「5 秒內看到客戶端斷線」。
pub async fn stalled_stream(first_event: String) -> (String, JoinHandle<bool>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 8192];
        let mut seen = Vec::new();
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = socket.read(&mut buf).await.unwrap();
            seen.extend_from_slice(&buf[..n]);
        }
        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                    transfer-encoding: chunked\r\n\r\n";
        socket.write_all(head.as_bytes()).await.unwrap();
        let chunk = format!("{:x}\r\n{first_event}\r\n", first_event.len());
        socket.write_all(chunk.as_bytes()).await.unwrap();
        // 客戶端斷線後 read 會讀到 EOF 或連線重置
        tokio::time::timeout(Duration::from_secs(5), async {
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
