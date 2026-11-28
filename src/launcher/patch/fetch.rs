//! 下載單一遊戲檔到暫存，CRC 驗證後換上正式位置。

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use flate2::read::GzDecoder;
use futures_util::StreamExt;

use super::PatchFailure;
use super::cache::file_stamp;
use super::crc::ki_crc_reader;
use super::list::FileEntry;
use super::url::is_gzip;
use crate::logs::log;
use crate::net::error_chain;

/// 下載暫存檔路徑：`<dest>.w101tmp`。
pub fn temp_path(dest: &Path) -> PathBuf {
    with_suffix(dest, ".w101tmp")
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    s.into()
}

fn io_failure(e: io::Error) -> PatchFailure {
    PatchFailure::Io(e.to_string())
}

/// 串流下載 `url` 到 `temp`（覆寫舊檔），每塊累加 `done`。
pub async fn fetch_to(
    http: &reqwest::Client,
    url: &str,
    temp: &Path,
    done: &AtomicU64,
) -> Result<(), PatchFailure> {
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|e| PatchFailure::Download(error_chain(e)))?;
    let status = response.status();
    if !status.is_success() {
        log(format!("[patch] download HTTP {status} url={url}"));
        return Err(PatchFailure::Download(format!("HTTP {status}")));
    }
    let result = write_stream(response, temp, done).await;
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

async fn write_stream(
    response: reqwest::Response,
    temp: &Path,
    done: &AtomicU64,
) -> Result<(), PatchFailure> {
    let mut file = File::create(temp).map_err(io_failure)?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| PatchFailure::Download(error_chain(e)))?;
        // 不用 block_in_place：current-thread runtime 會 panic，且每塊都很小。
        file.write_all(&bytes).map_err(io_failure)?;
        done.fetch_add(bytes.len() as u64, Ordering::Relaxed);
    }
    file.sync_all().map_err(io_failure)
}

/// 驗證下載內容的 CRC 與大小後換上 `dest`，回新檔的 `file_stamp`；失敗時 `dest` 不動。
pub fn install_file(
    entry: &FileEntry,
    temp: &Path,
    dest: &Path,
) -> Result<(u64, u64), PatchFailure> {
    let decoded = with_suffix(dest, ".w101tmp2");
    let outcome = if is_gzip(entry) {
        gunzip(temp, &decoded, u64::from(entry.size))
            .map_err(|e| PatchFailure::Download(format!("gunzip failed: {e}")))
            .and_then(|()| verify_and_swap(entry, &decoded, dest))
    } else {
        verify_and_swap(entry, temp, dest)
    };
    let _ = fs::remove_file(&decoded);
    let _ = fs::remove_file(temp);
    outcome?;
    if entry.file_type == 3 {
        let _ = fs::remove_file(with_suffix(dest, ".utd"));
    }
    file_stamp(dest).ok_or_else(|| PatchFailure::Io(format!("cannot stat {}", dest.display())))
}

/// 解壓到 `to`，最多寫 `max + 1` 位元組（多出的一個位元組讓大小檢查擋下）。
fn gunzip(from: &Path, to: &Path, max: u64) -> io::Result<()> {
    let mut decoder = GzDecoder::new(File::open(from)?).take(max + 1);
    let mut out = File::create(to)?;
    io::copy(&mut decoder, &mut out)?;
    out.sync_all()
}

fn verify_and_swap(entry: &FileEntry, source: &Path, dest: &Path) -> Result<(), PatchFailure> {
    let size = fs::metadata(source).map_err(io_failure)?.len();
    let crc = ki_crc_reader(File::open(source).map_err(io_failure)?).map_err(io_failure)?;
    if crc != entry.crc || size != u64::from(entry.size) {
        log(format!(
            "[patch] CRC mismatch file={} expected_crc={:08x} actual_crc={crc:08x} expected_size={} actual_size={size}",
            entry.src, entry.crc, entry.size
        ));
        return Err(PatchFailure::Download(format!(
            "CRC mismatch for {}: expected {:08x}/{} bytes, got {crc:08x}/{size} bytes",
            entry.src, entry.crc, entry.size
        )));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(io_failure)?;
    }
    fs::rename(source, dest).map_err(io_failure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::patch::crc::ki_crc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn entry(file_type: u32, body: &[u8]) -> FileEntry {
        FileEntry {
            package: String::new(),
            src: "f.dat".into(),
            tar: String::new(),
            file_type,
            size: body.len() as u32,
            crc: ki_crc(body),
        }
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn install_file_replaces_after_crc_check() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("sub/f.dat");
        let temp = temp_path(&dest);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(&dest, b"old").unwrap();
        fs::write(&temp, b"new content").unwrap();
        let stamp = install_file(&entry(1, b"new content"), &temp, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new content");
        assert_eq!(stamp.0, 11);
        assert!(!temp.exists());
    }

    #[test]
    fn install_file_creates_missing_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("a/b/f.dat");
        let temp = dir.path().join("f.w101tmp");
        fs::write(&temp, b"abc").unwrap();
        install_file(&entry(1, b"abc"), &temp, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"abc");
    }

    #[test]
    fn install_file_rejects_bad_crc_and_keeps_original() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.dat");
        let temp = temp_path(&dest);
        fs::write(&dest, b"old").unwrap();
        fs::write(&temp, b"corrupted!!").unwrap();
        let err = install_file(&entry(1, b"new content"), &temp, &dest).unwrap_err();
        assert!(matches!(err, PatchFailure::Download(m) if m.contains("CRC mismatch")));
        assert_eq!(fs::read(&dest).unwrap(), b"old");
        assert!(!temp.exists());
        assert!(!with_suffix(&dest, ".w101tmp2").exists());
    }

    #[test]
    fn install_file_gunzips_type_4() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.dat");
        let temp = temp_path(&dest);
        fs::write(&temp, gzip(b"plain bytes")).unwrap();
        install_file(&entry(4, b"plain bytes"), &temp, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"plain bytes");
        assert!(!temp.exists());
        assert!(!with_suffix(&dest, ".w101tmp2").exists());
    }

    #[test]
    fn install_file_removes_utd_for_type_3() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("Foo-WorldData.wad");
        let utd = with_suffix(&dest, ".utd");
        fs::write(&utd, b"x").unwrap();
        fs::write(temp_path(&dest), b"wad").unwrap();
        install_file(&entry(3, b"wad"), &temp_path(&dest), &dest).unwrap();
        assert!(!utd.exists());

        let other = dir.path().join("g.dat");
        let other_utd = with_suffix(&other, ".utd");
        fs::write(&other_utd, b"x").unwrap();
        fs::write(temp_path(&other), b"abc").unwrap();
        install_file(&entry(1, b"abc"), &temp_path(&other), &other).unwrap();
        assert!(other_utd.exists());
    }

    #[test]
    fn install_file_rejects_corrupt_gzip_and_keeps_original() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.dat");
        let temp = temp_path(&dest);
        fs::write(&dest, b"old").unwrap();
        fs::write(&temp, b"definitely not gzip").unwrap();
        let err = install_file(&entry(4, b"plain bytes"), &temp, &dest).unwrap_err();
        assert!(matches!(err, PatchFailure::Download(m) if m.contains("gunzip")));
        assert_eq!(fs::read(&dest).unwrap(), b"old");
        assert!(!temp.exists());
        assert!(!with_suffix(&dest, ".w101tmp2").exists());
    }

    #[test]
    fn install_file_rejects_oversized_gzip_output() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.dat");
        let temp = temp_path(&dest);
        fs::write(&dest, b"old").unwrap();
        fs::write(&temp, gzip(&vec![0u8; 100_000])).unwrap();
        let err = install_file(&entry(4, b"tiny"), &temp, &dest).unwrap_err();
        assert!(matches!(err, PatchFailure::Download(m) if m.contains("CRC mismatch")));
        assert_eq!(fs::read(&dest).unwrap(), b"old");
        assert!(!with_suffix(&dest, ".w101tmp2").exists());
    }

    #[tokio::test]
    async fn fetch_to_removes_partial_temp_on_disconnect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf).await;
            s.write_all(
                b"HTTP/1.1 200 OK
Content-Length: 1000

short",
            )
            .await
            .unwrap();
        });
        let url = format!("http://127.0.0.1:{port}/f");
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join("f.w101tmp");
        let http = crate::net::download_client(&url);
        let err = fetch_to(&http, &url, &temp, &AtomicU64::new(0))
            .await
            .unwrap_err();
        assert!(matches!(err, PatchFailure::Download(_)));
        assert!(!temp.exists());
    }

    async fn serve_once(status: &'static str, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf).await;
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            s.write_all(head.as_bytes()).await.unwrap();
            s.write_all(&body).await.unwrap();
        });
        format!("http://127.0.0.1:{port}/f")
    }

    #[tokio::test]
    async fn fetch_to_streams_and_counts() {
        let body = vec![7u8; 200_000];
        let url = serve_once("200 OK", body.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join("f.w101tmp");
        fs::write(&temp, b"stale and longer than nothing").unwrap();
        let done = AtomicU64::new(0);
        let http = crate::net::download_client(&url);
        fetch_to(&http, &url, &temp, &done).await.unwrap();
        assert_eq!(fs::read(&temp).unwrap(), body);
        assert_eq!(done.load(Ordering::Relaxed), 200_000);
    }

    #[tokio::test]
    async fn fetch_to_reports_http_status() {
        let url = serve_once("404 Not Found", b"nope".to_vec()).await;
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join("f.w101tmp");
        let http = crate::net::download_client(&url);
        let err = fetch_to(&http, &url, &temp, &AtomicU64::new(0))
            .await
            .unwrap_err();
        assert_eq!(err, PatchFailure::Download("HTTP 404 Not Found".into()));
        assert!(!temp.exists());
    }
}
