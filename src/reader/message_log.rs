//! messages.log：逐筆記錄 AddLine hook 讀到的聊天行與 reader 的判定，供事後 debug。
//!
//! 原文不清理不過濾（只排除遊戲自己的除錯行）；雙開時每個客戶端一個實例共寫同一串流，以 `[slot=N]` 前綴區分。

use std::io::Write;
use std::sync::{Arc, LazyLock, Mutex};

use crate::util::lock;

use regex::Regex;

use super::mem_reader::MessageSink;

/// 多個客戶端共寫的 messages.log 串流（實機為 `TimestampedWriter<File>`，時戳由它補）。
pub type SharedWriter = Arc<Mutex<dyn Write + Send>>;

// 遊戲把自己的除錯輸出也寫進聊天框，以 [WARN]／[ERRO]／[DBGM] 之類全大寫標籤開頭
static DEBUG_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\[[A-Z]{3,5}\]").unwrap());

/// 這行是遊戲自己的除錯輸出（而非聊天）嗎？
pub fn is_debug_line(raw: &str) -> bool {
    DEBUG_LINE.is_match(raw)
}

/// 把每筆 hook 記錄與判定寫進共用串流。
pub struct MessageLog {
    writer: SharedWriter,
    prefix: String,
}

impl MessageLog {
    /// 某個客戶端（`slot`）的寫入端。
    pub fn new(writer: SharedWriter, slot: u32) -> Self {
        Self {
            writer,
            prefix: format!("[slot={slot}] "),
        }
    }

    fn write(&self, line: &str) {
        let line = format!("{}{line}\n", self.prefix);
        // 整行一次寫出：多個客戶端共寫時才不會互相切斷；寫不進去無處可報，丟棄
        let _ = lock(&self.writer).write_all(line.as_bytes());
    }
}

impl MessageSink for MessageLog {
    fn record(&mut self, seq: u64, tab: u32, raw: &str, verdict: &str) {
        if !is_debug_line(raw) {
            self.write(&format!("[seq={seq} tab={tab} {verdict}] {raw}"));
        }
    }

    fn note(&mut self, text: &str) {
        self.write(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAY: &str = "<color;FF66CC><image;Art/Art_Chat_Say.dds;24;24;FF><link;GID:1,Bob,2>[Bob]</link> hi &lt;3 </color>";
    const SYS: &str =
        "<color;FFFFFF><image;Art/Art_Chat_System.dds;24;24;FF> A wizard has joined </color>";
    const WARN: &str = "[WARN] Texture was not loaded correctly</color>";
    const DBGM: &str = "[DBGM] ------ CloseInteraction</color>";

    fn log(slot: u32) -> (Arc<Mutex<Vec<u8>>>, MessageLog) {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer: SharedWriter = buf.clone();
        (buf, MessageLog::new(writer, slot))
    }

    fn text(buf: &Mutex<Vec<u8>>) -> String {
        String::from_utf8(buf.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn record_writes_seq_tab_verdict_and_raw_markup() {
        let (buf, mut log) = log(1);
        log.record(7, 1, SAY, "player");
        assert_eq!(text(&buf), format!("[slot=1] [seq=7 tab=1 player] {SAY}\n"));
    }

    #[test]
    fn note_writes_the_text_as_is() {
        let (buf, mut log) = log(1);
        log.note("lost 3 chat lines (seq 4-9)");
        assert_eq!(text(&buf), "[slot=1] lost 3 chat lines (seq 4-9)\n");
    }

    #[test]
    fn debug_lines_are_not_recorded() {
        let (buf, mut log) = log(1);
        log.record(1, 0, WARN, "dropped");
        log.record(2, 0, SYS, "system-off");
        assert_eq!(
            text(&buf),
            format!("[slot=1] [seq=2 tab=0 system-off] {SYS}\n")
        );
    }

    #[test]
    fn game_debug_tags_are_recognised() {
        assert!(is_debug_line(WARN));
        assert!(is_debug_line(DBGM));
        assert!(is_debug_line(
            "[ERRO] Texture::LoadTexture(FriendlyPlayer) FAILED!</color>"
        ));
        assert!(is_debug_line("  [INFO] x"));
    }

    #[test]
    fn chat_lines_are_not_debug_lines() {
        assert!(!is_debug_line(SAY));
        assert!(!is_debug_line(SYS));
        assert!(!is_debug_line("[Bob] hi"));
    }

    #[test]
    fn slot_prefix_marks_every_line() {
        // 雙開時兩條 reader 共寫同一份 messages.log，沒前綴就分不出哪個帳號的記錄
        let (buf, mut log) = log(2);
        log.record(1, 0, SAY, "player");
        log.note("lost 1 chat lines");
        let text = text(&buf);
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.starts_with("[slot=2] ")));
    }

    #[test]
    fn plugs_into_the_reader_as_its_message_sink() {
        let (buf, log) = log(3);
        let mut sink: Box<dyn MessageSink> = Box::new(log);
        sink.note("x");
        assert_eq!(text(&buf), "[slot=3] x\n");
    }
}
