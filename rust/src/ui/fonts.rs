//! 介面字型：依介面語言的文字系統選字族。

use crate::i18n::chinese_is_traditional;

const FALLBACK_FAMILY: &str = "Segoe UI";

/// 語言碼 → 介面字族；認不得或沒有專用字族時回保底字族。
pub fn font_family(code: &str) -> &'static str {
    match chinese_is_traditional(code) {
        Some(true) => return "Microsoft JhengHei",
        Some(false) => return "Microsoft YaHei",
        None => {}
    }
    match code.to_ascii_lowercase().split(['-', '_']).next() {
        Some("ja") => "Yu Gothic UI",
        Some("ko") => "Malgun Gothic",
        _ => FALLBACK_FAMILY,
    }
}
