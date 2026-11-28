//! 啟動遊戲帳號視窗的純狀態：勾了哪些帳號、哪些在執行、等檢查或登入中、全選三態、實際會開的數量，以及這次啟動何時算全部完成。

use std::collections::HashMap;

use crate::launcher::Account;
use crate::launcher::orchestrate::{LaunchStatus, forget_exited};

/// 「全選」勾選框的三態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectAll {
    All,
    /// 部分勾選，顯示「−」。
    Some,
    None,
}

/// 從視窗啟動的帳號全部有了結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settled {
    /// 啟動成功或本來就在執行的數量。
    pub opened: usize,
    pub failed: usize,
}

/// 帳號清單、勾選與逐帳號的啟動進度。
#[derive(Default)]
pub struct LaunchList {
    accounts: Vec<Account>,
    checked: Vec<String>,
    running: Vec<String>,
    statuses: HashMap<String, LaunchStatus>,
    /// 從視窗啟動、還沒全部有結果的帳號 id；啟動中再啟動別的就併進來。
    batch: Vec<String>,
}

impl LaunchList {
    /// 以上次的勾選 `selection` 初始化；已不存在的帳號 id 丟掉。
    pub fn new(accounts: Vec<Account>, selection: &[String]) -> Self {
        let mut state = Self::default();
        state.set_accounts(accounts, selection);
        state
    }

    /// 換上新的帳號清單與勾選（設定存檔後）；執行中名單與現有帳號的啟動進度保留。
    pub fn set_accounts(&mut self, accounts: Vec<Account>, selection: &[String]) {
        self.checked = selection
            .iter()
            .filter(|id| accounts.iter().any(|a| &a.id == *id))
            .cloned()
            .collect();
        // 刪掉的帳號不留進度，免得同 id 卡在登入中；改過的帳號不留上次的結果
        let old = std::mem::take(&mut self.accounts);
        self.statuses.retain(|id, status| {
            accounts
                .iter()
                .any(|a| &a.id == id && (status.in_progress() || old.iter().any(|o| o == a)))
        });
        self.accounts = accounts;
    }

    /// 全部帳號，依存放順序。
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// 換上執行中的帳號名稱（不分大小寫比對）。
    pub fn set_running(&mut self, names: Vec<String>) {
        forget_exited(&mut self.statuses, &self.accounts, &self.running, &names);
        self.running = names;
    }

    /// 記下某帳號的啟動進度。
    pub fn set_status(&mut self, id: &str, status: LaunchStatus) {
        self.statuses.insert(id.to_owned(), status);
    }

    /// 某帳號目前的啟動進度。
    pub fn status(&self, id: &str) -> Option<&LaunchStatus> {
        self.statuses.get(id)
    }

    /// 帳號是否已在執行。
    pub fn is_running(&self, account: &Account) -> bool {
        self.running
            .iter()
            .any(|r| r.eq_ignore_ascii_case(&account.name))
    }

    /// 能不能勾：不在執行、也不在等檢查或登入中。
    pub fn checkable(&self, account: &Account) -> bool {
        !self.is_running(account) && !self.in_progress(&account.id)
    }

    fn in_progress(&self, id: &str) -> bool {
        self.status(id).is_some_and(LaunchStatus::in_progress)
    }

    /// 帳號是否勾選（含停用中仍保留的勾）。
    pub fn is_checked(&self, id: &str) -> bool {
        self.checked.iter().any(|c| c == id)
    }

    /// 勾選或取消；不能勾的帳號不動，回傳是否有變。
    pub fn set_checked(&mut self, id: &str, checked: bool) -> bool {
        let Some(account) = self.accounts.iter().find(|a| a.id == id) else {
            return false;
        };
        if !self.checkable(account) || self.is_checked(id) == checked {
            return false;
        }
        if checked {
            self.checked.push(id.to_owned());
        } else {
            self.checked.retain(|c| c != id);
        }
        true
    }

    /// 「全選」的狀態，只看能勾的帳號。
    pub fn select_all_state(&self) -> SelectAll {
        let (mut on, mut total) = (0, 0);
        for account in self.accounts.iter().filter(|a| self.checkable(a)) {
            total += 1;
            on += usize::from(self.is_checked(&account.id));
        }
        match on {
            0 => SelectAll::None,
            n if n == total => SelectAll::All,
            _ => SelectAll::Some,
        }
    }

    /// 按「全選」：全勾時全取消，否則把能勾的全勾上；執行中的不動。
    pub fn toggle_all(&mut self) {
        let check = self.select_all_state() != SelectAll::All;
        let ids: Vec<String> = self.accounts.iter().map(|a| a.id.clone()).collect();
        for id in ids {
            self.set_checked(&id, check);
        }
    }

    /// 這次按「啟動」會開的帳號 id（勾了且能勾），依存放順序。
    pub fn launchable(&self) -> Vec<String> {
        self.accounts
            .iter()
            .filter(|a| self.is_checked(&a.id) && self.checkable(a))
            .map(|a| a.id.clone())
            .collect()
    }

    /// 「啟動 N 個帳號」的 N。
    pub fn open_count(&self) -> usize {
        self.launchable().len()
    }

    /// 要記住的勾選，依存放順序。
    pub fn selection(&self) -> Vec<String> {
        self.accounts
            .iter()
            .filter(|a| self.is_checked(&a.id))
            .map(|a| a.id.clone())
            .collect()
    }

    /// 開始啟動勾選的帳號：回傳要開的 id，並先標成登入中，回報到來前再按一次也不會重開。
    pub fn begin_launch(&mut self) -> Vec<String> {
        let ids = self.launchable();
        self.mark_launching(&ids);
        ids
    }

    /// 單獨啟動一個帳號；執行中、等檢查或登入中就不開，回傳 `None`。
    pub fn begin_launch_one(&mut self, id: &str) -> Option<String> {
        let account = self.accounts.iter().find(|a| a.id == id)?;
        if !self.checkable(account) {
            return None;
        }
        let id = id.to_owned();
        self.mark_launching(std::slice::from_ref(&id));
        Some(id)
    }

    fn mark_launching(&mut self, ids: &[String]) {
        for id in ids {
            self.set_status(id, LaunchStatus::LoggingIn);
            if !self.batch.contains(id) {
                self.batch.push(id.clone());
            }
        }
    }

    /// 從視窗啟動的帳號都有結果了就結算並清掉；還有等檢查或登入中的、或沒在啟動時回 `None`。
    ///
    /// 已經沒有進度的帳號（刪掉了，或啟動後遊戲又關了）不算失敗。
    pub fn settle(&mut self) -> Option<Settled> {
        if self.batch.is_empty() || self.batch.iter().any(|id| self.in_progress(id)) {
            return None;
        }
        let batch = std::mem::take(&mut self.batch);
        let failed = batch
            .iter()
            .filter(|id| matches!(self.status(id), Some(LaunchStatus::Failed(_))))
            .count();
        Some(Settled {
            opened: batch.len() - failed,
            failed,
        })
    }

    /// 視窗關了：不再等這次啟動的結果。
    pub fn forget_batch(&mut self) {
        self.batch.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::AccountKind;
    use crate::launcher::install::InstallSource;
    use crate::launcher::login::LoginError;
    use crate::launcher::orchestrate::Failure;

    fn account(id: &str, name: &str) -> Account {
        Account {
            id: id.into(),
            name: name.into(),
            kind: AccountKind::Official,
            password_dpapi: String::new(),
        }
    }

    fn three() -> Vec<Account> {
        vec![
            account("a", "Alice"),
            account("b", "Bob"),
            account("c", "Carol"),
        ]
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unknown_selection_ids_are_dropped() {
        let state = LaunchList::new(three(), &ids(&["b", "gone", "a"]));
        assert_eq!(state.selection(), ids(&["a", "b"]));
    }

    #[test]
    fn open_count_excludes_running() {
        let mut state = LaunchList::new(three(), &ids(&["a", "b", "c"]));
        state.set_running(ids(&["bob"]));
        assert_eq!(state.open_count(), 2);
        assert_eq!(state.launchable(), ids(&["a", "c"]));
        assert_eq!(
            state.selection(),
            ids(&["a", "b", "c"]),
            "running keeps its check"
        );
    }

    #[test]
    fn select_all_only_checks_non_running() {
        let mut state = LaunchList::new(three(), &[]);
        state.set_running(ids(&["CAROL"]));
        assert_eq!(state.select_all_state(), SelectAll::None);
        state.toggle_all();
        assert_eq!(state.selection(), ids(&["a", "b"]));
        assert_eq!(state.select_all_state(), SelectAll::All);
        state.set_checked("a", false);
        assert_eq!(state.select_all_state(), SelectAll::Some);
        state.toggle_all();
        assert_eq!(state.select_all_state(), SelectAll::All);
        state.toggle_all();
        assert_eq!(state.selection(), Vec::<String>::new());
    }

    #[test]
    fn select_all_leaves_a_checked_running_account_checked() {
        let mut state = LaunchList::new(three(), &ids(&["c"]));
        state.set_running(ids(&["Carol"]));
        state.toggle_all();
        state.toggle_all();
        assert_eq!(state.selection(), ids(&["c"]));
    }

    #[test]
    fn all_running_disables_open() {
        let mut state = LaunchList::new(three(), &ids(&["a", "b", "c"]));
        state.set_running(ids(&["alice", "bob", "carol"]));
        assert_eq!(state.open_count(), 0);
        assert_eq!(state.select_all_state(), SelectAll::None);
        assert!(
            !state.set_checked("a", false),
            "running accounts can't be toggled"
        );
    }

    #[test]
    fn launching_marks_accounts_busy_so_a_second_press_opens_nothing() {
        let mut state = LaunchList::new(three(), &ids(&["a", "b"]));
        assert_eq!(state.begin_launch(), ids(&["a", "b"]));
        assert_eq!(state.status("a"), Some(&LaunchStatus::LoggingIn));
        assert_eq!(state.open_count(), 0);
        assert!(state.begin_launch().is_empty());
        state.set_status("a", LaunchStatus::Opened);
        assert_eq!(state.launchable(), ids(&["a"]));
    }

    #[test]
    fn a_deleted_account_does_not_keep_its_progress() {
        let mut state = LaunchList::new(three(), &ids(&["a"]));
        state.begin_launch();
        let rest: Vec<Account> = three().into_iter().filter(|a| a.id != "a").collect();
        state.set_accounts(rest, &[]);
        assert_eq!(state.status("a"), None);
    }

    #[test]
    fn editing_an_account_drops_its_last_result() {
        let mut state = LaunchList::new(three(), &[]);
        state.set_status("a", LaunchStatus::Opened);
        state.set_status("b", LaunchStatus::Opened);
        state.set_status("c", LaunchStatus::LoggingIn);
        let mut edited = three();
        edited[0].password_dpapi = "new".into();
        edited[2].password_dpapi = "new".into();
        state.set_accounts(edited, &[]);
        assert_eq!(state.status("a"), None);
        assert_eq!(state.status("b"), Some(&LaunchStatus::Opened));
        assert_eq!(state.status("c"), Some(&LaunchStatus::LoggingIn));
    }

    #[test]
    fn opened_is_cleared_once_the_game_exits() {
        let mut state = LaunchList::new(three(), &[]);
        let name = three()[0].name.clone();
        state.set_status("a", LaunchStatus::Opened);
        state.set_running(vec![name]);
        assert_eq!(state.status("a"), Some(&LaunchStatus::Opened));
        state.set_running(Vec::new());
        assert_eq!(state.status("a"), None);
    }

    #[test]
    fn a_new_launch_clears_the_old_result() {
        let mut state = LaunchList::new(three(), &ids(&["a"]));
        let raw = "Bad password".to_owned();
        state.set_status(
            "a",
            LaunchStatus::Failed(Failure::Login(LoginError::Rejected { raw })),
        );
        state.begin_launch();
        assert_eq!(state.status("a"), Some(&LaunchStatus::LoggingIn));
    }

    fn failed() -> LaunchStatus {
        let raw = "Bad password".to_owned();
        LaunchStatus::Failed(Failure::Login(LoginError::Rejected { raw }))
    }

    #[test]
    fn launching_one_account_marks_only_it_busy() {
        let mut state = LaunchList::new(three(), &[]);
        state.set_running(ids(&["bob"]));
        assert_eq!(state.begin_launch_one("a"), Some("a".into()));
        assert_eq!(state.status("a"), Some(&LaunchStatus::LoggingIn));
        assert_eq!(state.status("c"), None);
        assert_eq!(state.begin_launch_one("a"), None, "already logging in");
        assert_eq!(state.begin_launch_one("b"), None, "running");
        assert_eq!(state.begin_launch_one("gone"), None);
    }

    #[test]
    fn a_launch_settles_once_every_account_has_a_result() {
        let mut state = LaunchList::new(three(), &ids(&["a", "b"]));
        assert_eq!(state.settle(), None, "nothing launched");
        state.begin_launch();
        state.set_status("a", LaunchStatus::Opened);
        assert_eq!(state.settle(), None, "b is still logging in");
        state.set_status("b", LaunchStatus::SkippedRunning);
        let all_ok = Settled {
            opened: 2,
            failed: 0,
        };
        assert_eq!(state.settle(), Some(all_ok));
        assert_eq!(state.settle(), None, "settled only once");
    }

    #[test]
    fn a_failure_is_counted_when_the_launch_settles() {
        let mut state = LaunchList::new(three(), &ids(&["a", "b"]));
        state.begin_launch();
        state.set_status("a", failed());
        state.set_status("b", LaunchStatus::Opened);
        let settled = Settled {
            opened: 1,
            failed: 1,
        };
        assert_eq!(state.settle(), Some(settled));
    }

    #[test]
    fn a_second_launch_joins_the_one_in_progress() {
        let mut state = LaunchList::new(three(), &ids(&["a"]));
        state.begin_launch();
        state.begin_launch_one("c");
        state.set_status("a", LaunchStatus::Opened);
        assert_eq!(state.settle(), None, "c is still logging in");
        state.set_status("c", LaunchStatus::Opened);
        assert_eq!(state.settle().map(|s| s.opened), Some(2));
    }

    #[test]
    fn results_from_elsewhere_do_not_settle_anything() {
        let mut state = LaunchList::new(three(), &[]);
        state.set_status("a", LaunchStatus::LoggingIn);
        state.set_status("a", LaunchStatus::Opened);
        assert_eq!(state.settle(), None);
    }

    #[test]
    fn an_account_waiting_for_the_check_is_still_launching() {
        let mut state = LaunchList::new(three(), &ids(&["a"]));
        state.begin_launch();
        state.set_status("a", LaunchStatus::Waiting(InstallSource::Official));
        assert!(!state.checkable(&three()[0]));
        assert_eq!(state.settle(), None);
        state.set_status("a", LaunchStatus::Failed(Failure::Patch));
        let settled = Settled {
            opened: 0,
            failed: 1,
        };
        assert_eq!(
            state.settle(),
            Some(settled),
            "a blocked check keeps the window"
        );
    }

    #[test]
    fn forgetting_the_batch_stops_waiting_for_it() {
        let mut state = LaunchList::new(three(), &ids(&["a"]));
        state.begin_launch();
        state.forget_batch();
        state.set_status("a", LaunchStatus::Opened);
        assert_eq!(state.settle(), None);
    }
}
