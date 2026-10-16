//! SSE 串流的逐行解析：把位元組塊切成行、取出 `data:` 內容；OpenAI／Claude／Gemini 共用。

use std::ops::ControlFlow;

use futures_util::StreamExt;

use super::connection_error;
use crate::translation::errors::TranslateError;

/// 把任意切分的位元組塊重組成行（以 `\n` 為界，去掉行尾 `\r`）。
#[derive(Default)]
pub struct LineBuffer {
    pending: Vec<u8>,
}

impl LineBuffer {
    /// 餵入一塊位元組，回傳其中已完整的行。
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(chunk);
        let mut lines = Vec::new();
        while let Some(end) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            lines.push(decode(&line[..end]));
        }
        lines
    }

    /// 串流結束時取出沒有換行收尾的最後一行。
    pub fn finish(&mut self) -> Option<String> {
        (!self.pending.is_empty()).then(|| decode(&std::mem::take(&mut self.pending)))
    }
}

fn decode(line: &[u8]) -> String {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    String::from_utf8_lossy(line).into_owned()
}

/// `data:` 行的內容（去前後空白）；其他行（註解、`event:`、空行）回 `None`。
pub fn data_of(line: &str) -> Option<&str> {
    line.strip_prefix("data:").map(str::trim)
}

/// 逐筆把回應的 `data:` 內容交給 `on_data`；它回 `Break` 或串流結束即停止。
/// 讀取中途斷線以 [`TranslateError::Offline`] 回報。
pub async fn for_each_data(
    resp: reqwest::Response,
    mut on_data: impl FnMut(&str) -> Result<ControlFlow<()>, TranslateError>,
) -> Result<(), TranslateError> {
    let mut stream = resp.bytes_stream();
    let mut lines = LineBuffer::default();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(connection_error)?;
        for line in lines.push(&chunk) {
            if let Some(data) = data_of(&line)
                && on_data(data)?.is_break()
            {
                return Ok(());
            }
        }
    }
    if let Some(line) = lines.finish()
        && let Some(data) = data_of(&line)
    {
        let _ = on_data(data)?;
    }
    Ok(())
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
}
