//! 譯文後處理與品質判定：去掉 `<think>` 區塊、清除模型自行補上的括號原文、還原發送者，
//! 判斷譯文有沒有落回拉丁文字。純字串函式，不碰網路。
//!
//! 字元類別刻意對齊 Python `re` 的 Unicode 語意：`\w` 是字母／數字（不含組合記號）加底線，
//! 空白含 `\x1c`–`\x1f`，故不直接用 `regex` 預設的 `\w`／`\s`。

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::py::pystr::{self, casefold, lstrip, rstrip, strip};
use crate::reader::markup::SENDER_PREFIX;

pub(super) const SPACE: &str = r"[\s\x1c-\x1f]";
const WORD: &str = r"\p{L}\p{N}_";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|e| panic!("bad regex {pattern}: {e}"))
}

// Python 的 IGNORECASE 讓 i 也吃 ı 與 İ（Rust 的簡單摺疊不會）
static THINK_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"(?is)<th[iıİ]nk>.*?</th[iıİ]nk>{SPACE}*")));
// 只到最後一個孤立 `</think>` 為止的推理（自架推理模型常省略開頭標籤）
static THINK_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"(?is)^.*</th[iıİ]nk>{SPACE}*")));
static SENDER_RE: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"^{SENDER_PREFIX}{SPACE}*")));
static LEADING_BRACKETS: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"^(?:{SENDER_PREFIX}{SPACE}*)+")));
static SENDER_ANY: LazyLock<Regex> = LazyLock::new(|| re(SENDER_PREFIX));
static PARENTHESISED: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"{SPACE}*[（(]([^（）()\n]+)[)）]")));
static LATIN: LazyLock<Regex> = LazyLock::new(|| re("[A-Za-z]"));
// 非拉丁字母的字（漢字、假名、諺文、西里爾等）；帶重音的拉丁字母（é、ñ）不算
static NON_LATIN_LETTER: LazyLock<Regex> =
    LazyLock::new(|| re(r"[[\p{L}\p{N}]--[\p{Nd}_\x00-\u{24F}\u{1E00}-\u{1EFF}]]"));
// 一段連續的拉丁文字（字母起頭，可含數字、詞內標點與空白）
static LATIN_RUN: LazyLock<Regex> = LazyLock::new(|| re(r"[A-Za-z][A-Za-z0-9 .,'’\-]*"));
// 任何文字的字母；空白、數字與標點不算
static LETTER: LazyLock<Regex> = LazyLock::new(|| re(r"[[\p{L}\p{N}]--[\p{Nd}_]]"));
// 略過修飾字母：`ʻŌlelo` 的 ʻokina 不決定文字
static FIRST_LETTER: LazyLock<Regex> = LazyLock::new(|| re(r"[\p{L}--\p{Lm}]"));
// 拉丁單字（含帶重音字母）；只取拉丁字母，原文沒空格時（`我要Amy的帽子`）才切得出 `Amy`
static LATIN_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"[\p{Latin}0-9]+"));
static NON_WORD: LazyLock<Regex> = LazyLock::new(|| re(&format!(r"[^{WORD}]+|_+")));
static NUMBERED_LINE: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"^{SPACE}*(\p{{Nd}}+){SPACE}*[.．、)]{SPACE}*(.*)$"
    ))
});
static DECIMAL: LazyLock<Regex> = LazyLock::new(|| re(r"^\p{Nd}$"));

/// 移除回應中的 `<think>…</think>` 推理區塊與孤立 `</think>` 之前的推理（無論是否啟用思考都套用）。
pub fn strip_think(text: &str) -> String {
    let text = THINK_BLOCK.replace_all(text, "");
    THINK_PREFIX.replace(&text, "").into_owned()
}

/// 只留字母與數字並忽略大小寫：模型照抄原文時常換掉撇號、連字號或空白。
fn alnum(text: &str) -> String {
    casefold(&NON_WORD.replace_all(text, ""))
}

fn remove_sender(text: &str) -> Cow<'_, str> {
    SENDER_RE.replace(text, "")
}

/// 訊息本文：去掉開頭的發送者前綴，沒有前綴回整段。
pub fn message_body(text: &str) -> &str {
    SENDER_RE.find(text).map_or(text, |m| &text[m.end()..])
}

/// 移除譯文裡不該有的括號原文：原文沒出現過的外文，以及只是重複前面譯名的。
///
/// 「外文」以括號外的譯文用什麼文字判斷，不看目標語言名稱；`[發送者]` 不算原文。
pub fn tidy_parentheses(source: &str, translated: &str) -> String {
    let body = PARENTHESISED
        .replace_all(&remove_sender(translated), "")
        .into_owned();
    let latin_translation = !NON_LATIN_LETTER.is_match(&body);
    let haystack = alnum(&remove_sender(source));
    let body_has_no_letters = !LETTER.is_match(&body);

    let foreign = |content: &str| {
        if body_has_no_letters {
            return false; // 括號外沒有字（整行都是括號）就無從判斷，一律保留
        }
        if latin_translation {
            return NON_LATIN_LETTER.is_match(content);
        }
        LATIN.is_match(content) && !NON_LATIN_LETTER.is_match(content)
    };

    PARENTHESISED
        .replace_all(translated, |caps: &Captures| {
            let raw = &caps[1];
            let content = alnum(raw);
            if foreign(raw) && !haystack.contains(&content) {
                return String::new();
            }
            // 只認有字母的重複：玩家自己打的「lvl 50 (50)」不是模型的回聲
            let start = caps.get(0).map_or(0, |m| m.start());
            if content.chars().count() > 1
                && LETTER.is_match(&content)
                && alnum(&translated[..start]).ends_with(&content)
            {
                return String::new();
            }
            caps[0].to_string()
        })
        .into_owned()
}

fn leading_bracket_count(text: &str) -> usize {
    LEADING_BRACKETS
        .find(text)
        .map_or(0, |m| SENDER_ANY.find_iter(m.as_str()).count())
}

/// 行首的 `[發送者]`（不含其後空白）；沒有回 `None`。
pub fn sender_of(text: &str) -> Option<String> {
    SENDER_RE.find(text).map(|m| rstrip(m.as_str()).to_string())
}

/// 譯文開頭的 `[發送者]` 一律換回原文那一個；模型丟掉時補回去。
///
/// 譯文已以同一個發送者開頭就不補；否則以開頭連續的中括號組數判斷模型是改了還是丟了發送者，
/// 內容本身以 `[WTS]` 開頭時才不會誤換。
pub fn restore_sender(source: &str, translated: &str) -> String {
    let Some(prefix) = sender_of(source) else {
        return translated.to_string();
    };
    if sender_of(translated).as_ref() != Some(&prefix)
        && leading_bracket_count(translated) < leading_bracket_count(source)
    {
        return format!("{prefix} {}", lstrip(translated));
    }
    format!("{prefix} {}", SENDER_RE.replace(translated, ""))
}

/// 這個語言是否以拉丁字母書寫：看語言名稱第一個字母的文字，`中文 (Chinese)` 不算拉丁。
pub fn uses_latin_script(language: &str) -> bool {
    FIRST_LETTER
        .find(language)
        .is_some_and(|letter| !NON_LATIN_LETTER.is_match(letter.as_str()))
}

/// 譯文是否出現了不該有的拉丁文字 —— 模型改用英文名，或整句沒翻。
///
/// 兩條規則：整段都是拉丁、沒有一個目標語言的字；或拉丁片段裡有原文沒出現過的字。
/// 已知盲點：目標語言本身用拉丁字母時一律回 `false`。
pub fn has_stray_latin(source: &str, translated: &str, target_language: &str) -> bool {
    if uses_latin_script(target_language) {
        return false;
    }
    let body = remove_sender(translated);
    if !LATIN.is_match(&body) {
        return false;
    }
    if !LETTER.is_match(&LATIN_RUN.replace_all(&body, "")) {
        return true;
    }
    let words = |text: &str| {
        LATIN_WORD
            .find_iter(text)
            .map(|word| casefold(word.as_str()))
            .collect::<Vec<_>>()
    };
    let source_words: HashSet<String> = words(&remove_sender(source)).into_iter().collect();
    // 逐字比對、兩邊同樣切字：模型常改掉字間的標點或連接詞（`Kai and Bob` → `Kai, Bob`）
    words(&body)
        .into_iter()
        .filter(|word| LATIN.is_match(word))
        .any(|word| !source_words.contains(&word))
}

/// 拆成去掉前後空白的非空白行。
pub fn nonblank_lines(text: &str) -> Vec<String> {
    pystr::splitlines(text)
        .into_iter()
        .map(strip)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// 把多行文字拆成非空白行並加上 `1. `、`2. ` 編號，回傳（原始行, 編號後的文字）。
pub fn number_lines(text: &str) -> (Vec<String>, String) {
    let lines = nonblank_lines(text);
    let numbered = lines
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{}. {line}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    (lines, numbered)
}

/// Unicode 十進位數字（Nd）的數值；Nd 皆為連續 10 個一組，從組首起算。
fn decimal_value(c: char) -> u32 {
    let is_nd = |c: char| DECIMAL.is_match(c.encode_utf8(&mut [0; 4]));
    let mut first = c as u32;
    while first > 0 && char::from_u32(first - 1).is_some_and(is_nd) {
        first -= 1;
    }
    (c as u32 - first) % 10
}

fn parse_number(digits: &str) -> u128 {
    digits.chars().fold(0u128, |n, c| {
        n.saturating_mul(10)
            .saturating_add(u128::from(decimal_value(c)))
    })
}

/// 模型回傳的編號行 → `{編號: 內容}`；沒帶編號的行不算。
pub fn numbered_entries(output: &str) -> BTreeMap<u128, String> {
    let mut entries = BTreeMap::new();
    for line in pystr::splitlines(output) {
        if let Some(caps) = NUMBERED_LINE.captures(line) {
            entries.insert(parse_number(&caps[1]), strip(&caps[2]).to_string());
        }
    }
    entries
}

/// 把模型回傳的編號譯文依編號對回原始行；缺的編號以原文補上（寧可留原文也不漏行）。
///
/// 模型完全沒帶編號但行數剛好相同時視為逐行對應；其餘情況整段原樣回傳。
pub fn unnumber_lines(output: &str, originals: &[String]) -> String {
    let numbered = numbered_entries(output);
    if !numbered.is_empty() {
        return originals
            .iter()
            .zip(1u128..)
            .map(|(original, i)| match numbered.get(&i) {
                Some(entry) if !entry.is_empty() => entry.as_str(),
                _ => original.as_str(),
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    let plain = nonblank_lines(output);
    if plain.len() == originals.len() {
        return plain.join("\n");
    }
    strip(output).to_string()
}
