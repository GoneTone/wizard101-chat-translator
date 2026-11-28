//! 一鍵啟動：官網帳號並行、Steam 帳號逐一，逐帳號回報結果；單一帳號失敗不影響其他帳號。

use std::borrow::Borrow;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared, join_all};
use zeroize::Zeroizing;

use super::accounts::decrypt_password;
use super::install::{Install, InstallSource};
use super::launch::{Credentials, LaunchError, build_args, select_install, spawn_client};
use super::login::{self, LoginError, LoginOutcome};
use super::patch::{self, PatchFailure, PatchOutcome, PatchStatus, SystemPatchEnv};
use super::steam::{self, SteamError, SteamSession};
use super::{Account, AccountKind, SteamTicket, running};
use crate::logs::log;
use crate::util::lock;

/// 單一帳號為什麼沒開成；介面依類型轉成文字，伺服器拒絕時直接顯示原文。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// 存的密碼解不開（換了 Windows 使用者或設定檔被搬到別台電腦）。
    Password,
    Steam(SteamError),
    Login(LoginError),
    Launch(LaunchError),
    /// 安裝沒通過更新檢查而未啟動；原因只顯示在該安裝的狀態。
    Patch,
}

/// 單一帳號的啟動進度與結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchStatus {
    /// 等這份安裝的更新檢查。
    Waiting(InstallSource),
    LoggingIn,
    Opened,
    /// 已在執行（不論是誰開的），不重開。
    SkippedRunning,
    Failed(Failure),
}

impl LaunchStatus {
    /// 還沒有結果（等檢查或登入中）。
    pub fn in_progress(&self) -> bool {
        matches!(self, Self::Waiting(_) | Self::LoggingIn)
    }
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
    /// 檢查（官網版順便更新）一份安裝；不借用 `self`，別批的帳號才能一起等。
    fn ensure_patched(
        &self,
        install: &Install,
    ) -> impl Future<Output = PatchResult> + Send + 'static;
    /// 某份安裝的檢查狀態有變；`None` 表示收起。
    fn report_patch(&self, source: InstallSource, status: Option<&PatchStatus>);
    /// 跨批次共用的檢查閘門。
    fn patch_gate(&self) -> &PatchGate;
}

type PatchResult = Result<PatchOutcome, PatchFailure>;
type PatchCheck = Shared<BoxFuture<'static, PatchResult>>;

/// 同一份安裝同時只跑一個更新檢查；不分批次，等待中的帳號共用結果。
#[derive(Default)]
pub struct PatchGate {
    checks: Mutex<HashMap<PathBuf, (PatchCheck, usize)>>,
}

/// 一個帳號在等某份安裝的檢查；drop 時離開，最後離開的收起通過的狀態。
struct PatchWait<'a, D: LauncherDeps> {
    deps: &'a D,
    install: &'a Install,
    check: PatchCheck,
}

impl<'a, D: LauncherDeps> PatchWait<'a, D> {
    fn enter(deps: &'a D, install: &'a Install) -> Self {
        let mut checks = lock(&deps.patch_gate().checks);
        let (check, waiters) = checks
            .entry(install.root.clone())
            .or_insert_with(|| (deps.ensure_patched(install).boxed().shared(), 0));
        *waiters += 1;
        log(format!(
            "[launch] waiting for install check: root={} waiters={waiters}",
            install.root.display()
        ));
        let check = check.clone();
        Self {
            deps,
            install,
            check,
        }
    }
}

impl<D: LauncherDeps> Drop for PatchWait<'_, D> {
    fn drop(&mut self) {
        let root = &self.install.root;
        let mut checks = lock(&self.deps.patch_gate().checks);
        let Some((check, waiters)) = checks.get_mut(root) else {
            return;
        };
        *waiters -= 1;
        if *waiters == 0 {
            // 與 enter 同一把鎖下收起，免得晚到的 None 蓋掉下一次檢查的狀態
            if matches!(check.peek(), Some(Ok(_))) {
                self.deps.report_patch(self.install.source, None);
            }
            checks.remove(root);
        }
    }
}

/// 收到單一帳號進度的回呼。
pub type StatusSink = Box<dyn Fn(&str, &LaunchStatus) + Send + Sync>;

/// 收到安裝檢查狀態的回呼；檢查跑在背景、可能比送它的這批還久，所以是 `Arc`。
pub type PatchSink = Arc<dyn Fn(InstallSource, Option<&PatchStatus>) + Send + Sync>;

/// 實際的系統動作；`installs` 由呼叫端給（偵測結果或使用者手動指定的位置）。
pub struct SystemDeps {
    pub installs: Vec<Install>,
    pub on_status: StatusSink,
    pub on_patch: PatchSink,
    pub patch_gate: Arc<PatchGate>,
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
    fn ensure_patched(
        &self,
        install: &Install,
    ) -> impl Future<Output = PatchResult> + Send + 'static {
        let (install, sink) = (install.clone(), Arc::clone(&self.on_patch));
        async move {
            let report = |status: PatchStatus| sink(install.source, Some(&status));
            patch::check_install(&install, &SystemPatchEnv::new(), &report).await
        }
    }
    fn report_patch(&self, source: InstallSource, status: Option<&PatchStatus>) {
        (self.on_patch)(source, status);
    }
    fn patch_gate(&self) -> &PatchGate {
        &self.patch_gate
    }
}

impl Borrow<SteamTicket> for SteamSession {
    fn borrow(&self) -> &SteamTicket {
        &self.ticket
    }
}

/// 同一時間只允許一個 Steam 會話（初始化、取票據、登入到釋放）。
static STEAM_SESSION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 帳號已排進某份安裝的檢查：要用的安裝、密碼與正在等的檢查。
struct Ready<'a, D: LauncherDeps> {
    install: &'a Install,
    password: Zeroizing<String>,
    wait: PatchWait<'a, D>,
}

/// 不必等檢查就能判定的先判定（已在執行、沒有安裝、密碼解不開），其餘排進該安裝的檢查。
fn prepare<'a, D: LauncherDeps>(
    account: &Account,
    deps: &'a D,
    installs: &'a [Install],
    running: &[String],
) -> Result<Ready<'a, D>, LaunchStatus> {
    if running
        .iter()
        .any(|r| r.eq_ignore_ascii_case(&account.name))
    {
        return Err(LaunchStatus::SkippedRunning);
    }
    let failed = LaunchStatus::Failed;
    let install = select_install(installs, account.kind).map_err(|e| failed(Failure::Launch(e)))?;
    let password = deps.password(account).ok_or(failed(Failure::Password))?;
    deps.report(&account.id, &LaunchStatus::Waiting(install.source));
    Ok(Ready {
        install,
        password,
        wait: PatchWait::enter(deps, install),
    })
}

async fn attempt<D: LauncherDeps>(
    account: &Account,
    deps: &D,
    ready: &Ready<'_, D>,
    machine: &str,
) -> Result<LaunchStatus, Failure> {
    let install = ready.install;
    let patched = ready.wait.check.clone().await.map_err(|_| Failure::Patch)?;
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
            &ready.password,
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
    // Steam 版的檔案歸 Steam 更新，這裡只檢查
    let patch_seconds = match install.source {
        InstallSource::Steam => 0,
        InstallSource::Official => patched.seconds,
    };
    // Steam 帳號走到這裡必定剛取得票據，Steam 一定開著
    let args = build_args(install, account.kind, true, patch_seconds, &creds);
    deps.spawn(install, &args).map_err(Failure::Launch)?;
    Ok(LaunchStatus::Opened)
}

/// 跑完一個帳號並回報結果。
async fn run_one<D: LauncherDeps>(
    account: &Account,
    ready: Result<Ready<'_, D>, LaunchStatus>,
    deps: &D,
    machine: &str,
) -> (String, LaunchStatus) {
    let status = match &ready {
        Ok(ready) => attempt(account, deps, ready, machine)
            .await
            .unwrap_or_else(LaunchStatus::Failed),
        Err(status) => status.clone(),
    };
    log(format!("[launch] account {}: {status:?}", account.id));
    deps.report(&account.id, &status);
    // 帳號結果送出後才離開等待，安裝狀態才不會比帳號結果先收起
    drop(ready);
    (account.id.clone(), status)
}

/// 啟動勾選的帳號；回每個帳號 id 與結果，順序同 `selected`。
pub async fn launch_accounts<D: LauncherDeps>(
    selected: &[Account],
    deps: &D,
) -> Vec<(String, LaunchStatus)> {
    let installs = deps.installs();
    let running = deps.running_accounts();
    let machine = deps.machine_id();
    let machine = machine.as_str();
    // 先全部排進檢查：逐一輪到的 Steam 帳號也算等待中，檢查才不會被前一個帳號提早收掉
    let (official, steam): (Vec<_>, Vec<_>) = selected
        .iter()
        .map(|a| (a, prepare(a, deps, &installs, &running)))
        .partition(|(a, _)| a.kind == AccountKind::Official);
    // Steam 一次只能發一張票據：Steam 帳號逐一，與官網帳號同時進行
    let steam_in_turn = async {
        let mut out = Vec::new();
        for (account, ready) in steam {
            out.push(run_one(account, ready, deps, machine).await);
        }
        out
    };
    let official = official
        .into_iter()
        .map(|(account, ready)| run_one(account, ready, deps, machine));
    let (mut done, steam_done) = futures_util::join!(join_all(official), steam_in_turn);
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
        patch: fn(InstallSource) -> PatchResult,
        barrier: Option<Barrier>,
        spawned: Mutex<Vec<String>>,
        args: Mutex<Vec<Vec<String>>>,
        reports: Mutex<Vec<(String, LaunchStatus)>>,
        patch_reports: Mutex<Vec<(InstallSource, Option<PatchStatus>)>>,
        checks: AtomicUsize,
        gate: PatchGate,
        steam_active: AtomicUsize,
        steam_peak: AtomicUsize,
    }

    fn passed(seconds: u64) -> PatchResult {
        Ok(PatchOutcome {
            downloaded: seconds > 0,
            seconds,
        })
    }

    impl Default for Fake {
        fn default() -> Self {
            Self {
                running: Vec::new(),
                reject: None,
                steam: Ok(()),
                patch: |_| passed(0),
                barrier: None,
                spawned: Mutex::new(Vec::new()),
                args: Mutex::new(Vec::new()),
                reports: Mutex::new(Vec::new()),
                patch_reports: Mutex::new(Vec::new()),
                checks: AtomicUsize::new(0),
                gate: PatchGate::default(),
                steam_active: AtomicUsize::new(0),
                steam_peak: AtomicUsize::new(0),
            }
        }
    }

    impl LauncherDeps for Fake {
        type Ticket = SteamTicket;

        fn installs(&self) -> Vec<Install> {
            vec![
                Install {
                    source: InstallSource::Steam,
                    root: PathBuf::from(r"C:\Steam\W101"),
                },
                Install {
                    source: InstallSource::Official,
                    root: PathBuf::from(r"C:\W101"),
                },
            ]
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
            self.args.lock().unwrap().push(args.to_vec());
            Ok(1)
        }
        fn report(&self, id: &str, status: &LaunchStatus) {
            self.reports
                .lock()
                .unwrap()
                .push((id.into(), status.clone()));
        }
        fn ensure_patched(
            &self,
            install: &Install,
        ) -> impl Future<Output = PatchResult> + Send + 'static {
            self.checks.fetch_add(1, Ordering::SeqCst);
            let result = (self.patch)(install.source);
            // 要真的讓出執行權，另一批才有機會在檢查途中加入
            async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                result
            }
        }
        fn report_patch(&self, source: InstallSource, status: Option<&PatchStatus>) {
            self.patch_reports
                .lock()
                .unwrap()
                .push((source, status.cloned()));
        }
        fn patch_gate(&self) -> &PatchGate {
            &self.gate
        }
    }

    fn reports_of(fake: &Fake, id: &str) -> Vec<LaunchStatus> {
        let reports = fake.reports.lock().unwrap();
        reports
            .iter()
            .filter(|(i, _)| i == id)
            .map(|(_, s)| s.clone())
            .collect()
    }

    fn hidden(source: InstallSource) -> Vec<(InstallSource, Option<PatchStatus>)> {
        vec![(source, None)]
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
        assert_eq!(reports_of(&fake, "a"), [LaunchStatus::SkippedRunning]);
    }

    #[tokio::test]
    async fn accounts_wait_for_the_install_check_before_login() {
        let fake = Fake::default();
        let selected = [
            account("o", AccountKind::Official),
            account("s", AccountKind::Steam),
        ];
        launch_accounts(&selected, &fake).await;
        for (id, source) in [("o", InstallSource::Official), ("s", InstallSource::Steam)] {
            assert_eq!(
                reports_of(&fake, id),
                [
                    LaunchStatus::Waiting(source),
                    LaunchStatus::LoggingIn,
                    LaunchStatus::Opened
                ]
            );
        }
    }

    #[tokio::test]
    async fn failed_check_blocks_every_account_on_that_install_only() {
        let fake = Fake {
            patch: |source| match source {
                InstallSource::Official => Err(PatchFailure::GameOpen),
                InstallSource::Steam => passed(0),
            },
            ..Fake::default()
        };
        let selected = [
            account("o1", AccountKind::Official),
            account("o2", AccountKind::Official),
            account("s", AccountKind::Steam),
        ];
        let results = launch_accounts(&selected, &fake).await;
        let blocked = LaunchStatus::Failed(Failure::Patch);
        assert_eq!(*status_of(&results, "o1"), blocked);
        assert_eq!(*status_of(&results, "o2"), blocked);
        assert_eq!(*status_of(&results, "s"), LaunchStatus::Opened);
        assert_eq!(*fake.spawned.lock().unwrap(), vec!["s-name".to_string()]);
        assert!(!reports_of(&fake, "o1").contains(&LaunchStatus::LoggingIn));
        assert_eq!(fake.checks.load(Ordering::SeqCst), 2, "one per install");
        assert_eq!(
            *fake.patch_reports.lock().unwrap(),
            hidden(InstallSource::Steam),
            "a failed check stays visible"
        );
    }

    #[tokio::test]
    async fn concurrent_batches_share_one_check() {
        let fake = Fake::default();
        let first = [account("a", AccountKind::Official)];
        let second = [account("b", AccountKind::Official)];
        tokio::join!(
            launch_accounts(&first, &fake),
            launch_accounts(&second, &fake)
        );
        assert_eq!(fake.checks.load(Ordering::SeqCst), 1);
        assert_eq!(fake.spawned.lock().unwrap().len(), 2);
        assert_eq!(
            *fake.patch_reports.lock().unwrap(),
            hidden(InstallSource::Official),
            "hidden once, after both batches"
        );
    }

    #[tokio::test]
    async fn steam_accounts_in_one_batch_share_one_check() {
        let fake = Fake::default();
        let selected = [
            account("s1", AccountKind::Steam),
            account("s2", AccountKind::Steam),
        ];
        launch_accounts(&selected, &fake).await;
        assert_eq!(fake.checks.load(Ordering::SeqCst), 1);
        assert_eq!(
            *fake.patch_reports.lock().unwrap(),
            hidden(InstallSource::Steam)
        );
    }

    #[tokio::test]
    async fn the_next_launch_checks_again() {
        let fake = Fake::default();
        let selected = [account("a", AccountKind::Official)];
        launch_accounts(&selected, &fake).await;
        launch_accounts(&selected, &fake).await;
        assert_eq!(fake.checks.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn patch_seconds_reach_the_pt_argument() {
        let fake = Fake {
            patch: |_| passed(7),
            ..Fake::default()
        };
        let selected = [
            account("o", AccountKind::Official),
            account("s", AccountKind::Steam),
        ];
        launch_accounts(&selected, &fake).await;
        let args = fake.args.lock().unwrap();
        let pt = |name: &str| {
            let args = args.iter().find(|a| a.last().unwrap() == name).unwrap();
            let at = args.iter().position(|a| a == "-PT").unwrap();
            args[at + 1].clone()
        };
        assert_eq!(pt("o-name"), "7");
        assert_eq!(pt("s-name"), "0", "Steam installs are never patched here");
    }

    #[tokio::test]
    async fn successful_check_is_hidden_after_its_accounts_finish() {
        let fake = Fake::default();
        let selected = [
            account("a", AccountKind::Official),
            account("b", AccountKind::Official),
        ];
        launch_accounts(&selected, &fake).await;
        assert_eq!(
            *fake.patch_reports.lock().unwrap(),
            hidden(InstallSource::Official)
        );
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
            on_patch: Arc::new(|_, _| {}),
            patch_gate: Arc::default(),
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
