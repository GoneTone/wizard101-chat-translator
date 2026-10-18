//! 逐字重現 Python `repr(str)`，讓範例集指紋與 Python 版算出同一個 sha1。

use std::sync::LazyLock;

use regex::Regex;

// Python `str.isprintable` 的反面：Cc／Cf／Cs／Co／Cn 與 Zl／Zp／Zs（空格除外）
static NOT_PRINTABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{C}\p{Z}--[ ]]").unwrap_or_else(|e| panic!("bad regex: {e}")));

/// Python `repr(str)`：引號選擇與跳脫規則同 CPython。
pub fn repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if NOT_PRINTABLE.is_match(c.encode_utf8(&mut [0; 4])) => {
                let code = c as u32;
                out.push_str(&match code {
                    0..=0xff => format!("\\x{code:02x}"),
                    0x100..=0xffff => format!("\\u{code:04x}"),
                    _ => format!("\\U{code:08x}"),
                });
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}
