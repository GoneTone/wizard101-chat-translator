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

/// 把 `data` 逐行補上時戳、換行換成 `eol` 後附加到 `out`；`at_line_start` 跨呼叫記錄是否停在行首。
pub(crate) fn push_stamped(out: &mut Vec<u8>, data: &[u8], at_line_start: &mut bool, eol: &[u8]) {
    for part in data.split_inclusive(|&b| b == b'\n') {
        if *at_line_start {
            out.extend_from_slice(utc_stamp(Utc::now()).as_bytes());
            out.push(b' ');
        }
        match part.strip_suffix(b"\n") {
            Some(body) => {
                out.extend_from_slice(body);
                out.extend_from_slice(eol);
                *at_line_start = true;
            }
            None => {
                out.extend_from_slice(part);
                *at_line_start = false;
            }
        }
    }
}

impl<W: Write> Write for TimestampedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut out = Vec::with_capacity(buf.len() + 32);
        push_stamped(&mut out, buf, &mut self.at_line_start, b"\r\n");
        self.inner.write_all(&out)?;
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

    fn temp() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        (dir, path)
    }

    fn day_header(days_ago: i64) -> String {
        session_header(now() - Duration::days(days_ago))
    }

    #[test]
    fn open_session_log_caps_oversized_file() {
        let (_dir, path) = temp();
        // 多個近期 session：砍到尾端後第一段殘缺被丟，其後完整段落必須留下
        let mut big = String::new();
        let mut i = 0;
        while big.len() < 6 * 1024 * 1024 {
            big.push_str(&format!("{}\r\n", day_header(1)));
            big.push_str(&format!("[app] filler {i} {}\r\n", "x".repeat(100_000)));
            i += 1;
        }
        let last = big.lines().last().unwrap().to_string();
        std::fs::write(&path, &big).unwrap();
        open_session_log(&path, now()).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(
            out.len() <= LOG_KEEP_TAIL + session_header(now()).len() + 2,
            "len={}",
            out.len()
        );
        assert!(out.contains(&last));
    }

    #[test]
    fn lines_are_crlf() {
        let (_dir, path) = temp();
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
        let (_dir, path) = temp();
        std::fs::write(&path, format!("{}\r\r\r\n[app] a\r\r\r\n", day_header(1))).unwrap();
        prepare_log(&path, now()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("\r\r"));
        assert_eq!(text.matches("\r\n").count(), 2);
    }

    #[test]
    fn healthy_file_is_left_untouched() {
        let (_dir, path) = temp();
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

    fn stamped_lines(out: Vec<u8>) -> Vec<String> {
        let text = String::from_utf8(out).unwrap();
        text.split_terminator("\r\n").map(str::to_owned).collect()
    }

    #[test]
    fn each_line_gets_a_stamp_prefix() {
        let mut out = Vec::new();
        TimestampedWriter::new(&mut out)
            .write_all(b"[app] one\n[reader] two\n")
            .unwrap();
        let lines = stamped_lines(out);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("Z [app] one") && lines[0].as_bytes()[24] == b' ');
        assert!(lines[1].ends_with("Z [reader] two") && lines[1].as_bytes()[24] == b' ');
    }

    #[test]
    fn next_line_after_split_write_gets_new_stamp() {
        let mut out = Vec::new();
        let mut w = TimestampedWriter::new(&mut out);
        w.write_all(b"[app] a\n").unwrap();
        w.write_all(b"[app] b\n").unwrap();
        let lines = stamped_lines(out);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("Z [app] a") && lines[1].ends_with("Z [app] b"));
    }

    #[test]
    fn empty_write_emits_nothing() {
        let mut out = Vec::new();
        TimestampedWriter::new(&mut out).write_all(b"").unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn a_lone_newline_at_line_start_stamps_a_blank_line() {
        let mut out = Vec::new();
        let mut w = TimestampedWriter::new(&mut out);
        w.write_all(b"\n").unwrap();
        w.write_all(b"[app] after\n").unwrap();
        let lines = stamped_lines(out);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("Z ") && lines[1].ends_with("Z [app] after"));
    }

    #[test]
    fn a_write_finishing_one_line_and_starting_another_stamps_only_the_new_one() {
        let mut out = Vec::new();
        let mut w = TimestampedWriter::new(&mut out);
        w.write_all(b"[app] he").unwrap();
        w.write_all(b"llo\n[app] next").unwrap();
        w.write_all(b"\n").unwrap();
        let lines = stamped_lines(out);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("Z [app] hello") && lines[1].ends_with("Z [app] next"));
        assert_eq!(lines[0].matches('Z').count(), 1);
    }

    #[test]
    fn non_ascii_text_passes_through_and_write_reports_the_input_length() {
        let mut out = Vec::new();
        let mut w = TimestampedWriter::new(&mut out);
        let text = "[app] 你好 ✓\n".as_bytes();
        assert_eq!(w.write(text).unwrap(), text.len());
        assert!(
            String::from_utf8(out)
                .unwrap()
                .ends_with("Z [app] 你好 ✓\r\n")
        );
    }

    #[test]
    fn a_failing_inner_writer_surfaces_the_error() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("disk full"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let error = TimestampedWriter::new(Broken)
            .write_all(b"[app] x\n")
            .unwrap_err();
        assert_eq!(error.to_string(), "disk full");
    }

    #[test]
    fn flush_reaches_the_wrapped_writer() {
        #[derive(Default)]
        struct Counting {
            bytes: Vec<u8>,
            flushes: usize,
        }
        impl Write for Counting {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.bytes.extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                self.flushes += 1;
                Ok(())
            }
        }
        let mut sink = Counting::default();
        let mut w = TimestampedWriter::new(&mut sink);
        w.write_all(b"[app] x\n").unwrap();
        w.flush().unwrap();
        assert_eq!(sink.flushes, 1);
        assert!(
            String::from_utf8(sink.bytes)
                .unwrap()
                .ends_with("Z [app] x\r\n")
        );
    }
}
