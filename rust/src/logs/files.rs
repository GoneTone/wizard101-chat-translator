//! log 檔基礎設施：session 分段標頭、過期段落清理、每行 UTC 時戳。
//!
//! `app.log`（診斷輸出）與 `messages.log`（收訊原始內容）共用這裡的規則。

use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

const SESSION_HEADER_PREFIX: &str = "===== session started ";
const LOG_RETENTION_DAYS: i64 = 7;
const LOG_HARD_CAP: usize = 5 * 1024 * 1024;
const LOG_KEEP_TAIL: usize = 1024 * 1024;
const HEADER_TS_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

/// 行首時戳（UTC＋0、毫秒），例如 `2026-10-17T01:02:03.456Z`。
pub fn utc_stamp(now: DateTime<Utc>) -> String {
    now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// log 的啟動分段標頭（UTC＋0）。
pub fn session_header(now: DateTime<Utc>) -> String {
    format!(
        "{SESSION_HEADER_PREFIX}{} =====",
        now.format(HEADER_TS_FORMAT)
    )
}

/// 以 session 標頭切段，只保留近 7 天內開始的段落；無標頭的前言與標頭解析失敗的段落一併丟棄。
pub fn trim_log_sessions(text: &str, now: DateTime<Utc>) -> String {
    let cutoff = now - Duration::days(LOG_RETENTION_DAYS);
    let mut keep = String::new();
    let mut keeping = false;
    for line in text.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix(SESSION_HEADER_PREFIX) {
            let token = rest.split(' ').next().unwrap_or("");
            keeping = NaiveDateTime::parse_from_str(token, HEADER_TS_FORMAT)
                .is_ok_and(|ts| ts.and_utc() >= cutoff);
        }
        if keeping {
            keep.push_str(line);
        }
    }
    keep
}

/// 開檔前清理過期段落，並把行尾正規化成單一 CRLF（修好舊版疊出的多餘 CR）。
fn prepare_log(path: &Path, now: DateTime<Utc>) -> io::Result<()> {
    let mut raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let original = raw.clone();
    if raw.len() > LOG_HARD_CAP {
        raw.drain(..raw.len() - LOG_KEEP_TAIL);
    }
    let lossy = String::from_utf8_lossy(&raw);
    let text = lossy
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .collect::<Vec<_>>()
        .join("\n");
    let out = trim_log_sessions(&text, now)
        .replace('\n', "\r\n")
        .into_bytes();
    if out != original {
        std::fs::write(path, out)?;
    }
    Ok(())
}

/// 清掉過期段落後以 append 開檔並寫入 session 標頭。
pub fn open_session_log(path: &Path, now: DateTime<Utc>) -> io::Result<File> {
    prepare_log(path, now)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(format!("{}\r\n", session_header(now)).as_bytes())?;
    Ok(file)
}

/// 每行前綴 UTC 時戳並以 CRLF 換行的 writer（`messages.log` 用）。
pub struct TimestampedWriter<W: Write> {
    inner: W,
    at_line_start: bool,
}

impl<W: Write> TimestampedWriter<W> {
    /// 包住一個 writer，從行首開始。
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            at_line_start: true,
        }
    }
}

impl<W: Write> Write for TimestampedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for part in buf.split_inclusive(|&b| b == b'\n') {
            if self.at_line_start {
                write!(self.inner, "{} ", utc_stamp(Utc::now()))?;
            }
            match part.strip_suffix(b"\n") {
                Some(body) => {
                    self.inner.write_all(body)?;
                    self.inner.write_all(b"\r\n")?;
                    self.at_line_start = true;
                }
                None => {
                    self.inner.write_all(part)?;
                    self.at_line_start = false;
                }
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 22, 12, 0, 0).unwrap()
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("w101-logs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("app.log")
    }

    fn day_header(days_ago: i64) -> String {
        session_header(now() - Duration::days(days_ago))
    }

    #[test]
    fn open_session_log_caps_oversized_file() {
        let path = temp("cap");
        std::fs::write(&path, vec![b'x'; 6 * 1024 * 1024]).unwrap();
        open_session_log(&path, now()).unwrap();
        let len = std::fs::metadata(&path).unwrap().len() as usize;
        assert!(
            len <= LOG_KEEP_TAIL + session_header(now()).len() + 2,
            "len={len}"
        );
    }

    #[test]
    fn lines_are_crlf() {
        let path = temp("crlf");
        let old = format!("{}\r\n[app] old\r\n", day_header(10));
        let recent = format!("{}\r\n[app] a\r\n", day_header(1));
        std::fs::write(&path, format!("{old}{recent}")).unwrap();
        let mut file = open_session_log(&path, now()).unwrap();
        let mut w = TimestampedWriter::new(&mut file);
        w.write_all(b"[app] x\n[app] y").unwrap();
        w.write_all(b"\n").unwrap();
        let text = String::from_utf8(std::fs::read(&path).unwrap()).unwrap();
        assert!(!text.contains("old") && !text.contains("\r\r"));
        assert_eq!(text.matches('\n').count(), 5);
        assert_eq!(text.matches("\r\n").count(), 5);
    }

    #[test]
    fn stacked_carriage_returns_are_repaired() {
        let path = temp("stack");
        std::fs::write(&path, format!("{}\r\r\r\n[app] a\r\r\r\n", day_header(1))).unwrap();
        prepare_log(&path, now()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("\r\r"));
        assert_eq!(text.matches("\r\n").count(), 2);
    }

    #[test]
    fn healthy_file_is_left_untouched() {
        let path = temp("healthy");
        let body = format!("{}\r\n[app] fine\r\n", day_header(1));
        std::fs::write(&path, &body).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        prepare_log(&path, now()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), body);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before
        );
    }

    #[test]
    fn writer_stamps_each_line_once_across_split_writes() {
        let mut out = Vec::new();
        let mut w = TimestampedWriter::new(&mut out);
        w.write_all(b"hello").unwrap();
        w.write_all(b"\n\nnext\n").unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.split("\r\n").collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].ends_with("Z hello"));
        assert!(lines[1].ends_with("Z "));
        assert!(lines[2].ends_with("Z next"));
    }
}
