//! 診斷輸出：一行訊息加上 UTC 時戳後寫到目前的 sink（打包版為 exe 旁的 `app.log`）。
//!
//! 訊息一律英文、帶 `[模組]` 前綴，內容要有足夠 context；API 金鑰絕不可寫入。

mod files;
mod library;

pub use files::{
    TimestampedWriter, open_session_log, session_header, trim_log_sessions, utc_stamp,
};
pub use library::forward_library_logs;

use std::cell::Cell;
use std::fs::File;
use std::io::{self, Write};
use std::panic::UnwindSafe;
use std::sync::Mutex;

use crate::util::lock;

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
    *lock(&SINK) = sink;
}

/// 寫一行帶 UTC 時戳的訊息；整行一次寫出，多執行緒下不會互相切斷。
pub fn log(message: impl AsRef<str>) {
    write_to(&mut lock(&SINK), message.as_ref());
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

/// 低階鍵盤 hook 執行緒的名稱；`input::hook` 以它命名執行緒，panic hook 靠它認出自己跑在 hook 裡。
pub const KEYBOARD_HOOK_THREAD: &str = "keyboard-hook";

thread_local! {
    static EXPECTED_PANIC: Cell<bool> = const { Cell::new(false) };
}

/// 跑 `f` 並接住它的 panic，panic hook 不記 `[app] panic` 與 backtrace；給「panic 是預期中的失敗」的地方用（如解析不可信的檔頭）。
pub fn catch_expected_panic<T>(f: impl FnOnce() -> T + UnwindSafe) -> std::thread::Result<T> {
    let outer = EXPECTED_PANIC.replace(true);
    let result = std::panic::catch_unwind(f);
    EXPECTED_PANIC.set(outer);
    result
}

/// 掛上 panic hook：以 `[app] panic: ...` 與 backtrace 寫進 log。
///
/// 鍵盤 hook 執行緒上的 panic 不抓 backtrace：擷取要花時間，超過系統的 hook 逾時整個 hook 會被拿掉。
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        if EXPECTED_PANIC.get() {
            return;
        }
        if std::thread::current().name() == Some(KEYBOARD_HOOK_THREAD) {
            log(format!(
                "[app] panic: {info} (backtrace skipped inside the keyboard hook)"
            ));
            return;
        }
        let backtrace = std::backtrace::Backtrace::force_capture();
        log(format!("[app] panic: {info}\n{backtrace}"));
    }));
}

/// 動到全域 SINK（與 panic hook）的測試必須串行。
#[cfg(test)]
static GLOBAL_SINK_TESTS: Mutex<()> = Mutex::new(());

/// 跑 `run` 並回傳這段期間寫進 log 的全部文字；其他執行緒同時寫的行也會混進來，斷言請用 `contains`。
#[cfg(test)]
pub(crate) fn capture(run: impl FnOnce()) -> String {
    let _guard = lock(&GLOBAL_SINK_TESTS);
    let dir = tempfile::tempdir().expect("temp dir for the log capture");
    let path = dir.path().join("capture.log");
    init(Sink::File(
        File::create(&path).expect("create the capture log"),
    ));
    // 閉包 panic 時也要把 sink 換回，否則後面的測試會寫進已刪除的暫存檔。
    struct RestoreSink;
    impl Drop for RestoreSink {
        fn drop(&mut self) {
            init(Sink::Stderr);
        }
    }
    let restore = RestoreSink;
    run();
    drop(restore);
    std::fs::read_to_string(&path).expect("read the capture log")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_restores_the_stderr_sink_when_the_closure_panics() {
        let result = std::panic::catch_unwind(|| capture(|| panic!("boom")));
        assert!(result.is_err());
        let _serial = lock(&GLOBAL_SINK_TESTS);
        assert!(matches!(*lock(&SINK), Sink::Stderr));
    }

    fn temp_sink() -> (tempfile::TempDir, std::path::PathBuf, Sink) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        let file = File::create(&path).unwrap();
        (dir, path, Sink::File(file))
    }

    #[test]
    fn multiline_message_stamps_and_crlfs_every_line() {
        let (_dir, path, mut sink) = temp_sink();
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
        let _guard = lock(&GLOBAL_SINK_TESTS);
        let (_dir, path, sink) = temp_sink();
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
    fn an_expected_panic_is_caught_without_logging() {
        let _guard = lock(&GLOBAL_SINK_TESTS);
        let (_dir, path, sink) = temp_sink();
        init(sink);
        install_panic_hook();
        let caught = catch_expected_panic(|| panic!("quiet boom"));
        let _ = std::thread::spawn(|| panic!("loud boom")).join();
        let _ = std::panic::take_hook();
        init(Sink::Stderr);
        assert!(caught.is_err());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("quiet boom"), "{text}");
        assert!(text.contains("loud boom"), "{text}");
    }

    #[test]
    fn a_panic_inside_the_keyboard_hook_is_logged_without_a_backtrace() {
        let _guard = lock(&GLOBAL_SINK_TESTS);
        let (_dir, path, sink) = temp_sink();
        init(sink);
        install_panic_hook();
        let _ = std::thread::Builder::new()
            .name(KEYBOARD_HOOK_THREAD.into())
            .spawn(|| panic!("hook boom"))
            .unwrap()
            .join();
        let _ = std::panic::take_hook();
        init(Sink::Stderr);
        let text = std::fs::read_to_string(&path).unwrap();
        let line = text
            .split("\r\n")
            .find(|l| l.contains("hook boom"))
            .unwrap_or_else(|| panic!("{text}"));
        assert!(
            line.ends_with("(backtrace skipped inside the keyboard hook)"),
            "{line}"
        );
    }

    #[test]
    fn log_from_many_threads_keeps_every_line_whole() {
        let _guard = lock(&GLOBAL_SINK_TESTS);
        let (_dir, path, sink) = temp_sink();
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
