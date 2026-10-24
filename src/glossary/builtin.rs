//! 內建術語表：`src/glossary/builtin/<語系代碼>.json` 編進 exe，key 是英文（遊戲語言）術語，value 是該語系譯名；
//! 建立者寫在 [`CREATORS_KEY`](super::CREATORS_KEY)。

use std::sync::LazyLock;

use super::{Entry, GlossaryFile, ParseError, fold, parse_file};
use crate::logs::log;

/// 編進 exe 的內建檔，依代碼字母序；清單由 `build.rs` 掃描 `builtin/` 資料夾產生，新增語系只要放檔案。
const BUILTINS: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/glossary_builtins.rs"));

/// 測試用的內建術語表代碼；不隨正式版發佈，讓設定頁測試畫得出內建區塊。
#[cfg(any(test, feature = "test-support"))]
pub const TEST_BUILTIN: &str = "zz-TEST";
#[cfg(any(test, feature = "test-support"))]
const TEST_BUILTINS: &[(&str, &str)] = &[(
    TEST_BUILTIN,
    r#"{"glossary.creators": "[Tester](https://example.com)", "Ravenwood": "RW", "Krokotopia": "KT", "Malistaire": "MS"}"#,
)];
#[cfg(not(any(test, feature = "test-support")))]
const TEST_BUILTINS: &[(&str, &str)] = &[];

static LOADED: LazyLock<Vec<(&'static str, GlossaryFile)>> = LazyLock::new(|| {
    let mut loaded = Vec::new();
    for (code, raw) in BUILTINS.iter().chain(TEST_BUILTINS) {
        match filter_untranslated(raw) {
            Ok((file, dropped)) => {
                log(format!(
                    "[glossary] builtin {code}: {} entries ({dropped} untranslated dropped)",
                    file.entries.len()
                ));
                if !file.entries.is_empty() {
                    loaded.push((*code, file));
                }
            }
            Err(err) => log(format!("[glossary] builtin {code} parse failed: {err}")),
        }
    }
    loaded
});

/// 有內建術語表的語系代碼（濾完非空），依代碼排序。
pub fn builtin_codes() -> Vec<&'static str> {
    let mut codes: Vec<_> = LOADED.iter().map(|(code, _)| *code).collect();
    codes.sort_unstable();
    codes
}

/// 取某語系的內建條目；不存在或濾完為空回 `None`。
pub fn builtin_entries(code: &str) -> Option<Vec<Entry>> {
    find(code).map(|file| file.entries.clone())
}

/// 某語系內建術語表的建立者；沒有這份或沒寫回空字串。
pub fn builtin_creators(code: &str) -> String {
    find(code).map_or_else(String::new, |file| file.creators.clone())
}

fn find(code: &str) -> Option<&'static GlossaryFile> {
    LOADED
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, file)| file)
}

/// 解析內建檔並濾掉未翻譯（原文與譯名相同）的條目；解析失敗記 log 並回空。
pub fn load(raw: &str) -> Vec<Entry> {
    filter_untranslated(raw).map_or_else(
        |err| {
            log(format!("[glossary] builtin parse failed: {err}"));
            Vec::new()
        },
        |(file, _)| file.entries,
    )
}

fn filter_untranslated(raw: &str) -> Result<(GlossaryFile, usize), ParseError> {
    let mut file = parse_file(raw)?;
    let total = file.entries.len();
    file.entries.retain(|e| fold(&e.source) != fold(&e.target));
    let dropped = total - file.entries.len();
    Ok((file, dropped))
}
