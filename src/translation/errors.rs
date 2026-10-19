//! 翻譯失敗的分類，以及把 HTTP 錯誤回應整理成可顯示說明的共用函式。

use std::fmt;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::py::pystr;

/// 翻譯請求失敗的分類；`detail` 是 API 回應裡的說明或連線失敗原因，UI 與 log 直接顯示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslateError {
    /// 可重試：連線失敗、逾時、408、409、429、5xx。
    Offline { status: Option<u16>, detail: String },
    /// 設定錯誤（4xx）：持續重試，使用者修好設定後自動恢復。
    Config { status: Option<u16>, detail: String },
    /// 不可重試的輸出異常：譯文在 `max_tokens` 被截斷，或模型沒有回任何文字。
    BadOutput(String),
    /// 請求被呼叫端取消，不是錯誤。
    Cancelled,
    /// 端點不提供模型清單，使用者仍可自行輸入模型名稱。
    NoModelList,
    /// 對應 Python 未分類的例外（回應格式異常等），呼叫端依「非預期錯誤」處理。
    Unexpected(String),
}

/// 金鑰含 HTTP 標頭不允許的字元時的說明（請求根本沒送出）。
const INVALID_KEY_DETAIL: &str = "api key contains characters invalid in an HTTP header";

impl TranslateError {
    /// 金鑰格式錯誤：歸為設定錯誤（使用者改好金鑰即恢復），UI 顯示金鑰無效。
    pub fn invalid_key() -> Self {
        Self::Config {
            status: None,
            detail: INVALID_KEY_DETAIL.into(),
        }
    }

    /// 連線層失敗（無狀態碼）。
    pub fn offline(detail: impl Into<String>) -> Self {
        Self::Offline {
            status: None,
            detail: detail.into(),
        }
    }

    /// Python 版對應的例外類別名稱，log 沿用 `類別: 訊息` 的格式。
    pub fn class_name(&self) -> &'static str {
        match self {
            Self::Offline { .. } => "TranslatorOffline",
            Self::Config { .. } => "TranslatorConfigError",
            Self::BadOutput(_) => "TranslatorBadOutput",
            Self::Cancelled => "TranslatorCancelled",
            Self::NoModelList => "TranslatorNoModelList",
            Self::Unexpected(_) => "Exception",
        }
    }

    /// HTTP 狀態碼；連線層失敗與非 HTTP 類別為 `None`。
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Offline { status, .. } | Self::Config { status, .. } => *status,
            _ => None,
        }
    }
}

impl fmt::Display for TranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Offline { status, detail } | Self::Config { status, detail } => {
                match (status, detail.is_empty()) {
                    (Some(s), false) => write!(f, "HTTP {s}: {detail}"),
                    (Some(s), true) => write!(f, "HTTP {s}"),
                    (None, _) => f.write_str(detail),
                }
            }
            Self::BadOutput(message) | Self::Unexpected(message) => f.write_str(message),
            Self::Cancelled => f.write_str("request cancelled"),
            Self::NoModelList => f.write_str("endpoint provides no model list"),
        }
    }
}

impl std::error::Error for TranslateError {}

static HTML_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
static REJECTED_PARAM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"Unrecognized request argument supplied: (\w+)|Unsupported (?:parameter|value): '(\w+)'",
    )
    .unwrap()
});

/// 折成單行（不截斷 —— 訊息尾端常是說明網址）。
pub fn one_line(text: &str) -> String {
    text.split(pystr::is_space)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// OpenAI／Anthropic 的 `error.message`，或自架後端直接放字串的 `error`／`message`。
pub(crate) fn message_of(body: &Value) -> Option<&str> {
    let body = body.as_object()?;
    match body.get("error") {
        Some(Value::Object(error)) => {
            if let Some(Value::String(message)) = error.get("message") {
                return Some(message);
            }
        }
        Some(Value::String(error)) => return Some(error),
        _ => {}
    }
    body.get("message")?.as_str()
}

/// 把錯誤回應 body 整理成單行說明：JSON 取錯誤欄位（單元素陣列先拆開），HTML 去標籤，其餘照原文。
pub fn error_detail(text: &str) -> String {
    let message = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|body| match body {
            Value::Array(mut items) if items.len() == 1 => {
                message_of(&items.remove(0)).map(str::to_owned)
            }
            body => message_of(&body).map(str::to_owned),
        })
        .unwrap_or_else(|| HTML_TAG.replace_all(text, " ").into_owned());
    one_line(&message)
}

/// 從 400 的說明文字抓出被拒的參數名；認不出來回 `None`。
pub fn rejected_parameter(detail: &str) -> Option<String> {
    let caps = REJECTED_PARAM.captures(detail)?;
    caps.get(1)
        .or_else(|| caps.get(2))
        .map(|m| m.as_str().to_owned())
}

/// 翻譯請求的狀態碼對應：408／409／429／5xx 是暫時性錯誤，其餘 4xx 當設定錯誤；`None` 表示可繼續解析。
pub fn status_error(status: u16, detail: &str) -> Option<TranslateError> {
    let (status, detail) = (Some(status), detail.to_owned());
    match status {
        Some(408 | 409 | 429 | 500..) => Some(TranslateError::Offline { status, detail }),
        Some(400..) => Some(TranslateError::Config { status, detail }),
        _ => None,
    }
}

/// 模型清單請求的狀態碼對應：404／405 是「端點不提供清單」，其餘沿用 [`status_error`]。
pub fn model_list_error(status: u16, detail: &str) -> Option<TranslateError> {
    match status {
        404 | 405 => Some(TranslateError::NoModelList),
        _ => status_error(status, detail),
    }
}

/// 翻譯例外 → （文案 key，format 變數），顯示端一律 `t(key, args)`。
///
/// 有 API 說明（detail）就照實顯示；狀態碼猜的提示會誤導，只在 API 什麼都沒說時才用。
pub fn friendly_error(e: &TranslateError) -> (&'static str, Vec<(&'static str, String)>) {
    use TranslateError::{Config, Offline};
    match e {
        Config {
            status: None,
            detail,
        } if detail == INVALID_KEY_DETAIL => ("error.bad_key", vec![]),
        Config { status, detail } | Offline { status, detail } if !detail.is_empty() => {
            match status {
                Some(status) => (
                    "error.api_response",
                    vec![("status", status.to_string()), ("message", detail.clone())],
                ),
                None => ("error.offline_detail", vec![("message", detail.clone())]),
            }
        }
        Config {
            status: Some(401 | 403),
            ..
        } => ("error.bad_key", vec![]),
        Config {
            status: Some(404), ..
        } => ("error.model_not_found", vec![]),
        Config { status, .. } => (
            "error.api_http",
            vec![(
                "status",
                status.map_or_else(|| "None".to_owned(), |s| s.to_string()),
            )],
        ),
        Offline {
            status: Some(status),
            ..
        } => ("error.server_http", vec![("status", status.to_string())]),
        Offline { status: None, .. } => ("error.offline", vec![]),
        other => ("error.unexpected", vec![("error", other.to_string())]),
    }
}

#[cfg(test)]
mod tests {
    use super::one_line;

    #[test]
    fn one_line_splits_on_python_whitespace() {
        assert_eq!(one_line(" a\x1cb \u{3000}c\n"), "a b c");
    }
}
