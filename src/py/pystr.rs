//! 對齊 Python `str` 的空白、換行與大小寫摺疊語意（std 的近似函式與其不同）。

/// Python `str.isspace` 認定的空白（比 `char::is_whitespace` 多 `\x1c`–`\x1f`）。
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// Python `str.strip()`。
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// Python `str.lstrip()`。
pub fn lstrip(s: &str) -> &str {
    s.trim_start_matches(is_space)
}

/// Python `str.rstrip()`。
pub fn rstrip(s: &str) -> &str {
    s.trim_end_matches(is_space)
}

/// Python `" ".join(s.split())`：連續空白折成單一空格並去頭尾。
pub fn one_line(s: &str) -> String {
    s.split(is_space)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Python `str.casefold()`（完整大小寫摺疊，如 `ß` → `ss`）。
pub fn casefold(s: &str) -> String {
    caseless::default_case_fold_str(s)
}

/// Python `str.splitlines()`：依 `\n \r \r\n \v \f \x1c-\x1e \x85 \u{2028} \u{2029}` 切行，不保留換行符。
pub fn splitlines(s: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if !matches!(
            c,
            '\n' | '\r' | '\x0b' | '\x0c' | '\x1c'..='\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}'
        ) {
            continue;
        }
        lines.push(&s[start..i]);
        start = i + c.len_utf8();
        if c == '\r' && matches!(chars.peek(), Some((_, '\n'))) {
            chars.next();
            start += 1;
        }
    }
    if start < s.len() {
        lines.push(&s[start..]);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::one_line;

    #[test]
    fn one_line_splits_on_python_whitespace() {
        assert_eq!(one_line(" a\x1cb \u{3000}c\n"), "a b c");
    }
}
