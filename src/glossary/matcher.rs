//! 術語表比對：在原文中找出雙向命中的術語條目。

use std::collections::{HashMap, HashSet};

use aho_corasick::{AhoCorasick, MatchKind};

use super::{Entry, fold};
use crate::logs::log;

/// 比對字串在條目的哪一側。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Source,
    Target,
}

/// 自訂條目的詞集合，用來判斷內建條目是否被「以詞為單位」蓋掉。
#[derive(Debug)]
pub(crate) struct CustomWords {
    sources: HashSet<String>,
    targets: HashSet<String>,
}

impl CustomWords {
    pub(crate) fn new(custom: &[Entry]) -> Self {
        Self {
            sources: custom.iter().map(|e| fold(&e.source)).collect(),
            targets: custom.iter().map(|e| fold(&e.target)).collect(),
        }
    }

    /// `word`（已 fold）在內建條目的 `side` 側時，是否被自訂條目蓋掉。
    pub(crate) fn shadows(&self, word: &str, side: Side) -> bool {
        self.sources.contains(word) || (side == Side::Source && self.targets.contains(word))
    }
}

/// 某個比對字串對應的條目索引與所在側。
#[derive(Debug)]
struct Owner {
    entry: usize,
    side: Side,
}

/// 自訂與內建術語表合併後的比對器。
#[derive(Debug)]
pub struct Glossary {
    /// 條目與「是否為自訂」，自訂在前。
    entries: Vec<(Entry, bool)>,
    patterns: Vec<String>,
    owners: Vec<Vec<Owner>>,
    custom_words: CustomWords,
    automaton: Option<AhoCorasick>,
}

impl Glossary {
    /// 建立比對器；參數順序為先自訂、後內建，同詞時自訂優先。
    pub fn new(custom: Vec<Entry>, builtin: Vec<Entry>) -> Self {
        let custom_words = CustomWords::new(&custom);
        let entries: Vec<(Entry, bool)> = custom
            .into_iter()
            .map(|e| (e, true))
            .chain(builtin.into_iter().map(|e| (e, false)))
            .collect();
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut patterns: Vec<String> = Vec::new();
        let mut owners: Vec<Vec<Owner>> = Vec::new();
        for (i, (entry, _)) in entries.iter().enumerate() {
            let (src, tgt) = (fold(&entry.source), fold(&entry.target));
            let mut words = vec![(src.clone(), Side::Source)];
            if tgt != src {
                words.push((tgt, Side::Target));
            }
            for (word, side) in words.into_iter().filter(|(w, _)| !w.is_empty()) {
                let p = *index.entry(word.clone()).or_insert_with(|| {
                    patterns.push(word);
                    owners.push(Vec::new());
                    patterns.len() - 1
                });
                owners[p].push(Owner { entry: i, side });
            }
        }
        let automaton = if patterns.is_empty() {
            None
        } else {
            AhoCorasick::builder()
                .match_kind(MatchKind::Standard)
                .build(&patterns)
                .inspect_err(|err| {
                    log(format!(
                        "[glossary] matcher build failed: {err}; glossary disabled"
                    ));
                })
                .ok()
        };
        Self {
            entries,
            patterns,
            owners,
            custom_words,
            automaton,
        }
    }

    /// 沒有任何條目時為 true。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 原文中命中的條目，依首次出現位置排序並去重。
    pub fn hits(&self, text: &str) -> Vec<Entry> {
        let Some(ac) = &self.automaton else {
            return Vec::new();
        };
        // 偏移量只對 fold 後的字串有意義，不可拿去索引原文。
        let text = fold(text);
        let mut spans: Vec<(usize, usize, usize)> = ac
            .find_overlapping_iter(&text)
            .filter(|m| boundaries_ok(&text, m.start(), m.end()))
            .map(|m| (m.start(), m.end(), m.pattern().as_usize()))
            .collect();
        spans.sort_by_key(|&(start, end, _)| (start, std::cmp::Reverse(end - start)));

        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let mut covered_to = 0;
        for (start, end, p) in spans {
            if start < covered_to {
                continue;
            }
            covered_to = end;
            for owner in &self.owners[p] {
                let (entry, is_custom) = &self.entries[owner.entry];
                let shadowed =
                    !is_custom && self.custom_words.shadows(&self.patterns[p], owner.side);
                if !shadowed && seen.insert(owner.entry) {
                    out.push(entry.clone());
                }
            }
        }
        out
    }
}

/// 命中條目的穩定鍵（與順序無關），空集合為空字串。
pub fn hits_key(hits: &[Entry]) -> String {
    let mut lines: Vec<String> = hits.iter().map(Entry::to_string).collect();
    lines.sort();
    lines.join("\n")
}

/// 以空白分詞的文字才需要單字邊界。
fn needs_boundary(c: char) -> bool {
    c.is_alphanumeric() && !unspaced(c)
}

fn unspaced(c: char) -> bool {
    matches!(u32::from(c),
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF
        | 0x3040..=0x309F | 0x30A0..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F
        | 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF
        | 0x0E00..=0x0E7F | 0x0E80..=0x0EFF | 0x1780..=0x17FF | 0x1000..=0x109F)
}

fn boundaries_ok(text: &str, start: usize, end: usize) -> bool {
    let clear = |term: Option<char>, neighbour: Option<char>| {
        !term.is_some_and(needs_boundary) || !neighbour.is_some_and(needs_boundary)
    };
    clear(
        text[start..].chars().next(),
        text[..start].chars().next_back(),
    ) && clear(text[..end].chars().next_back(), text[end..].chars().next())
}
