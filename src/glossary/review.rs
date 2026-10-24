//! 編輯畫面用的純函式：自訂條目與內建的衝突檢查，以及匯入檔案的合併規劃。

use std::collections::HashSet;

use super::matcher::{CustomWords, Side};
use super::{Entry, fold};
use crate::py::pystr;

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

/// 內建條目被單一自訂條目蓋掉時，回傳內建那條不共用的一側。
fn unshared_side(custom: &Entry, builtin: &Entry) -> Option<String> {
    let words = CustomWords::new(std::slice::from_ref(custom));
    let source_hit = words.shadows(&fold(&builtin.source), Side::Source);
    let target_hit = words.shadows(&fold(&builtin.target), Side::Target);
    match (source_hit, target_hit) {
        (false, false) => None,
        (false, true) => Some(builtin.source.clone()),
        _ => Some(builtin.target.clone()),
    }
}

fn same_entry(a: &Entry, b: &Entry) -> bool {
    fold(&a.source) == fold(&b.source) && fold(&a.target) == fold(&b.target)
}

/// 每條自訂一個狀態，優先序為 Incomplete、DuplicateSource、SameAsBuiltin、BuiltinDiffers、Ok。
pub fn review(custom: &[Entry], builtin: &[Entry]) -> Vec<RowIssue> {
    let mut seen = HashSet::new();
    custom
        .iter()
        .map(|c| {
            if pystr::strip(&c.source).is_empty() || pystr::strip(&c.target).is_empty() {
                RowIssue::Incomplete
            } else if !seen.insert(fold(&c.source)) {
                RowIssue::DuplicateSource
            } else if builtin.iter().any(|b| same_entry(c, b)) {
                RowIssue::SameAsBuiltin
            } else if let Some(other) = builtin.iter().find_map(|b| unshared_side(c, b)) {
                RowIssue::BuiltinDiffers { other }
            } else {
                RowIssue::Ok
            }
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

/// 有重複來源或未填完的列時不可存檔。
pub fn blocks_saving(issues: &[RowIssue]) -> bool {
    issues
        .iter()
        .any(|i| matches!(i, RowIssue::DuplicateSource | RowIssue::Incomplete))
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

/// 以 fold 後的來源比對，規劃新增、相同與衝突各幾筆。
pub fn plan_merge(current: &[Entry], incoming: &[Entry]) -> MergePlan {
    let mut plan = MergePlan {
        added: Vec::new(),
        identical: 0,
        conflicts: Vec::new(),
    };
    for inc in incoming {
        let key = fold(&inc.source);
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
