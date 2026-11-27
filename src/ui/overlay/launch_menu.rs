//! 開啟遊戲選單的純狀態：勾了哪些帳號、哪些在執行或登入中、全選三態、實際會開的數量與選單擺放位置。

use std::collections::HashMap;

use eframe::egui::{Pos2, Rect, Vec2};

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

/// 選單貼著誰出現。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    /// 內容區中央的大按鈕（範圍）：優先貼在它下方。
    Button(Rect),
    /// 標題列的 ▶：貼在標題列下方、靠右。
    TitleBar,
}

/// 帳號清單、勾選與逐帳號的開啟進度。
#[derive(Default)]
pub struct MenuState {
    accounts: Vec<Account>,
    checked: Vec<String>,
    running: Vec<String>,
    statuses: HashMap<String, LaunchStatus>,
}

impl MenuState {
    /// 以上次的勾選 `selection` 初始化；已不存在的帳號 id 丟掉。
    pub fn new(accounts: Vec<Account>, selection: &[String]) -> Self {
        let mut state = Self::default();
        state.set_accounts(accounts, selection);
        state
    }

    /// 換上新的帳號清單與勾選（設定存檔後）；執行中名單與現有帳號的開啟進度保留。
    pub fn set_accounts(&mut self, accounts: Vec<Account>, selection: &[String]) {
        self.checked = selection
            .iter()
            .filter(|id| accounts.iter().any(|a| &a.id == *id))
            .cloned()
            .collect();
        // 刪掉的帳號不留進度，免得同 id 卡在登入中；改過的帳號不留上次的結果
        let old = std::mem::take(&mut self.accounts);
        self.statuses.retain(|id, status| {
            accounts.iter().any(|a| {
                &a.id == id && (*status == LaunchStatus::LoggingIn || old.iter().any(|o| o == a))
            })
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

    /// 記下某帳號的開啟進度。
    pub fn set_status(&mut self, id: &str, status: LaunchStatus) {
        self.statuses.insert(id.to_owned(), status);
    }

    /// 某帳號目前的開啟進度。
    pub fn status(&self, id: &str) -> Option<&LaunchStatus> {
        self.statuses.get(id)
    }

    /// 帳號是否已在執行。
    pub fn is_running(&self, account: &Account) -> bool {
        self.running
            .iter()
            .any(|r| r.eq_ignore_ascii_case(&account.name))
    }

    /// 能不能勾：不在執行、也不在登入中。
    pub fn checkable(&self, account: &Account) -> bool {
        !self.is_running(account) && self.status(&account.id) != Some(&LaunchStatus::LoggingIn)
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

    /// 這次按「開啟」會開的帳號 id（勾了且能勾），依存放順序。
    pub fn launchable(&self) -> Vec<String> {
        self.accounts
            .iter()
            .filter(|a| self.is_checked(&a.id) && self.checkable(a))
            .map(|a| a.id.clone())
            .collect()
    }

    /// 「開啟 N 個帳號」的 N。
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

    /// 開始開啟：回傳要開的 id，並先標成登入中，回報到來前再按一次也不會重開。
    pub fn begin_launch(&mut self) -> Vec<String> {
        let ids = self.launchable();
        for id in &ids {
            self.set_status(id, LaunchStatus::LoggingIn);
        }
        ids
    }
}

/// 選單與中央按鈕之間的距離。
const BUTTON_GAP: f32 = 4.0;

/// 選單在 `avail`（可用的內容區）裡的範圍；`size` 是選單想要的大小，放不下時高度縮到 `avail` 為止。
pub fn menu_rect(anchor: Anchor, size: Vec2, avail: Rect) -> Rect {
    let height = size.y.min(avail.height());
    let width = size.x.min(avail.width());
    let (center_x, top) = match anchor {
        Anchor::Button(button) => {
            let below = button.max.y + BUTTON_GAP;
            (button.center().x, below.min(avail.max.y - height))
        }
        Anchor::TitleBar => (avail.max.x - width / 2.0, avail.min.y),
    };
    let left = (center_x - width / 2.0).clamp(avail.min.x, avail.max.x - width);
    Rect::from_min_size(
        Pos2::new(left, top.max(avail.min.y)),
        Vec2::new(width, height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::AccountKind;
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
        let state = MenuState::new(three(), &ids(&["b", "gone", "a"]));
        assert_eq!(state.selection(), ids(&["a", "b"]));
    }

    #[test]
    fn open_count_excludes_running() {
        let mut state = MenuState::new(three(), &ids(&["a", "b", "c"]));
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
        let mut state = MenuState::new(three(), &[]);
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
        let mut state = MenuState::new(three(), &ids(&["c"]));
        state.set_running(ids(&["Carol"]));
        state.toggle_all();
        state.toggle_all();
        assert_eq!(state.selection(), ids(&["c"]));
    }

    #[test]
    fn all_running_disables_open() {
        let mut state = MenuState::new(three(), &ids(&["a", "b", "c"]));
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
        let mut state = MenuState::new(three(), &ids(&["a", "b"]));
        assert_eq!(state.begin_launch(), ids(&["a", "b"]));
        assert_eq!(state.status("a"), Some(&LaunchStatus::LoggingIn));
        assert_eq!(state.open_count(), 0);
        assert!(state.begin_launch().is_empty());
        state.set_status("a", LaunchStatus::Opened);
        assert_eq!(state.launchable(), ids(&["a"]));
    }

    #[test]
    fn a_deleted_account_does_not_keep_its_progress() {
        let mut state = MenuState::new(three(), &ids(&["a"]));
        state.begin_launch();
        let rest: Vec<Account> = three().into_iter().filter(|a| a.id != "a").collect();
        state.set_accounts(rest, &[]);
        assert_eq!(state.status("a"), None);
    }

    #[test]
    fn editing_an_account_drops_its_last_result() {
        let mut state = MenuState::new(three(), &[]);
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
        let mut state = MenuState::new(three(), &[]);
        let name = three()[0].name.clone();
        state.set_status("a", LaunchStatus::Opened);
        state.set_running(vec![name]);
        assert_eq!(state.status("a"), Some(&LaunchStatus::Opened));
        state.set_running(Vec::new());
        assert_eq!(state.status("a"), None);
    }

    #[test]
    fn a_new_launch_clears_the_old_result() {
        let mut state = MenuState::new(three(), &ids(&["a"]));
        let raw = "Bad password".to_owned();
        state.set_status(
            "a",
            LaunchStatus::Failed(Failure::Login(LoginError::Rejected { raw })),
        );
        state.begin_launch();
        assert_eq!(state.status("a"), Some(&LaunchStatus::LoggingIn));
    }

    fn avail() -> Rect {
        Rect::from_min_max(Pos2::new(6.0, 28.0), Pos2::new(394.0, 400.0))
    }

    #[test]
    fn button_menu_sits_below_the_button_when_it_fits() {
        let button = Rect::from_min_max(Pos2::new(150.0, 100.0), Pos2::new(250.0, 132.0));
        let r = menu_rect(Anchor::Button(button), Vec2::new(200.0, 150.0), avail());
        assert_eq!(r.min.y, 132.0 + 4.0);
        assert_eq!(r.center().x, 200.0);
        assert_eq!(r.size(), Vec2::new(200.0, 150.0));
    }

    #[test]
    fn button_menu_moves_up_over_the_button_when_below_is_short() {
        let button = Rect::from_min_max(Pos2::new(150.0, 300.0), Pos2::new(250.0, 332.0));
        let r = menu_rect(Anchor::Button(button), Vec2::new(200.0, 150.0), avail());
        assert_eq!((r.min.y, r.max.y), (250.0, 400.0));
    }

    #[test]
    fn a_too_tall_menu_fills_the_area_below_the_title_bar() {
        let button = Rect::from_min_max(Pos2::new(150.0, 100.0), Pos2::new(250.0, 132.0));
        for anchor in [Anchor::Button(button), Anchor::TitleBar] {
            let r = menu_rect(anchor, Vec2::new(200.0, 900.0), avail());
            assert_eq!((r.min.y, r.max.y), (28.0, 400.0), "{anchor:?}");
        }
    }

    #[test]
    fn title_bar_menu_hangs_from_the_bar_at_the_right() {
        let r = menu_rect(Anchor::TitleBar, Vec2::new(200.0, 150.0), avail());
        assert_eq!((r.min.y, r.max.x), (28.0, 394.0));
    }

    #[test]
    fn a_wide_menu_is_clamped_inside_the_area() {
        let button = Rect::from_min_max(Pos2::new(10.0, 100.0), Pos2::new(60.0, 132.0));
        let r = menu_rect(Anchor::Button(button), Vec2::new(500.0, 100.0), avail());
        assert_eq!((r.min.x, r.max.x), (6.0, 394.0));
    }
}
