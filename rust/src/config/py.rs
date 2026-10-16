//! Python 相容的 JSON 輸出與值的表示法：寫回的 config.json 與 log 文字要和 Python 版逐字相同。

use serde_json::{Number, Value};

/// 等同 Python `json.dumps(value, ensure_ascii=False, indent=2)`。
pub(crate) fn dumps_indent2(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0);
    out
}

fn newline(out: &mut String, depth: usize) {
    out.push('\n');
    out.push_str(&"  ".repeat(depth));
}

fn write_value(out: &mut String, value: &Value, depth: usize) {
    match value {
        Value::Array(items) if !items.is_empty() => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                write_value(out, item, depth + 1);
            }
            newline(out, depth);
            out.push(']');
        }
        Value::Object(map) if !map.is_empty() => {
            out.push('{');
            for (i, (key, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                out.push_str(&Value::String(key.clone()).to_string());
                out.push_str(": ");
                write_value(out, item, depth + 1);
            }
            newline(out, depth);
            out.push('}');
        }
        Value::Number(n) => out.push_str(&number_repr(n)),
        // serde_json 的字串跳脫（控制字元 \u00xx 小寫、不跳脫非 ASCII）與 Python 相同
        other => out.push_str(&other.to_string()),
    }
}

fn number_repr(n: &Number) -> String {
    match n.as_f64() {
        Some(x) if n.is_f64() => float_repr(x),
        _ => n.to_string(),
    }
}

/// Python `repr(float)`：最短來回位數，指數小於 -4 或不小於 16 時改用科學記號。
pub(crate) fn float_repr(x: f64) -> String {
    let sci = format!("{x:e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(m) => ("-", m),
        None => ("", mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let body = if (-4..16).contains(&exp) {
        let point = exp + 1;
        if point <= 0 {
            format!("0.{}{digits}", "0".repeat(point.unsigned_abs() as usize))
        } else if point as usize >= digits.len() {
            format!("{digits}{}.0", "0".repeat(point as usize - digits.len()))
        } else {
            let (int, frac) = digits.split_at(point as usize);
            format!("{int}.{frac}")
        }
    } else {
        let (first, rest) = digits.split_at(1);
        let frac = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let exp_sign = if exp < 0 { '-' } else { '+' };
        format!("{first}{frac}e{exp_sign}{:02}", exp.unsigned_abs())
    };
    format!("{sign}{body}")
}

/// Python `repr(value)`。
pub(crate) fn repr(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(b) => py_bool(*b).into(),
        Value::Number(n) => number_repr(n),
        Value::String(s) => str_repr(s),
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(repr).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}: {}", str_repr(k), repr(v)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

/// Python `str(value)`（f-string 不帶 `!r` 時的樣子）。
pub(crate) fn display(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => repr(other),
    }
}

/// Python 的 `True`／`False`。
pub(crate) fn py_bool(b: bool) -> &'static str {
    if b { "True" } else { "False" }
}

/// Python `repr(str)`：引號選擇與跳脫規則同 CPython。
pub(crate) fn str_repr(s: &str) -> String {
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
            // shortcut: 不可列印只認控制字元與空白類，Cf／Co／Cn 照原樣輸出；只影響 log 文字
            c if c.is_control() || (c.is_whitespace() && c != ' ') => {
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

/// Python `type(value).__name__`。
pub(crate) fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// Python 真值判定；缺值視同 `None`。
pub(crate) fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(m)) => !m.is_empty(),
    }
}

/// Python `==`：bool 與數字互通（`False == 0`、`1 == 1.0`），dict 不看鍵順序。
pub(crate) fn eq(a: &Value, b: &Value) -> bool {
    let num = |v: &Value| match v {
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Number(n) => n.as_f64(),
        _ => None,
    };
    match (a, b) {
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| eq(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| eq(v, w)))
        }
        _ => match (num(a), num(b)) {
            (Some(x), Some(y)) => x == y,
            _ => a == b,
        },
    }
}

/// `not s.strip()`：Python 的空白比 Rust 多認 U+001C～U+001F。
pub(crate) fn is_blank(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn float_repr_matches_python() {
        let cases = [
            (0.8, "0.8"),
            (1.0, "1.0"),
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1e-5, "1e-05"),
            (1e-4, "0.0001"),
            (1e15, "1000000000000000.0"),
            (1e16, "1e+16"),
            (1.5e300, "1.5e+300"),
            (123.456, "123.456"),
            (5e-324, "5e-324"),
        ];
        for (x, expected) in cases {
            assert_eq!(float_repr(x), expected, "{x}");
        }
    }

    #[test]
    fn repr_matches_python() {
        assert_eq!(repr(&json!("it's")), "\"it's\"");
        assert_eq!(repr(&json!("a'\"")), "'a\\'\"'");
        assert_eq!(repr(&json!("\u{1}\u{a0}\u{2028}")), "'\\x01\\xa0\\u2028'");
        assert_eq!(
            repr(&json!([null, true, 1, 2.0, {"k": "v"}])),
            "[None, True, 1, 2.0, {'k': 'v'}]"
        );
    }

    #[test]
    fn eq_follows_python_numeric_equality() {
        assert!(eq(&json!(false), &json!(0)));
        assert!(eq(&json!(1), &json!(1.0)));
        assert!(eq(&json!({"a": 1, "b": 2}), &json!({"b": 2, "a": 1})));
        assert!(!eq(&json!(null), &json!(false)));
        assert!(!eq(&json!("1"), &json!(1)));
    }

    #[test]
    fn blank_follows_python_strip() {
        assert!(is_blank(" \u{3000}\u{1c}\t"));
        assert!(!is_blank(" x "));
    }
}
