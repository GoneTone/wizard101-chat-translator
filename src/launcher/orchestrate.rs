//! 一鍵啟動：官網帳號並行、Steam 帳號逐一，逐帳號回報結果；單一帳號失敗不影響其他帳號。

use std::borrow::Borrow;
use std::collections::HashMap;
use std::future::Future;

use futures_util::future::join_all;
use zeroize::Zeroizing;

use super::accounts::decrypt_password;
use super::install::Install;
use super::launch::{Credentials, LaunchError, build_args, select_install, spawn_client};
use super::login::{self, LoginError, LoginOutcome};
use super::steam::{self, SteamError, SteamSession};
use super::{Account, AccountKind, SteamTicket, running};
use crate::logs::log;

/// 單一帳號為什麼沒開成；介面依類型轉成文字，伺服器拒絕時直接顯示原文。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// 存的密碼解不開（換了 Windows 使用者或設定檔被搬到別台電腦）。
    Password,
    Steam(SteamError),
    Login(LoginError),
    Launch(LaunchError),
}

/// 單一帳號的啟動進度與結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchStatus {
    LoggingIn,
    Opened,
    /// 已在執行（不論是誰開的），不重開。
    SkippedRunning,
    Failed(Failure),
}

/// 遊戲關掉後撤掉「已啟動／已在執行」：上次掃描還在執行、這次不在的帳號。
pub fn forget_exited(
    statuses: &mut HashMap<String, LaunchStatus>,
    accounts: &[Account],
    before: &[String],
    after: &[String],
) {
    let listed =
        |names: &[String], a: &Account| names.iter().any(|n| n.eq_ignore_ascii_case(&a.name));
    for account in accounts
        .iter()
        .filter(|a| listed(before, a) && !listed(after, a))
    {
        if matches!(
            statuses.get(&account.id),
            Some(LaunchStatus::Opened | LaunchStatus::SkippedRunning)
        ) {
            statuses.remove(&account.id);
        }
    }
}

/// 一鍵啟動用到的外部動作；測試以假實作替換。
pub trait LauncherDeps: Sync {
    /// Steam 會話；持有期間票據有效。
    type Ticket: Borrow<SteamTicket> + Send;

    fn installs(&self) -> Vec<Install>;
    fn running_accounts(&self) -> Vec<String>;
    fn password(&self, account: &Account) -> Option<Zeroizing<String>>;
    fn machine_id(&self) -> String;
    fn acquire_ticket(
        &self,
        install: &Install,
    ) -> impl Future<Output = Result<Self::Ticket, SteamError>> + Send;
    fn sign_in(
        &self,
        account: &str,
        password: &str,
        machine: &str,
        steam: Option<&SteamTicket>,
    ) -> impl Future<Output = Result<LoginOutcome, LoginError>> + Send;
    fn spawn(&self, install: &Install, args: &[String]) -> Result<u32, LaunchError>;
    /// 某帳號的進度有變（登入中、完成、失敗）。
    fn report(&self, id: &str, status: &LaunchStatus);
}

/// 收到單一帳號進度的回呼。
pub type StatusSink = Box<dyn Fn(&str, &LaunchStatus) + Send + Sync>;

/// 實際的系統動作；`installs` 由呼叫端給（偵測結果或使用者手動指定的位置）。
pub struct SystemDeps {
    pub installs: Vec<Install>,
    pub on_status: StatusSink,
}

impl LauncherDeps for SystemDeps {
    type Ticket = SteamSession;

    fn installs(&self) -> Vec<Install> {
        self.installs.clone()
    }
    fn running_accounts(&self) -> Vec<String> {
        running::running_accounts()
    }
    fn password(&self, account: &Account) -> Option<Zeroizing<String>> {
        decrypt_password(&account.password_dpapi)
            .inspect_err(|e| log(format!("[launch] account {}: {}", account.id, e.0)))
            .ok()
    }
    fn machine_id(&self) -> String {
        login::machine_id()
    }
    async fn acquire_ticket(&self, install: &Install) -> Result<SteamSession, SteamError> {
        let install = install.clone();
        // 等 Steam 回票據最久 30 秒，不能卡住非同步執行緒
        tokio::task::spawn_blocking(move || steam::acquire_ticket(&install))
            .await
            .unwrap_or(Err(SteamError::InterfaceUnavailable))
    }
    async fn sign_in(
        &self,
        account: &str,
        password: &str,
        machine: &str,
        steam: Option<&SteamTicket>,
    ) -> Result<LoginOutcome, LoginError> {
        login::sign_in(account, password, machine, steam).await
    }
    fn spawn(&self, install: &Install, args: &[String]) -> Result<u32, LaunchError> {
        spawn_client(install, args)
    }
    fn report(&self, id: &str, status: &LaunchStatus) {
        (self.on_status)(id, status);
    }
}

impl Borrow<SteamTicket> for SteamSession {
    fn borrow(&self) -> &SteamTicket {
        &self.ticket
    }
}

/// 同一時間只允許一個 Steam 會話（初始化、取票據、登入到釋放）。
static STEAM_SESSION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn attempt<D: LauncherDeps>(
    account: &Account,
    deps: &D,
    installs: &[Install],
    running: &[String],
    machine: &str,
) -> Result<LaunchStatus, Failure> {
    if running
        .iter()
        .any(|r| r.eq_ignore_ascii_case(&account.name))
    {
        return Ok(LaunchStatus::SkippedRunning);
    }
    let install = select_install(installs, account.kind).map_err(Failure::Launch)?;
    let password = deps.password(account).ok_or(Failure::Password)?;
    deps.report(&account.id, &LaunchStatus::LoggingIn);
    // 跨批次也要排隊：兩批同時開時，一批的 Shutdown 會打斷另一批正在用的 Steam API
    let steam_turn = match account.kind {
        AccountKind::Steam => Some(STEAM_SESSION.lock().await),
        AccountKind::Official => None,
    };
    let ticket = match account.kind {
        AccountKind::Steam => Some(deps.acquire_ticket(install).await.map_err(Failure::Steam)?),
        AccountKind::Official => None,
    };
    let outcome = deps
        .sign_in(
            &account.name,
            &password,
            machine,
            ticket.as_ref().map(Borrow::borrow),
        )
        .await
        .map_err(Failure::Login)?;
    drop(ticket);
    drop(steam_turn);
    let creds = Credentials {
        user_id: outcome.user_id,
        ck2: outcome.ck2,
        account: account.name.clone(),
    };
    // Steam 帳號走到這裡必定剛取得票據，Steam 一定開著
    let args = build_args(install, account.kind, true, &creds);
    deps.spawn(install, &args).map_err(Failure::Launch)?;
    Ok(LaunchStatus::Opened)
}

/// 啟動勾選的帳號；回每個帳號 id 與結果，順序同 `selected`。
pub async fn launch_accounts<D: LauncherDeps>(
    selected: &[Account],
    deps: &D,
) -> Vec<(String, LaunchStatus)> {
    let installs = deps.installs();
    let running = deps.running_accounts();
    let machine = deps.machine_id();
    let one = |account: &Account| {
        let (installs, running, machine) = (&installs, &running, &machine);
        let account = account.clone();
        async move {
            let status = attempt(&account, deps, installs, running, machine)
                .await
                .unwrap_or_else(LaunchStatus::Failed);
            log(format!("[launch] account {}: {status:?}", account.id));
            deps.report(&account.id, &status);
            (account.id, status)
        }
    };
    let (official, steam): (Vec<_>, Vec<_>) = selected
        .iter()
        .partition(|a| a.kind == AccountKind::Official);
    // Steam 一次只能發一張票據：Steam 帳號逐一，與官網帳號同時進行
    let steam_in_turn = async {
        let mut out = Vec::new();
        for account in steam {
            out.push(one(account).await);
        }
        out
    };
    let (mut done, steam_done) =
        futures_util::join!(join_all(official.into_iter().map(one)), steam_in_turn);
    done.extend(steam_done);
    selected
        .iter()
        .filter_map(|a| {
            let at = done.iter().position(|(id, _)| *id == a.id)?;
            Some(done.swap_remove(at))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use tokio::sync::Barrier;

    use crate::launcher::install::InstallSource;

    fn account(id: &str, kind: AccountKind) -> Account {
        Account {
            id: id.into(),
            name: format!("{id}-name"),
            kind,
            password_dpapi: String::new(),
        }
    }

    struct Fake {
        running: Vec<String>,
        reject: Option<(&'static str, &'static str)>,
        steam: Result<(), SteamError>,
        barrier: Option<Barrier>,
        spawned: Mutex<Vec<String>>,
        reports: Mutex<Vec<(String, LaunchStatus)>>,
        steam_active: AtomicUsize,
        steam_peak: AtomicUsize,
    }

    impl Default for Fake {
        fn default() -> Self {
            Self {
                running: Vec::new(),
                reject: None,
                steam: Ok(()),
                barrier: None,
                spawned: Mutex::new(Vec::new()),
                reports: Mutex::new(Vec::new()),
                steam_active: AtomicUsize::new(0),
                steam_peak: AtomicUsize::new(0),
            }
        }
    }

    impl LauncherDeps for Fake {
        type Ticket = SteamTicket;

        fn installs(&self) -> Vec<Install> {
            [InstallSource::Steam, InstallSource::Official]
                .map(|source| Install {
                    source,
                    root: PathBuf::from(r"C:\W101"),
                })
                .into()
        }
        fn running_accounts(&self) -> Vec<String> {
            self.running.clone()
        }
        fn password(&self, _: &Account) -> Option<Zeroizing<String>> {
            Some(Zeroizing::new("pw".into()))
        }
        fn machine_id(&self) -> String {
            "machine".into()
        }
        async fn acquire_ticket(&self, _: &Install) -> Result<SteamTicket, SteamError> {
            let now = self.steam_active.fetch_add(1, Ordering::SeqCst) + 1;
            self.steam_peak.fetch_max(now, Ordering::SeqCst);
            self.steam.clone().map(|()| SteamTicket {
                steam_id: "1".into(),
                ticket: vec![1],
            })
        }
        async fn sign_in(
            &self,
            account: &str,
            _: &str,
            _: &str,
            steam: Option<&SteamTicket>,
        ) -> Result<LoginOutcome, LoginError> {
            if let Some(barrier) = &self.barrier {
                barrier.wait().await;
            }
            if steam.is_some() {
                tokio::time::sleep(Duration::from_millis(20)).await;
                self.steam_active.fetch_sub(1, Ordering::SeqCst);
            }
            if let Some((name, raw)) = self.reject
                && name == account
            {
                return Err(LoginError::Rejected { raw: raw.into() });
            }
            Ok(LoginOutcome {
                user_id: "1".into(),
                ck2: "ck2".into(),
            })
        }
        fn spawn(&self, _: &Install, args: &[String]) -> Result<u32, LaunchError> {
            self.spawned
                .lock()
                .unwrap()
                .push(args.last().unwrap().clone());
            Ok(1)
        }
        fn report(&self, id: &str, status: &LaunchStatus) {
            self.reports
                .lock()
                .unwrap()
                .push((id.into(), status.clone()));
        }
    }

    fn status_of<'a>(results: &'a [(String, LaunchStatus)], id: &str) -> &'a LaunchStatus {
        &results.iter().find(|(i, _)| i == id).unwrap().1
    }

    #[tokio::test]
    async fn official_accounts_run_in_parallel_and_report_opened() {
        let fake = Fake {
            barrier: Some(Barrier::new(3)),
            ..Fake::default()
        };
        let selected = ["a", "b", "c"].map(|id| account(id, AccountKind::Official));
        // 三個登入都卡在同一個 barrier：依序執行就會逾時
        let results =
            tokio::time::timeout(Duration::from_secs(5), launch_accounts(&selected, &fake))
                .await
                .expect("official sign-ins must run concurrently");
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|(_, s)| *s == LaunchStatus::Opened));
        assert_eq!(fake.spawned.lock().unwrap().len(), 3);
        let reports = fake.reports.lock().unwrap();
        assert!(reports.contains(&("a".into(), LaunchStatus::LoggingIn)));
        assert!(reports.contains(&("a".into(), LaunchStatus::Opened)));
    }

    #[tokio::test]
    async fn running_account_is_skipped() {
        let fake = Fake {
            running: vec!["A-NAME".into()],
            ..Fake::default()
        };
        let selected = [
            account("a", AccountKind::Official),
            account("b", AccountKind::Official),
        ];
        let results = launch_accounts(&selected, &fake).await;
        assert_eq!(*status_of(&results, "a"), LaunchStatus::SkippedRunning);
        assert_eq!(*status_of(&results, "b"), LaunchStatus::Opened);
        assert_eq!(*fake.spawned.lock().unwrap(), vec!["b-name".to_string()]);
    }

    #[tokio::test]
    async fn login_rejection_surfaces_server_text_and_others_continue() {
        let fake = Fake {
            reject: Some(("a-name", "Invalid password")),
            ..Fake::default()
        };
        let selected = [
            account("a", AccountKind::Official),
            account("b", AccountKind::Official),
            account("s", AccountKind::Steam),
        ];
        let results = launch_accounts(&selected, &fake).await;
        assert_eq!(
            *status_of(&results, "a"),
            LaunchStatus::Failed(Failure::Login(LoginError::Rejected {
                raw: "Invalid password".into()
            }))
        );
        assert_eq!(*status_of(&results, "b"), LaunchStatus::Opened);
        assert_eq!(*status_of(&results, "s"), LaunchStatus::Opened);
    }

    #[tokio::test]
    async fn steam_error_fails_only_that_account() {
        let fake = Fake {
            steam: Err(SteamError::NotRunning),
            ..Fake::default()
        };
        let selected = [
            account("s", AccountKind::Steam),
            account("o", AccountKind::Official),
        ];
        let results = launch_accounts(&selected, &fake).await;
        assert_eq!(
            *status_of(&results, "s"),
            LaunchStatus::Failed(Failure::Steam(SteamError::NotRunning))
        );
        assert_eq!(*status_of(&results, "o"), LaunchStatus::Opened);
        assert_eq!(*fake.spawned.lock().unwrap(), vec!["o-name".to_string()]);
    }

    #[tokio::test]
    async fn steam_sessions_never_overlap_across_batches() {
        let fake = Fake::default();
        let first = [account("s1", AccountKind::Steam)];
        let second = [account("s2", AccountKind::Steam)];
        tokio::join!(
            launch_accounts(&first, &fake),
            launch_accounts(&second, &fake)
        );
        assert_eq!(fake.steam_peak.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn system_launch_future_is_send() {
        fn assert_send<T: Send>(_: &T) {}
        let deps = SystemDeps {
            installs: Vec::new(),
            on_status: Box::new(|_, _| {}),
        };
        assert_send(&launch_accounts(&[], &deps));
    }

    #[tokio::test]
    async fn results_keep_the_selection_order() {
        let fake = Fake::default();
        let selected = [
            account("s", AccountKind::Steam),
            account("o", AccountKind::Official),
        ];
        let ids: Vec<String> = launch_accounts(&selected, &fake)
            .await
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, ["s", "o"]);
    }

    #[test]
    fn forget_exited_clears_only_accounts_that_stopped_running() {
        let accounts = [
            account("a", AccountKind::Official),
            account("b", AccountKind::Official),
            account("c", AccountKind::Official),
        ];
        let mut statuses = HashMap::from([
            ("a".to_owned(), LaunchStatus::Opened),
            ("b".to_owned(), LaunchStatus::SkippedRunning),
            ("c".to_owned(), LaunchStatus::Opened),
        ]);
        let before = ["A-NAME".to_owned(), "b-name".to_owned()];
        forget_exited(&mut statuses, &accounts, &before, &["b-name".to_owned()]);
        assert_eq!(statuses.get("a"), None);
        assert_eq!(statuses.get("b"), Some(&LaunchStatus::SkippedRunning));
        // 還沒被掃描看到過的剛開帳號不清
        assert_eq!(statuses.get("c"), Some(&LaunchStatus::Opened));
    }
}
