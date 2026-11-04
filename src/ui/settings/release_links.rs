//! release 內文的連結預處理：CommonMark 不會把裸網址變成連結，這裡先改寫成 Markdown 連結。

use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::runtime::updater::PROJECT_URL;

/// 依序比對：既有連結、`<url>` 自動連結（原樣保留）、裸網址、`@使用者`。
static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?P<keep>\[[^\]]*\]\([^)]*\) | <https?://[^>\s]+>)
        | (?P<url>https?://[^\s<>]+)
        | @(?P<user>[A-Za-z0-9][A-Za-z0-9-]*)(?P<bot>\[bot\])?",
    )
    .expect("valid regex")
});

/// 網址尾端的句讀不算網址的一部分。
const TRAILING: &[char] = &[
    '.', ',', ';', ':', '!', '?', ')', ']', '}', '\'', '"', '。', '，', '、', '；', '：', '！',
    '？', '）', '」', '』',
];

/// 把裸網址與 `@使用者` 改寫成 Markdown 連結；程式碼區塊與行內程式碼不動。
pub fn linkify_github(body: &str) -> String {
    let mut fenced = false;
    body.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                fenced = !fenced;
                return line.to_owned();
            }
            if fenced || line.len() - trimmed.len() >= 4 {
                return line.to_owned();
            }
            // 奇數段在反引號之間，是行內程式碼
            line.split('`')
                .enumerate()
                .map(|(i, seg)| {
                    if i % 2 == 0 {
                        linkify(seg)
                    } else {
                        seg.to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("`")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn linkify(text: &str) -> String {
    TOKEN
        .replace_all(text, |c: &Captures| {
            let whole = c.get(0).expect("whole match");
            if c.name("keep").is_some() {
                return whole.as_str().to_owned();
            }
            if let Some(url) = c.name("url") {
                let url = url.as_str();
                let clean = url.trim_end_matches(TRAILING);
                return format!("{}{}", url_link(clean), &url[clean.len()..]);
            }
            // 前一個字元若是帳號／路徑字元，就是信箱或路徑而不是提及
            let before = text[..whole.start()].chars().next_back();
            if before.is_some_and(|b| b.is_ascii_alphanumeric() || "._%+@/[-".contains(b)) {
                return whole.as_str().to_owned();
            }
            let user = &c["user"];
            match c.name("bot") {
                Some(_) => format!("[@{user}[bot]](https://github.com/apps/{user})"),
                None => format!("[@{user}](https://github.com/{user})"),
            }
        })
        .into_owned()
}

fn url_link(url: &str) -> String {
    let rest = url
        .strip_prefix(PROJECT_URL)
        .and_then(|r| r.strip_prefix('/'));
    let label = rest.and_then(|r| {
        let (kind, id) = r.split_once('/')?;
        match kind {
            "pull" | "issues" if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => {
                Some(format!("#{id}"))
            }
            "compare" if !id.is_empty() => Some(id.to_owned()),
            "commit"
                if (7..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit()) =>
            {
                Some(id[..7].to_owned())
            }
            _ => None,
        }
    });
    match label {
        Some(label) => format!("[{label}]({url})"),
        None => format!("<{url}>"),
    }
}
