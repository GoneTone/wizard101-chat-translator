//! 可續傳、sha256 驗證的檔案下載（GGUF 模型、llama.cpp zip）。

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

use crate::logs::log;
use crate::net::error_chain;
use crate::util::hex;

/// 下載完成後磁碟上至少要剩的空間。
const FREE_SPACE_MARGIN: u64 = 1 << 30;
const BODY_PREVIEW_CHARS: usize = 200;

/// 下載進度；UI 執行緒讀、下載任務寫。
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
}

/// 下載失敗原因；`Display` 為給使用者看的原文。
#[derive(Debug)]
pub enum DownloadError {
    Cancelled,
    Http(String),
    Io(String),
    Checksum { expected: String, actual: String },
    NoSpace { need: u64, free: u64 },
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(f, "cancelled"),
            Self::Http(m) | Self::Io(m) => write!(f, "{m}"),
            Self::Checksum { expected, actual } => {
                write!(f, "sha256 mismatch: expected {expected}, got {actual}")
            }
            Self::NoSpace { need, free } => {
                write!(
                    f,
                    "not enough disk space: need {need} bytes, free {free} bytes"
                )
            }
        }
    }
}

impl From<io::Error> for DownloadError {
    fn from(e: io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

/// `dir` 所在磁碟對目前使用者的可用位元組數。
pub fn free_space(dir: &Path) -> io::Result<u64> {
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    let mut free = 0u64;
    // SAFETY: wide 以 NUL 結尾，free 為有效的輸出指標；其餘參數允許 null
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(free)
}

/// 下載 `url` 到 `dest`：沿用 `dest.part` 續傳，（有 `sha256` 時）驗證通過才改名；取消或網路失敗保留 `.part`。
pub async fn download(
    http: &reqwest::Client,
    url: &str,
    dest: &Path,
    size: u64,
    sha256: Option<&str>,
    progress: &Progress,
    cancel: &CancellationToken,
) -> Result<(), DownloadError> {
    let result = run(http, url, dest, size, sha256, progress, cancel).await;
    if let Err(e) = &result {
        log(format!("[local] download failed: {e}"));
    }
    result
}

async fn run(
    http: &reqwest::Client,
    url: &str,
    dest: &Path,
    size: u64,
    sha256: Option<&str>,
    progress: &Progress,
    cancel: &CancellationToken,
) -> Result<(), DownloadError> {
    let started = Instant::now();
    let part = part_path(dest);
    let dir = dest.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;

    let mut existing = std::fs::metadata(&part).map_or(0, |m| m.len());
    if existing > size {
        std::fs::remove_file(&part)?;
        existing = 0;
    }
    let need = (size - existing).saturating_add(FREE_SPACE_MARGIN);
    let free = free_space(dir)?;
    if free < need {
        return Err(DownloadError::NoSpace { need, free });
    }

    progress.total.store(size, Ordering::Relaxed);
    progress.done.store(existing, Ordering::Relaxed);
    log(format!(
        "[local] download start: url={} size={size} resume_from={existing}",
        strip_query(url)
    ));

    // 先建出 .part，取消時才一定留得住
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&part)?;
    file.seek(SeekFrom::End(0))?;
    if existing < size {
        fetch(http, url, size, existing, &mut file, progress, cancel).await?;
    }
    drop(file);

    if let Some(expected) = sha256 {
        let actual = hash_file(part.clone()).await?;
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = std::fs::remove_file(&part);
            return Err(DownloadError::Checksum {
                expected: expected.to_ascii_lowercase(),
                actual,
            });
        }
    }
    std::fs::rename(&part, dest)?;

    let secs = started.elapsed().as_secs_f64();
    log(format!(
        "[local] download done: bytes={size} secs={secs:.1} avg_mb_s={:.1}",
        size as f64 / 1e6 / secs.max(0.001)
    ));
    Ok(())
}

async fn fetch(
    http: &reqwest::Client,
    url: &str,
    size: u64,
    existing: u64,
    file: &mut File,
    progress: &Progress,
    cancel: &CancellationToken,
) -> Result<(), DownloadError> {
    let mut response = send(http, url, existing, cancel).await?;
    let mut start = existing;
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT
        && content_range_start(&response) != Some(existing)
    {
        log(format!(
            "[local] unexpected Content-Range {:?} for resume_from={existing}; restarting from zero",
            response.headers().get(reqwest::header::CONTENT_RANGE)
        ));
        response = send(http, url, 0, cancel).await?;
        start = 0;
    }
    let status = response.status();
    if status == reqwest::StatusCode::OK {
        tokio::task::block_in_place(|| {
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0)).map(|_| ())
        })?;
        start = 0;
        progress.done.store(0, Ordering::Relaxed);
    } else if status != reqwest::StatusCode::PARTIAL_CONTENT || start != existing {
        let body = response.text().await.unwrap_or_default();
        let body: String = body.chars().take(BODY_PREVIEW_CHARS).collect();
        return Err(DownloadError::Http(format!("HTTP {status}: {body}")));
    }

    let expected = size - start;
    if let Some(len) = response.content_length()
        && len != expected
    {
        return Err(DownloadError::Http(format!(
            "unexpected Content-Length: server announced {len} bytes, expected {expected}"
        )));
    }

    let mut received = start;
    let mut stream = response.bytes_stream();
    loop {
        let chunk = tokio::select! {
            c = stream.next() => c,
            () = cancel.cancelled() => return Err(DownloadError::Cancelled),
        };
        match chunk {
            Some(Ok(bytes)) => {
                received += bytes.len() as u64;
                if received > size {
                    return Err(DownloadError::Http(format!(
                        "server sent more than the expected {size} bytes"
                    )));
                }
                tokio::task::block_in_place(|| file.write_all(&bytes))?;
                progress
                    .done
                    .fetch_add(bytes.len() as u64, Ordering::Relaxed);
            }
            Some(Err(e)) => return Err(DownloadError::Http(error_chain(e))),
            None => break,
        }
    }
    if received != size {
        return Err(DownloadError::Http(format!(
            "connection ended early: received {received} of {size} bytes"
        )));
    }
    Ok(())
}

async fn send(
    http: &reqwest::Client,
    url: &str,
    from: u64,
    cancel: &CancellationToken,
) -> Result<reqwest::Response, DownloadError> {
    let mut request = http.get(url);
    if from > 0 {
        request = request.header("Range", format!("bytes={from}-"));
    }
    tokio::select! {
        r = request.send() => r.map_err(|e| DownloadError::Http(error_chain(e))),
        () = cancel.cancelled() => Err(DownloadError::Cancelled),
    }
}

/// `Content-Range: bytes START-END/TOTAL` 的 START。
fn content_range_start(response: &reqwest::Response) -> Option<u64> {
    let value = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    value
        .strip_prefix("bytes ")?
        .split('-')
        .next()?
        .parse()
        .ok()
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

/// 去掉查詢字串（CDN 簽章）後的網址，供 log 用。
fn strip_query(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(mut u) => {
            u.set_query(None);
            u.to_string()
        }
        Err(_) => "<invalid url>".into(),
    }
}

async fn hash_file(path: PathBuf) -> Result<String, DownloadError> {
    tokio::task::spawn_blocking(move || {
        let mut file = File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hex(&hasher.finalize()))
    })
    .await
    .map_err(|e| DownloadError::Io(e.to_string()))?
}
