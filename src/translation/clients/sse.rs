//! SSE 串流的逐行解析：把位元組塊切成行、取出 `data:` 內容；OpenAI／Claude／Gemini 共用。

use std::time::Duration;

use futures_util::StreamExt;
use serde_json::Value;
use tokio::time::{Instant, timeout_at};

use super::connection_error;
use crate::logs::log;
use crate::py::pystr;
use crate::translation::errors::{TranslateError, error_detail};

/// 非 `data:` 行最多留這麼多 bytes 當樣本，回應不是事件串流時拿來說明。
const SAMPLE_BYTES: usize = 200;

/// 把任意切分的位元組塊重組成行（以 `\n` 為界，去掉行尾 `\r`）。
#[derive(Default)]
struct LineBuffer {
    pending: Vec<u8>,
}

impl LineBuffer {
    /// 餵入一塊位元組，回傳其中已完整的行。
    fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(chunk);
        let mut lines = Vec::new();
        while let Some(end) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            lines.push(decode(&line[..end]));
        }
        lines
    }

    /// 串流結束時取出沒有換行收尾的最後一行。
    fn finish(&mut self) -> Option<String> {
        (!self.pending.is_empty()).then(|| decode(&std::mem::take(&mut self.pending)))
    }
}

fn decode(line: &[u8]) -> String {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    String::from_utf8_lossy(line).into_owned()
}

/// `data:` 行的內容（去前後空白）；其他行（註解、`event:`、空行）回 `None`。
fn data_of(line: &str) -> Option<&str> {
    line.strip_prefix("data:").map(pystr::strip)
}

/// `on_data` 處理完一筆 `data:` 後的下一步。
pub(super) enum Next {
    /// 有進展：閒置期限重新計時。
    Continue,
    /// 只是保持連線（如 Claude 的 `ping`）：不重新計時。
    KeepAlive,
    /// 停止讀取。
    Stop,
}

/// 讀到的行數與非 `data:` 行的樣本。
#[derive(Default)]
struct Seen {
    data_lines: usize,
    sample: String,
}

impl Seen {
    /// 處理一行：`data:` 交給 `on_data` 並回傳它的下一步，其他行記進樣本。
    fn line(
        &mut self,
        line: &str,
        on_data: &mut impl FnMut(&str) -> Result<Next, TranslateError>,
    ) -> Result<Option<Next>, TranslateError> {
        if let Some(data) = data_of(line) {
            self.data_lines += 1;
            return on_data(data).map(Some);
        }
        // SSE 註解行是保持連線用的，不代表回應不是事件串流
        if !line.is_empty() && !line.starts_with(':') && self.sample.len() < SAMPLE_BYTES {
            if !self.sample.is_empty() {
                self.sample.push('\n');
            }
            self.sample.push_str(line);
            self.sample
                .truncate(self.sample.floor_char_boundary(SAMPLE_BYTES));
        }
        Ok(None)
    }
}

/// 逐筆把回應的 `data:` 內容交給 `on_data`；它回 [`Next::Stop`] 或串流結束即停止。
///
/// 中途斷線、最後一行斷在半截、超過 `idle` 沒有新的 `data:` 都回 [`TranslateError::Offline`]；
/// 一筆 `data:` 都沒有卻有其他內容（錯誤 JSON、HTML 頁面）回 `Unexpected`。`label` 是 log 用的服務商與模型。
pub(super) async fn for_each_data(
    resp: reqwest::Response,
    label: &str,
    idle: Duration,
    mut on_data: impl FnMut(&str) -> Result<Next, TranslateError>,
) -> Result<(), TranslateError> {
    let mut stream = resp.bytes_stream();
    let mut lines = LineBuffer::default();
    let mut seen = Seen::default();
    // reqwest 的讀取逾時每收到一個 frame 就重新計時，擋不住只送註解行的閘道
    let mut deadline = Instant::now() + idle;
    loop {
        let chunk = match timeout_at(deadline, stream.next()).await {
            Ok(Some(chunk)) => chunk.map_err(connection_error)?,
            Ok(None) => break,
            Err(_) => {
                log(format!(
                    "[translate] stream idle for {:.1}s without data, giving up: {label} data_lines={}",
                    idle.as_secs_f64(),
                    seen.data_lines
                ));
                return Err(TranslateError::offline(format!(
                    "stream idle for {:.1}s without data",
                    idle.as_secs_f64()
                )));
            }
        };
        for line in lines.push(&chunk) {
            match seen.line(&line, &mut on_data)? {
                Some(Next::Stop) => return Ok(()),
                Some(Next::Continue) => deadline = Instant::now() + idle,
                Some(Next::KeepAlive) | None => {}
            }
        }
    }
    if let Some(line) = lines.finish() {
        if data_of(&line).is_some_and(cut_off) {
            log(format!(
                "[translate] stream ended mid-line: {label} data_lines={} line_bytes={}",
                seen.data_lines,
                line.len()
            ));
            return Err(TranslateError::offline("stream ended mid-line"));
        }
        if let Some(Next::Stop) = seen.line(&line, &mut on_data)? {
            return Ok(());
        }
    }
    if seen.data_lines == 0 && !seen.sample.is_empty() {
        let detail = error_detail(&seen.sample);
        log(format!(
            "[translate] response is not an event stream: {label} sample={detail}"
        ));
        return Err(TranslateError::Unexpected(format!(
            "response is not an event stream: {detail}"
        )));
    }
    Ok(())
}

/// 沒有換行收尾又解析不了的 `data:`：連線斷在一行中間。
fn cut_off(data: &str) -> bool {
    // 各家的 data 都是 JSON，唯一的例外是 OpenAI 的結束標記
    data != "[DONE]" && serde_json::from_str::<Value>(data).is_err()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_survive_arbitrary_chunk_boundaries() {
        let mut buf = LineBuffer::default();
        let mut out = buf.push(b"data: {\"a\"");
        out.extend(buf.push(b":1}\r\n\r\ndata: [DO"));
        out.extend(buf.push(b"NE]\n"));
        assert_eq!(out, ["data: {\"a\":1}", "", "data: [DONE]"]);
        assert_eq!(buf.finish(), None);
    }

    #[test]
    fn multibyte_split_across_chunks_is_not_mangled() {
        let bytes = "data: 譯文\n".as_bytes();
        let mut buf = LineBuffer::default();
        let mut out = buf.push(&bytes[..8]);
        out.extend(buf.push(&bytes[8..]));
        assert_eq!(out, ["data: 譯文"]);
    }

    #[test]
    fn last_line_without_newline_is_flushed() {
        let mut buf = LineBuffer::default();
        assert!(buf.push(b"data: x").is_empty());
        assert_eq!(buf.finish().as_deref(), Some("data: x"));
    }

    #[test]
    fn data_of_ignores_non_data_lines() {
        assert_eq!(data_of("data:  hi "), Some("hi"));
        assert_eq!(data_of(": comment"), None);
        assert_eq!(data_of("event: x"), None);
    }

    #[test]
    fn sample_skips_comments_and_is_capped_on_a_char_boundary() {
        let mut seen = Seen::default();
        let mut on_data = |_: &str| Ok(Next::Continue);
        for line in [": keep-alive", "", "<html>", &"譯".repeat(100)] {
            assert!(seen.line(line, &mut on_data).unwrap().is_none());
        }
        assert!(seen.sample.starts_with("<html>\n譯"), "{}", seen.sample);
        assert!(seen.sample.len() <= SAMPLE_BYTES);
        assert_eq!(seen.data_lines, 0);
    }
}
