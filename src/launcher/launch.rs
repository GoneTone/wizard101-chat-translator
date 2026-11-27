//! 用戶端啟動：依帳號類型挑安裝、組命令列參數、以 `Bin\` 為工作目錄啟動遊戲。

use std::process::Command;

use super::AccountKind;
use super::install::{Install, InstallSource, PROCESS_NAME};
use crate::logs::log;

const LOGIN_HOST: &str = "login.us.wizard101.com";
const LOGIN_PORT: &str = "12000";

/// 登入取得的憑證；刻意不實作 `Debug`，免得 ck2 被印進 log。
#[derive(Clone)]
pub struct Credentials {
    pub user_id: String,
    pub ck2: String,
    pub account: String,
}

/// 啟動失敗的原因；不含憑證。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchError {
    /// Steam 帳號需要 Steam 版安裝（票據 dll 在其 `Bin\`）。
    NoSteamInstall,
    /// 找不到可用的遊戲安裝或用戶端主程式。
    ClientMissing,
    /// 啟動程序失敗，帶系統錯誤原文。
    Spawn(String),
}

/// 依帳號類型挑安裝：Steam 帳號只能用 Steam 版；官網帳號優先官網版，沒有才用 Steam 版。
pub fn select_install(installs: &[Install], kind: AccountKind) -> Result<&Install, LaunchError> {
    let find = |source| installs.iter().find(|i| i.source == source);
    match kind {
        AccountKind::Steam => find(InstallSource::Steam).ok_or(LaunchError::NoSteamInstall),
        AccountKind::Official => find(InstallSource::Official)
            .or_else(|| find(InstallSource::Steam))
            .ok_or(LaunchError::ClientMissing),
    }
}

/// 組用戶端命令列參數。
///
/// Steam 參數（`-ST`、`-P 0`）只給 Steam 版上的 Steam 帳號；Steam 沒開時拿掉 `-ST`。
/// 官網帳號一律用官網參數，即使開的是 Steam 版目錄。
pub fn build_args(
    install: &Install,
    kind: AccountKind,
    steam_running: bool,
    creds: &Credentials,
) -> Vec<String> {
    let mut args: Vec<String> = ["-L", LOGIN_HOST, LOGIN_PORT, "-A", "en-US"]
        .map(String::from)
        .into();
    if install.source == InstallSource::Steam && kind == AccountKind::Steam {
        if steam_running {
            args.push("-ST".into());
        }
        args.extend(["-P".into(), "0".into()]);
    }
    // 本工具不做更新，啟動器本次更新秒數照實填 0
    args.extend(["-PT".into(), "0".into()]);
    args.extend([
        "-U".into(),
        format!("..{}", creds.user_id),
        creds.ck2.clone(),
        creds.account.clone(),
    ]);
    args
}

/// 以 `Bin\` 為工作目錄啟動用戶端，回 PID。
pub fn spawn_client(install: &Install, args: &[String]) -> Result<u32, LaunchError> {
    let bin = install.root.join("Bin");
    let exe = bin.join(PROCESS_NAME);
    if !exe.is_file() {
        log(format!("[launch] client missing at {}", exe.display()));
        return Err(LaunchError::ClientMissing);
    }
    match Command::new(&exe).args(args).current_dir(&bin).spawn() {
        Ok(child) => {
            log(format!(
                "[launch] started {:?} client pid={} from {}",
                install.source,
                child.id(),
                install.root.display()
            ));
            Ok(child.id())
        }
        Err(e) => {
            log(format!("[launch] spawn {} failed: {e}", exe.display()));
            Err(LaunchError::Spawn(e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn install(source: InstallSource) -> Install {
        Install {
            source,
            root: PathBuf::from(r"C:\W101"),
        }
    }

    fn creds() -> Credentials {
        Credentials {
            user_id: "1234567890123".into(),
            ck2: "K".repeat(88),
            account: "alice".into(),
        }
    }

    fn has(args: &[String], seq: &[&str]) -> bool {
        args.windows(seq.len()).any(|w| w == seq)
    }

    fn assert_skeleton(args: &[String]) {
        assert_eq!(args[..3], ["-L", "login.us.wizard101.com", "12000"]);
        assert!(has(args, &["-A", "en-US"]));
        let ck2 = "K".repeat(88);
        assert_eq!(
            args[args.len() - 4..],
            ["-U", "..1234567890123", ck2.as_str(), "alice"]
        );
    }

    #[test]
    fn steam_install_running_adds_st_and_p0() {
        let args = build_args(
            &install(InstallSource::Steam),
            AccountKind::Steam,
            true,
            &creds(),
        );
        assert_skeleton(&args);
        assert!(has(&args, &["-ST"]));
        assert!(has(&args, &["-P", "0"]));
        assert!(has(&args, &["-PT", "0"]));
    }

    #[test]
    fn steam_install_not_running_drops_st() {
        let args = build_args(
            &install(InstallSource::Steam),
            AccountKind::Steam,
            false,
            &creds(),
        );
        assert_skeleton(&args);
        assert!(!has(&args, &["-ST"]));
        assert!(has(&args, &["-P", "0"]));
        assert!(has(&args, &["-PT", "0"]));
    }

    #[test]
    fn official_install_minimal() {
        let args = build_args(
            &install(InstallSource::Official),
            AccountKind::Official,
            true,
            &creds(),
        );
        assert_skeleton(&args);
        assert!(has(&args, &["-PT", "0"]));
        assert!(!has(&args, &["-ST"]));
        assert!(!has(&args, &["-P"]));
    }

    #[test]
    fn official_account_on_steam_install_uses_official_args() {
        let args = build_args(
            &install(InstallSource::Steam),
            AccountKind::Official,
            true,
            &creds(),
        );
        assert_skeleton(&args);
        assert!(!has(&args, &["-ST"]));
        assert!(!has(&args, &["-P"]));
    }

    #[test]
    fn steam_account_without_steam_install_errors() {
        let installs = [install(InstallSource::Official)];
        assert_eq!(
            select_install(&installs, AccountKind::Steam),
            Err(LaunchError::NoSteamInstall)
        );
    }

    #[test]
    fn official_account_prefers_official() {
        let installs = [
            install(InstallSource::Steam),
            install(InstallSource::Official),
        ];
        assert_eq!(
            select_install(&installs, AccountKind::Official)
                .unwrap()
                .source,
            InstallSource::Official
        );
        assert_eq!(
            select_install(&installs[..1], AccountKind::Official)
                .unwrap()
                .source,
            InstallSource::Steam
        );
        assert_eq!(
            select_install(&[], AccountKind::Official),
            Err(LaunchError::ClientMissing)
        );
    }
}
