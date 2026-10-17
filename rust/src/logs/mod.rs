//! 診斷輸出：一行訊息加上 UTC 時戳後寫到目前的 sink（打包版為 exe 旁的 `app.log`）。
//!
//! 訊息一律英文、帶 `[模組]` 前綴，內容要有足夠 context；API 金鑰絕不可寫入。

mod files;
mod library;

pub use files::{
    TimestampedWriter, open_session_log, session_header, trim_log_sessions, utc_stamp,
};
pub use library::forward_library_logs;

use std::fs::File;
use std::io::{self, Write};
use std::sync::Mutex;

/// `log` 的輸出目的地。
pub enum Sink {
    /// 標準錯誤輸出（未呼叫 [`init`] 前的預設）。
    Stderr,
    /// 以 CRLF 換行寫入的 log 檔。
    File(File),
}

static SINK: Mutex<Sink> = Mutex::new(Sink::Stderr);

/// 切換 `log` 的輸出目的地。
pub fn init(sink: Sink) {
    *SINK.lock().unwrap_or_else(|e| e.into_inner()) = sink;
}

/// 寫一行帶 UTC 時戳的訊息；整行一次寫出，多執行緒下不會互相切斷。
pub fn log(message: impl AsRef<str>) {
    write_to(
        &mut SINK.lock().unwrap_or_else(|e| e.into_inner()),
        message.as_ref(),
    );
}

/// 多行訊息每行都補時戳；整段組好後一次寫出。
fn write_to(sink: &mut Sink, message: &str) {
    let (eol, mut out): (&[u8], Vec<u8>) = match sink {
        Sink::Stderr => (b"\n", Vec::new()),
        Sink::File(_) => (b"\r\n", Vec::new()),
    };
    let mut at_line_start = true;
    files::push_stamped(&mut out, message.as_bytes(), &mut at_line_start, eol);
    if !at_line_start {
        out.extend_from_slice(eol);
    }
    // 寫入失敗無處可報，丟棄以免 log 本身帶掉程式
    let _ = match sink {
        Sink::Stderr => io::stderr().write_all(&out),
        Sink::File(file) => file.write_all(&out),
    };
}

/// 掛上 panic hook：以 `[app] panic: ...` 與 backtrace 寫進 log。
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        log(format!("[app] panic: {info}\n{backtrace}"));
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 動到全域 SINK（與 panic hook）的測試必須串行。
    static GLOBAL_SINK_TESTS: Mutex<()> = Mutex::new(());

    fn temp_sink(name: &str) -> (std::path::PathBuf, Sink) {
        let path = std::env::temp_dir().join(format!("w101-logsink-{}-{name}", std::process::id()));
        let file = File::create(&path).unwrap();
        (path, Sink::File(file))
    }

    #[test]
    fn multiline_message_stamps_and_crlfs_every_line() {
        let (path, mut sink) = temp_sink("multi");
        write_to(&mut sink, "[app] panic: x\nframe 1\nframe 2");
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.split("\r\n").collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[3], "");
        assert!(lines[..3].iter().all(|l| l.as_bytes()[24] == b' '));
        assert!(lines[1].ends_with("Z frame 1"));
        assert_eq!(text.matches('\n').count(), 3);
    }

    #[test]
    fn panic_hook_logs_stamped_panic_line() {
        let _guard = GLOBAL_SINK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let (path, sink) = temp_sink("panic");
        init(sink);
        install_panic_hook();
        let _ = std::thread::spawn(|| panic!("boom")).join();
        let _ = std::panic::take_hook();
        init(Sink::Stderr);
        let text = std::fs::read_to_string(&path).unwrap();
        // 其他單元測試同時在寫全域 sink，只認自己的那一行
        let first = text
            .split("\r\n")
            .find(|l| l.contains("[app] panic: "))
            .unwrap_or_else(|| panic!("{text}"));
        assert_eq!(first.as_bytes()[24], b' ');
        assert!(first[25..].starts_with("[app] panic: "), "{first}");
        assert!(text.contains("boom"), "{text}");
    }

    #[test]
    fn log_from_many_threads_keeps_every_line_whole() {
        let _guard = GLOBAL_SINK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let (path, sink) = temp_sink("conc");
        init(sink);
        let threads: Vec<_> = "abcd"
            .chars()
            .map(|tag| {
                std::thread::spawn(move || {
                    for i in 0..300 {
                        log(format!("[logtest-{tag}] line {i}"));
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        init(Sink::Stderr);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.split_terminator("\r\n").collect();
        let ours = lines.iter().filter(|l| l.contains("[logtest-")).count();
        assert_eq!(ours, 1200);
        assert!(
            lines
                .iter()
                .all(|l| l.as_bytes()[24] == b' ' && l[25..].starts_with('['))
        );
    }
}
