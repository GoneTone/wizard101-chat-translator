//! 聊天富文字標記的純解析：去標記、判斷發言者／系統訊息、取遊戲顯示色。
//!
//! 聊天在顯示層是帶標記的富文字，每則一行：
//! 他人 `<color;..><image;Art/Art_Chat_Say.dds;..> <link;GID:<id>,<名>,2>[<名>]</link> 內文 </color>`，
//! 自己 `... [你] 內文 </color>`（無 `<link;GID>`）。玩家發言帶頻道圖示、系統訊息用
//! `Art_Chat_System`、除錯行無圖示，以此分流；去標記後回傳「[發送者] 內文」，連同行首顏色一起帶出。

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use regex::{Captures, Regex};

use crate::logs::log;
use crate::py::pyrepr::repr_str;
use crate::py::pystr;

/// 發送者前綴 `[名稱]`（名稱 1–40 字、不含 `]`）的正規表示式片段；玩家行判定與譯文後處理共用同一條。
pub const SENDER_PREFIX: &str = r"\[[^\]]{1,40}\]";

const PLAYER_IMG_PREFIXES: [&str; 3] = [
    "<image;Art/Art_Chat",
    "<image;Art/chat_balloon",
    "<image;Art/Art_Word_Balloon",
];
const SYSTEM_IMG: &str = "<image;Art/Art_Chat_System";
const PARTY_IMG: &str = "<image;Art/Art_Chat_Party";
const ART_IMG: &str = "<image;Art/";
const OTHER_PLAYER_LINK: &str = "<link;GID";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|e| panic!("bad regex {pattern}: {e}"))
}

// 顏色值為 6 位 RRGGBB 或 8 位 AARRGGBB，顯示色一律取後 6 位
static COLOR_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"<color;([0-9a-fA-F]{6,8})>"));
static TAG: LazyLock<Regex> = LazyLock::new(|| re(r"<[^>]*>"));
static VALID: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"^{SENDER_PREFIX} .+")));
// Python 的 IGNORECASE 讓 i 也吃 ı 與 İ（Rust 的簡單摺疊不會）
static EMOTE_TAG: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)<[iıİ]mage;Emot[iıİ]cons/([^.;>]+)\.dds[^>]*>"));
static ANY_ART_IMG: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)<[iıİ]mage;(Art/[^.;>]+)\.dds"));
static EMOTE_AFFIX: LazyLock<Regex> = LazyLock::new(|| re(r"^emoticons?_|\d+$"));
static EMOTE_NAME: LazyLock<Regex> = LazyLock::new(|| re(r"^[a-z0-9_]+$"));

// 所有客戶端共用的去重集合：任一客戶端連線成功時清空（失敗重試不清，否則每輪重洗）
static WARNED_ICONS: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);

/// 一行乾淨的聊天，帶遊戲顯示色。
///
/// `own` 是自己講的（無 `<link;GID>`）；`system` 是遊戲系統訊息或伺服器公告，走與玩家對話分離的翻譯路徑。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    pub text: String,
    pub color: Option<String>,
    pub own: bool,
    pub system: bool,
}

/// 清掉「已警告過的未知圖示」：重新連上（遊戲可能已改版）後要能再警告一次。
pub fn forget_warned_icons() {
    WARNED_ICONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

fn emote_to_text(caps: &Captures) -> String {
    let lowered = caps[1].to_lowercase();
    let name = EMOTE_AFFIX.replace_all(&lowered, "");
    // 撕裂的標記（名稱夾入雜字）：丟棄該表情，保留整行其餘內容
    if EMOTE_NAME.is_match(&name) {
        format!(":{name}:")
    } else {
        String::new()
    }
}

/// 表情標記保留成 `:名稱:`，去掉各種標記，還原 `&lt;` `&gt;` `&amp;`，壓縮空白。
pub fn clean(text: &str) -> String {
    let text = EMOTE_TAG.replace_all(text, emote_to_text);
    let text = TAG.replace_all(&text, "");
    let text = text
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace('\0', " ");
    text.split(pystr::is_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 從一行原始 markup 取遊戲顯示色（`#rrggbb` 小寫）；沒有顏色標記回傳 None。
pub fn line_color(raw: &str) -> Option<String> {
    let caps = COLOR_TAG.captures(raw)?;
    let hex = &caps[1];
    Some(format!("#{}", hex[hex.len() - 6..].to_ascii_lowercase()))
}

/// 有行首顏色、卻沒有任何 `Art/` 圖示 → 伺服器公告。
///
/// 帶 `Art/` 圖示的行先排除，未收錄的頻道才會落到未知圖示警告，不會被悄悄吞成系統訊息。
pub fn is_server_broadcast(raw: &str) -> bool {
    !raw.contains(ART_IMG) && pystr::lstrip(raw).starts_with("<color;")
}

fn system_line(raw: &str, line: String) -> Option<ChatLine> {
    (!line.is_empty()).then(|| ChatLine {
        text: line,
        color: line_color(raw),
        own: false,
        system: true,
    })
}

/// 把單行渲染 markup 解析成乾淨聊天行；非聊天行（除錯行、未知圖示、去標記後為空）回傳 None。
///
/// 收玩家發言（`[你]` 與 `<link;GID>[名]`）、系統訊息（含組隊通知）與伺服器公告；玩家行必須是
/// 「[發送者] 內容」，系統訊息與公告沒有固定前綴，去標記後非空即收。
pub fn parse_line(raw: &str) -> Option<ChatLine> {
    if raw.contains(SYSTEM_IMG) || is_server_broadcast(raw) {
        return system_line(raw, clean(raw));
    }
    if !PLAYER_IMG_PREFIXES.iter().any(|p| raw.contains(p)) {
        warn_unknown_icon(raw);
        return None;
    }
    let line = clean(raw);
    if !VALID.is_match(&line) {
        return if raw.contains(PARTY_IMG) {
            system_line(raw, line)
        } else {
            None
        };
    }
    Some(ChatLine {
        color: line_color(raw),
        own: !raw.contains(OTHER_PLAYER_LINK),
        text: line,
        system: false,
    })
}

/// 帶 `Art/` 圖示、格式像聊天行、但圖示不在白名單：每種圖示警告一次，漏接的頻道直接從 app.log 讀得到圖示名稱。
fn warn_unknown_icon(raw: &str) {
    let Some(caps) = ANY_ART_IMG.captures(raw) else {
        return;
    };
    let icon = &caps[1];
    if !VALID.is_match(&clean(raw)) {
        return;
    }
    let newly_seen = WARNED_ICONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(icon.to_owned());
    if newly_seen {
        let head: String = raw.chars().take(160).collect();
        log(format!(
            "[reader] unrecognized chat icon {}, line dropped (add its prefix to \
             PLAYER_IMG_PREFIXES if this is a player channel); raw={}",
            repr_str(icon),
            repr_str(&head)
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unknown_icon_line(icon: &str) -> String {
        format!("<color;FFFFFF><image;Art/{icon}.dds;24;24;FFFFFFFF> [Zed] hi </color>")
    }

    fn warned(icon: &str) -> bool {
        WARNED_ICONS
            .lock()
            .unwrap()
            .contains(&format!("Art/{icon}"))
    }

    #[test]
    fn unknown_icons_are_warned_once_per_icon() {
        let line = unknown_icon_line("Unit_Test_Icon_Marker");
        assert!(parse_line(&line).is_none());
        assert!(warned("Unit_Test_Icon_Marker"));
        assert!(parse_line(&line).is_none());
        WARNED_ICONS
            .lock()
            .unwrap()
            .remove("Art/Unit_Test_Icon_Marker");
        assert!(!warned("Unit_Test_Icon_Marker"));
    }

    #[test]
    fn clean_strips_markup_and_collapses_space() {
        let raw = "<color;FFFFFF><image;Art/x.dds;24;24;FFFFFFFF> [你] hi there </color>";
        assert_eq!(clean(raw), "[你] hi there");
    }

    #[test]
    fn clean_strips_emoticon_prefix_and_digits() {
        let emote = |name: &str| clean(&format!("<image;Emoticons/{name}.dds;24;24;FFFFFFFF>"));
        assert_eq!(emote("Emoticons_Heart"), ":heart:");
        assert_eq!(emote("Emoticons_School_Death"), ":school_death:");
        assert_eq!(emote("Eyes001"), ":eyes:");
        assert_eq!(emote("TeaCup001"), ":teacup:");
    }

    #[test]
    fn clean_keeps_emoticons_and_unescapes_entities() {
        let raw = "<link;GID:1,A,2>[Amy]</link> a &lt;3 <image;Emoticons/Laughter001.dds;24;24;1> &amp; c\0d";
        assert_eq!(clean(raw), "[Amy] a <3 :laughter: & c d");
    }
}
