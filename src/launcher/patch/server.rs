//! 向更新伺服器要最新檔案清單的位置（service 8 的清單請求）。

use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use super::PatchFailure;
use crate::launcher::wire::{self, MAX_RESPONSE_FRAMES, MESSAGE_PREFIX, u16_at, u32_at};
use crate::logs::log;

const TIMEOUT: Duration = Duration::from_secs(30);
/// 連線階段的上限，比系統預設的 TCP 逾時（約 21 秒）更早回報失敗。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PATCH_SERVICE: u8 = 8;
const MSG_LATEST_FILE_LIST: u8 = 2;

/// 伺服器回報的最新清單：版本、大小、CRC 與下載位置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListLocation {
    pub version: u32,
    pub size: u32,
    pub crc: u32,
    pub url: String,
    pub prefix: String,
    pub suffix: String,
}

fn attribute(tag: &str, name: &str) -> Regex {
    Regex::new(&format!(r#"<{tag}\b[^>]*\b{name}\s*=\s*"([^"]*)""#)).expect("valid regex")
}

/// 從遊戲目錄的 `PatchConfig.xml` 讀更新伺服器的主機與埠。
pub fn patch_server(root: &Path) -> Result<(String, u16), PatchFailure> {
    let path = root.join(r"PatchClient\BankA\PatchConfig.xml");
    let xml = std::fs::read_to_string(&path).map_err(|e| {
        log(format!("[patch] cannot read {}: {e}", path.display()));
        PatchFailure::Io(format!("{}: {e}", path.display()))
    })?;
    let attr = |re: &Regex| re.captures(&xml).map(|c| c[1].to_string());
    static HOST: LazyLock<Regex> = LazyLock::new(|| attribute("PatchServerHostname", "host"));
    static PORT: LazyLock<Regex> = LazyLock::new(|| attribute("PatchServerPort", "port"));
    let host = attr(&HOST);
    let port = attr(&PORT).and_then(|p| p.parse::<u16>().ok());
    match (host, port) {
        (Some(host), Some(port)) if !host.is_empty() => Ok((host, port)),
        _ => Err(PatchFailure::Protocol(
            "PatchConfig.xml has no usable patch server host and port".into(),
        )),
    }
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    out.extend((s.len() as u16).to_le_bytes());
    out.extend(s.as_bytes());
}

/// 組清單請求本體（未加框）；數值欄位皆 0、字串留空，Locale 為 English。
pub fn build_list_request() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend(0u32.to_le_bytes());
    push_str(&mut data, "");
    data.extend([0u8; 16]);
    for _ in 0..3 {
        push_str(&mut data, "");
    }
    push_str(&mut data, "English");
    let mut body = MESSAGE_PREFIX.to_vec();
    body.extend([PATCH_SERVICE, MSG_LATEST_FILE_LIST]);
    body.extend((data.len() as u16 + 4).to_le_bytes());
    body.extend(data);
    body
}

/// 依序讀回應欄位的游標。
struct Fields<'a> {
    body: &'a [u8],
    at: usize,
}

impl Fields<'_> {
    fn u32(&mut self) -> Option<u32> {
        let v = u32_at(self.body, self.at)?;
        self.at += 4;
        Some(v)
    }

    fn string(&mut self) -> Option<String> {
        let len = u16_at(self.body, self.at)? as usize;
        let bytes = self.body.get(self.at + 2..self.at + 2 + len)?;
        self.at += 2 + len;
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// 欄位順序同請求：Version、檔名、Type、Time、Size、CRC、URL、Prefix、Suffix。
fn read_location(body: &[u8]) -> Option<ListLocation> {
    let mut f = Fields { body, at: 8 };
    let version = f.u32()?;
    f.string()?;
    f.u32()?;
    f.u32()?;
    let size = f.u32()?;
    let crc = f.u32()?;
    Some(ListLocation {
        version,
        size,
        crc,
        url: f.string()?,
        prefix: f.string()?,
        suffix: f.string()?,
    })
}

/// 解析清單回應；不是 service 8 清單訊息的框回 `None`（略過繼續等）。
pub fn parse_list_response(body: &[u8]) -> Option<Result<ListLocation, PatchFailure>> {
    if body.len() < 6 || body[4] != PATCH_SERVICE || body[5] != MSG_LATEST_FILE_LIST {
        return None;
    }
    Some(read_location(body).ok_or_else(|| {
        PatchFailure::Protocol(format!(
            "truncated file list response ({} bytes)",
            body.len()
        ))
    }))
}

async fn request(host: &str, port: u16) -> Result<ListLocation, PatchFailure> {
    let connect_err = |e: std::io::Error| PatchFailure::Connect(e.to_string());
    let connecting = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)));
    let mut stream = match connecting.await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            log(format!("[patch] connect {host}:{port} failed: {e}"));
            return Err(connect_err(e));
        }
        Err(_) => {
            log(format!(
                "[patch] connect {host}:{port} timed out after {}s",
                CONNECT_TIMEOUT.as_secs()
            ));
            return Err(PatchFailure::Connect(format!(
                "connect timed out after {}s",
                CONNECT_TIMEOUT.as_secs()
            )));
        }
    };
    wire::handshake(&mut stream).await?;
    stream
        .write_all(&wire::frame(&build_list_request()))
        .await
        .map_err(connect_err)?;
    for _ in 0..MAX_RESPONSE_FRAMES {
        let body = wire::read_frame(&mut stream).await?;
        if let Some(result) = parse_list_response(&body) {
            return result;
        }
        log(format!(
            "[patch] skipped frame: {} bytes, service={:?}, message={:?}",
            body.len(),
            body.get(4),
            body.get(5)
        ));
    }
    Err(PatchFailure::Protocol("no file list response".into()))
}

/// 連上更新伺服器，取得最新清單的版本、大小、CRC 與下載位置。
pub async fn list_location(host: &str, port: u16) -> Result<ListLocation, PatchFailure> {
    let result = tokio::time::timeout(TIMEOUT, request(host, port))
        .await
        .unwrap_or_else(|_| {
            log(format!("[patch] timed out after {}s", TIMEOUT.as_secs()));
            Err(PatchFailure::Connect("timed out".into()))
        });
    match &result {
        Ok(l) => log(format!(
            "[patch] list location: host={host}:{port} version={} size={} crc={}",
            l.version, l.size, l.crc
        )),
        Err(e) => log(format!("[patch] list location failed: {e:?}")),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::wire::{frame, offer_body, read_frame};
    use tokio::net::TcpListener;

    fn response_body(version: u32, size: u32, crc: u32) -> Vec<u8> {
        let mut data = version.to_le_bytes().to_vec();
        push_str(&mut data, "LatestFileList.bin");
        for n in [0u32, 0, size, crc] {
            data.extend(n.to_le_bytes());
        }
        for s in ["http://host/list.bin", "http://host/pre", ".suf", "English"] {
            push_str(&mut data, s);
        }
        let mut body = vec![0, 0, 0, 0, 8, 2];
        body.extend((data.len() as u16 + 4).to_le_bytes());
        body.extend(data);
        body
    }

    fn expected() -> ListLocation {
        ListLocation {
            version: 7,
            size: 1234,
            crc: 0xDEADBEEF,
            url: "http://host/list.bin".into(),
            prefix: "http://host/pre".into(),
            suffix: ".suf".into(),
        }
    }

    #[test]
    fn list_request_matches_known_bytes() {
        let want = concat!(
            "00000000",
            "0802",
            "2900",
            "00000000",
            "0000",
            "00000000",
            "00000000",
            "00000000",
            "00000000",
            "0000",
            "0000",
            "0000",
            "0700",
            "456e676c697368"
        );
        assert_eq!(crate::util::hex(&build_list_request()), want);
    }

    #[test]
    fn parses_list_response() {
        let got = parse_list_response(&response_body(7, 1234, 0xDEADBEEF));
        assert_eq!(got, Some(Ok(expected())));
    }

    #[test]
    fn truncated_list_response_is_a_protocol_error() {
        let body = response_body(7, 1, 2);
        assert!(matches!(
            parse_list_response(&body[..30]),
            Some(Err(PatchFailure::Protocol(_)))
        ));
    }

    #[test]
    fn ignores_other_services() {
        assert!(parse_list_response(&[0, 0, 0, 0, 7, 2, 0, 0]).is_none());
        assert!(parse_list_response(&[0, 0, 0, 0, 8, 3, 0, 0]).is_none());
        assert!(parse_list_response(&[0, 0]).is_none());
    }

    #[test]
    fn patch_server_reads_host_and_port() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(patch_server(dir.path()), Err(PatchFailure::Io(_))));
        let bank = dir.path().join(r"PatchClient\BankA");
        std::fs::create_dir_all(&bank).unwrap();
        let file = bank.join("PatchConfig.xml");
        std::fs::write(
            &file,
            "<A><PatchServerHostname host=\"patch.example\"/>\n<PatchServerPort port=\"12500\"/></A>",
        )
        .unwrap();
        assert_eq!(
            patch_server(dir.path()),
            Ok(("patch.example".to_string(), 12500))
        );
        std::fs::write(&file, "<A><PatchServerPort port=\"12500\"/></A>").unwrap();
        assert!(matches!(
            patch_server(dir.path()),
            Err(PatchFailure::Protocol(_))
        ));
    }

    #[tokio::test]
    async fn list_location_over_fake_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            s.write_all(&frame(&offer_body())).await.unwrap();
            assert_eq!(read_frame(&mut s).await.unwrap().len(), 26);
            let req = read_frame(&mut s).await.unwrap();
            assert_eq!(req, build_list_request());
            s.write_all(&frame(&[0, 0, 0, 0, 7, 3, 0, 0]))
                .await
                .unwrap();
            s.write_all(&frame(&response_body(7, 1234, 0xDEADBEEF)))
                .await
                .unwrap();
        });
        let got = list_location("127.0.0.1", port).await.unwrap();
        server.await.unwrap();
        assert_eq!(got, expected());
    }
}
