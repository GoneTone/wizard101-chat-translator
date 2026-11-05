//! release 內文的連結預處理：CommonMark 不會把裸網址變成連結，這裡先改寫成 Markdown 連結。

use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::runtime::updater::PROJECT_URL;

/// 依序比對：既有連結、`<url>` 自動連結（原樣保留）、裸網址、`@使用者`。
static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?P<keep>\[[^\]]*\]\([^)]*\) | <https?://[^>\s]+>)
        | (?P<url>https?://[A-Za-z0-9\-._~:/?\#\[\]@!$&'()+,;=%]+)
        | @(?P<user>[A-Za-z0-9][A-Za-z0-9-]*)(?P<bot>\[bot\])?",
    )
    .expect("valid regex")
});

/// 網址尾端的 ASCII 句讀不算網址的一部分；`)` 另外依括號是否成對判斷。
const TRAILING: &[char] = &['.', ',', ';', ':', '!', '?', ']', '\'', '_'];

fn trim_trailing(url: &str) -> &str {
    let mut url = url;
    loop {
        let unbalanced = url.ends_with(')') && url.matches(')').count() > url.matches('(').count();
        if unbalanced || url.ends_with(TRAILING) {
            url = &url[..url.len() - 1];
        } else {
            return url;
        }
    }
}

/// 把裸網址與 `@使用者` 改寫成 Markdown 連結；程式碼區塊與行內程式碼不動（行內程式碼不跨行）。
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
            outside_code(line, linkify)
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
                let clean = trim_trailing(url);
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

/// 對 `line` 中不在行內程式碼裡的片段套用 `f`；N 個反引號開頭的程式碼在下一串剛好 N 個的反引號結束。
fn outside_code(line: &str, f: impl Fn(&str) -> String) -> String {
    let runs = |from: usize| {
        let mut found = Vec::new();
        let mut i = from;
        while let Some(off) = line[i..].find('`') {
            let start = i + off;
            let len = line[start..].len() - line[start..].trim_start_matches('`').len();
            found.push((start, len));
            i = start + len;
        }
        found
    };
    let all = runs(0);
    let (mut out, mut pos, mut k) = (String::new(), 0, 0);
    while k < all.len() {
        let (start, len) = all[k];
        match all[k + 1..].iter().position(|&(_, l)| l == len) {
            Some(j) => {
                let (cs, cl) = all[k + 1 + j];
                out += &f(&line[pos..start]);
                out += &line[start..cs + cl];
                pos = cs + cl;
                k += j + 2;
            }
            None => k += 1,
        }
    }
    out + &f(&line[pos..])
}
