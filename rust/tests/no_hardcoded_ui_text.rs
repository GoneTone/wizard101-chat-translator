//! 防迴歸：UI 與執行期模組的字串字面量不得出現中日韓文字，忘了走 i18n 就會直接失敗。
//!
//! 刻意不掃給 LLM 的資料與比對規則（提示詞、範例、後處理的正規式）；`#[cfg(test)]` 之後的測試程式碼也不掃。

use std::fs;
use std::path::{Path, PathBuf};

/// 例外：語言選單與翻譯目標語言清單一律顯示自稱，任何介面語言下都不翻譯；全形空白是標題列的間隔。
const ALLOWED: &[&str] = &[
    "繁體中文（台灣）",
    "简体中文（中国）",
    "日本語",
    "한국어",
    "\u{3000}",
];

/// 內容本來就是中文的模組：提示詞、範例示範、後處理用的全形標點正規式。
const EXEMPT_FILES: &[&str] = &["prompts.rs", "examples.rs", "postprocess.rs"];

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3000}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}' | '\u{FF00}'..='\u{FFEF}')
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs")
            && path
                .file_name()
                .is_some_and(|n| !EXEMPT_FILES.contains(&n.to_string_lossy().as_ref()))
        {
            out.push(path);
        }
    }
}

/// 掃出非註解的字串字面量（含 raw string）：回傳（行號，內容）。
fn string_literals(code: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = code.chars().collect();
    let (mut found, mut i, mut line) = (Vec::new(), 0, 1);
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\n' => line += 1,
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                let mut depth = 0;
                while i < chars.len() {
                    if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 1;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 1;
                        if depth == 0 {
                            break;
                        }
                    } else if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            '\'' => {
                // 字元字面量跳過；生命週期（'a）後面沒有收尾的引號
                let close = if chars.get(i + 1) == Some(&'\\') {
                    i + 3
                } else {
                    i + 2
                };
                if chars.get(close) == Some(&'\'') {
                    i = close;
                } else if chars.get(i + 1) == Some(&'\\') {
                    let end = (i + 2..chars.len())
                        .find(|&k| chars[k] == '\'')
                        .unwrap_or(i);
                    i = end;
                }
            }
            'r' | '"' => {
                let (start, hashes) = if c == 'r' {
                    let hashes = chars[i + 1..].iter().take_while(|&&h| h == '#').count();
                    if chars.get(i + 1 + hashes) != Some(&'"')
                        || (i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_'))
                    {
                        i += 1;
                        continue;
                    }
                    (i + 2 + hashes, hashes)
                } else {
                    (i + 1, 0)
                };
                let raw = c == 'r';
                let (mut k, first_line) = (start, line);
                let mut text = String::new();
                while k < chars.len() {
                    if chars[k] == '\n' {
                        line += 1;
                    }
                    if !raw && chars[k] == '\\' {
                        k += 2;
                        continue;
                    }
                    if chars[k] == '"' && (0..hashes).all(|h| chars.get(k + 1 + h) == Some(&'#')) {
                        break;
                    }
                    text.push(chars[k]);
                    k += 1;
                }
                found.push((first_line, text));
                i = k + hashes;
            }
            _ => {}
        }
        i += 1;
    }
    found
}

#[test]
fn no_hardcoded_cjk_string_literals() {
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut offenders = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path).unwrap();
        let code = text.split("#[cfg(test)]").next().unwrap();
        for (line, literal) in string_literals(code) {
            if literal.chars().any(is_cjk) && !ALLOWED.contains(&literal.as_str()) {
                offenders.push(format!("{}:{line}: {literal}", path.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "hard-coded UI text, route it through i18n::t():\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_scanner_sees_literals_but_not_comments_or_chars() {
    let code = "let a = \"中\"; // 註解 \"漢\"\nlet b = '中'; let c = r#\"語\"#; /* \"字\" */ let d = \"x\\\"y\";";
    let seen: Vec<String> = string_literals(code).into_iter().map(|(_, s)| s).collect();
    assert_eq!(seen, ["中", "語", "xy"]);
}
