//! 編輯畫面用的純函式：自訂條目與內建的衝突檢查，以及匯入檔案的合併規劃。

use std::collections::{HashMap, HashSet};

use super::matcher::{CustomWords, Side};
use super::{Entry, fold};

/// 一條自訂條目在編輯畫面上的狀態。
#[derive(Clone, Debug, PartialEq)]
pub enum RowIssue {
    Ok,
    /// 與內建共用一個詞但另一側不同；`other` 是內建那條不共用的那一側。
    BuiltinDiffers {
        other: String,
    },
    SameAsBuiltin,
    DuplicateSource,
    Incomplete,
}

/// 內建條目 fold 後的兩側，以及詞（任一側）→ 條目索引。
struct BuiltinIndex {
    folded: Vec<(String, String)>,
    by_word: HashMap<String, Vec<usize>>,
}

impl BuiltinIndex {
    fn new(builtin: &[Entry]) -> Self {
        let folded: Vec<(String, String)> = builtin
            .iter()
            .map(|b| (fold(&b.source), fold(&b.target)))
            .collect();
        let mut by_word: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, (source, target)) in folded.iter().enumerate() {
            by_word.entry(source.clone()).or_default().push(i);
            if target != source {
                by_word.entry(target.clone()).or_default().push(i);
            }
        }
        Self { folded, by_word }
    }

    /// 任一側含 `source` 或 `target` 的內建條目索引，依內建順序。
    fn sharing(&self, source: &str, target: &str) -> Vec<usize> {
        let mut found: Vec<usize> = [source, target]
            .iter()
            .filter_map(|w| self.by_word.get(*w))
            .flatten()
            .copied()
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }
}

/// 內建條目被自訂條目 `words` 蓋掉時，回傳內建那條不共用的一側。
fn unshared_side(
    words: &CustomWords,
    builtin: &Entry,
    folded: &(String, String),
) -> Option<String> {
    let source_hit = words.shadows(&folded.0, Side::Source);
    let target_hit = words.shadows(&folded.1, Side::Target);
    match (source_hit, target_hit) {
        (false, false) => None,
        (false, true) => Some(builtin.source.clone()),
        _ => Some(builtin.target.clone()),
    }
}

/// 條目是雙向的：兩個詞相同就算同一條，不論哪側。
fn same_words(a: (&str, &str), b: &(String, String)) -> bool {
    (a.0 == b.0 && a.1 == b.1) || (a.0 == b.1 && a.1 == b.0)
}

/// 每條自訂一個狀態，優先序為 Incomplete、DuplicateSource、SameAsBuiltin、BuiltinDiffers、Ok。
pub fn review(custom: &[Entry], builtin: &[Entry]) -> Vec<RowIssue> {
    let index = BuiltinIndex::new(builtin);
    let mut seen = HashSet::new();
    custom
        .iter()
        .map(|c| {
            if c.is_incomplete() {
                return RowIssue::Incomplete;
            }
            let (source, target) = (fold(&c.source), fold(&c.target));
            if !seen.insert(source.clone()) {
                return RowIssue::DuplicateSource;
            }
            let sharing = index.sharing(&source, &target);
            if sharing
                .iter()
                .any(|&i| same_words((&source, &target), &index.folded[i]))
            {
                return RowIssue::SameAsBuiltin;
            }
            let words = CustomWords::new(std::slice::from_ref(c));
            sharing
                .iter()
                .find_map(|&i| unshared_side(&words, &builtin[i], &index.folded[i]))
                .map_or(RowIssue::Ok, |other| RowIssue::BuiltinDiffers { other })
        })
        .collect()
}

/// 每條內建一個旗標，有詞被自訂覆蓋就是 true。
pub fn overridden(custom: &[Entry], builtin: &[Entry]) -> Vec<bool> {
    let words = CustomWords::new(custom);
    builtin
        .iter()
        .map(|b| {
            words.shadows(&fold(&b.source), Side::Source)
                || words.shadows(&fold(&b.target), Side::Target)
        })
        .collect()
}

/// 去掉與內建完全相同的自訂列（存檔用）。
pub fn without_builtin_duplicates(custom: &[Entry], builtin: &[Entry]) -> Vec<Entry> {
    review(custom, builtin)
        .into_iter()
        .zip(custom)
        .filter(|(issue, _)| *issue != RowIssue::SameAsBuiltin)
        .map(|(_, e)| e.clone())
        .collect()
}

/// 匯入檔案與現有清單來源相同、譯名不同的一筆。
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    /// 在現有清單裡的位置。
    pub index: usize,
    pub current: Entry,
    pub incoming: Entry,
}

/// 匯入檔案併入現有清單的規劃結果。
#[derive(Clone, Debug, PartialEq)]
pub struct MergePlan {
    pub added: Vec<Entry>,
    pub identical: usize,
    pub conflicts: Vec<Conflict>,
}

/// 以 fold 後的來源比對，規劃新增、相同與衝突各幾筆；檔案內來源重複（含大小寫差異）者只取第一筆。
pub fn plan_merge(current: &[Entry], incoming: &[Entry]) -> MergePlan {
    let mut plan = MergePlan {
        added: Vec::new(),
        identical: 0,
        conflicts: Vec::new(),
    };
    let mut seen = HashSet::new();
    for inc in incoming {
        let key = fold(&inc.source);
        if !seen.insert(key.clone()) {
            continue;
        }
        match current.iter().position(|c| fold(&c.source) == key) {
            None => plan.added.push(inc.clone()),
            Some(i) if fold(&current[i].target) == fold(&inc.target) => plan.identical += 1,
            Some(i) => plan.conflicts.push(Conflict {
                index: i,
                current: current[i].clone(),
                incoming: inc.clone(),
            }),
        }
    }
    plan
}

/// `take_incoming[i]` 為真就採用 `conflicts[i]` 的檔案譯名，`added` 依序接在後面。
pub fn apply_merge(current: &mut Vec<Entry>, plan: &MergePlan, take_incoming: &[bool]) {
    for (conflict, _) in plan
        .conflicts
        .iter()
        .zip(take_incoming)
        .filter(|(_, t)| **t)
    {
        current[conflict.index].target = conflict.incoming.target.clone();
    }
    current.extend(plan.added.iter().cloned());
}
