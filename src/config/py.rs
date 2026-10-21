//! 沿用 Python 語意的值判斷與表示法：int／float 區分、真值、相等，以及 log 與錯誤訊息裡的 repr。

use serde_json::{Number, Value};

use crate::py::pyrepr::repr_str;

/// 數字字面值是不是 Python 的 float（json.loads 依有無 `.`／`e` 決定 int 或 float）。
fn is_float(n: &Number) -> bool {
    n.as_str().contains(['.', 'e', 'E'])
}

fn to_f64(n: &Number) -> f64 {
    n.as_str().parse().unwrap_or(f64::NAN)
}

/// Python 數值比較用的值；bool 視為 0／1，超出 f64 的整數取近似值。
pub(crate) fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Number(n) => Some(to_f64(n)),
        _ => None,
    }
}

/// int 保留原字面值（大整數不失真），只把 JSON 的 `-0` 正規化成 Python 的 `0`。
fn int_repr(n: &Number) -> &str {
    match n.as_str() {
        "-0" => "0",
        s => s,
    }
}

fn number_repr(n: &Number) -> String {
    if is_float(n) {
        float_repr(to_f64(n))
    } else {
        int_repr(n).to_string()
    }
}

/// Python `repr(float)`：最短來回位數，指數小於 -4 或不小於 16 時改用科學記號。
pub(crate) fn float_repr(x: f64) -> String {
    if !x.is_finite() {
        return match x {
            x if x.is_nan() => "nan".into(),
            x if x > 0.0 => "inf".into(),
            _ => "-inf".into(),
        };
    }
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
        Value::String(s) => repr_str(s),
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(repr).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}: {}", repr_str(k), repr(v)))
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

/// Python `type(value).__name__`。
pub(crate) fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(n) if is_float(n) => "float",
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
        Some(Value::Number(n)) => to_f64(n) != 0.0,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(m)) => !m.is_empty(),
    }
}

/// Python `==`：bool 與數字互通（`False == 0`、`1 == 1.0`），dict 不看鍵順序。
pub(crate) fn eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) if !is_float(x) && !is_float(y) => {
            int_repr(x) == int_repr(y)
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| eq(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| eq(v, w)))
        }
        _ => match (number(a), number(b)) {
            (Some(x), Some(y)) => x == y,
            _ => a == b,
        },
    }
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
    fn big_ints_and_overflowing_floats_keep_their_type() {
        let value: Value =
            serde_json::from_str(r#"{"b": 123456789012345678901234567890, "c": 1e400}"#).unwrap();
        assert_eq!(repr(&value["c"]), "inf");
        assert_eq!(type_name(&value["b"]), "int");
        assert_eq!(type_name(&value["c"]), "float");
    }

    #[test]
    fn eq_compares_big_ints_exactly() {
        let parse = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        assert!(!eq(
            &parse("123456789012345678901234567890"),
            &parse("123456789012345678901234567891")
        ));
        assert!(eq(&parse("-0"), &parse("0")));
        assert!(eq(&parse("-0"), &json!(false)));
    }

    #[test]
    fn eq_follows_python_numeric_equality() {
        assert!(eq(&json!(false), &json!(0)));
        assert!(eq(&json!(1), &json!(1.0)));
        assert!(eq(&json!({"a": 1, "b": 2}), &json!({"b": 2, "a": 1})));
        assert!(!eq(&json!(null), &json!(false)));
        assert!(!eq(&json!("1"), &json!(1)));
    }
}
