//! 用戶端啟動：依帳號類型挑安裝、組命令列參數、以 `Bin\` 為工作目錄啟動遊戲。

use std::process::Command;

use super::AccountKind;
use super::install::{Install, InstallSource, PROCESS_NAME};
use super::steam::WIZARD101_APP_ID;
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
    /// 找不到可用的遊戲安裝或用戶端主程式。
    ClientMissing,
    /// 啟動程序失敗，帶系統錯誤原文。
    Spawn(String),
}

/// 依帳號類型挑安裝：優先用同類型的安裝，沒有就用另一種（兩者的用戶端與票據 dll 相同）。
pub fn select_install(installs: &[Install], kind: AccountKind) -> Result<&Install, LaunchError> {
    let find = |source| installs.iter().find(|i| i.source == source);
    let (first, second) = match kind {
        AccountKind::Steam => (InstallSource::Steam, InstallSource::Official),
        AccountKind::Official => (InstallSource::Official, InstallSource::Steam),
    };
    find(first)
        .or_else(|| find(second))
        .ok_or(LaunchError::ClientMissing)
}

/// 組用戶端命令列參數。
///
/// `-ST` 給 Steam 帳號（Steam 沒開時拿掉）；`-P 0` 只給 Steam 版上的 Steam 帳號，
/// 官網版要靠遊戲內更新邊玩邊下載。官網帳號一律用官網參數，即使開的是 Steam 版目錄。
pub fn build_args(
    install: &Install,
    kind: AccountKind,
    steam_running: bool,
    creds: &Credentials,
) -> Vec<String> {
    let mut args: Vec<String> = ["-L", LOGIN_HOST, LOGIN_PORT, "-A", "en-US"]
        .map(String::from)
        .into();
    if kind == AccountKind::Steam {
        if steam_running {
            args.push("-ST".into());
        }
        if install.source == InstallSource::Steam {
            args.extend(["-P".into(), "0".into()]);
        }
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

fn client_command(install: &Install, args: &[String]) -> Command {
    let bin = install.root.join("Bin");
    let mut command = Command::new(bin.join(PROCESS_NAME));
    command.args(args).current_dir(bin);
    for key in ["SteamAppId", "SteamGameId"] {
        // 帶 -ST 卻沒有這組變數時，用戶端會請 Steam 重開自己並跳出自訂參數確認窗；
        // 其餘情況不可繼承取票據時本程序暫設的值
        if args.iter().any(|a| a == "-ST") {
            command.env(key, WIZARD101_APP_ID);
        } else {
            command.env_remove(key);
        }
    }
    command
}

/// 以 `Bin\` 為工作目錄啟動用戶端，回 PID。
pub fn spawn_client(install: &Install, args: &[String]) -> Result<u32, LaunchError> {
    let bin = install.root.join("Bin");
    let exe = bin.join(PROCESS_NAME);
    if !exe.is_file() {
        log(format!("[launch] client missing at {}", exe.display()));
        return Err(LaunchError::ClientMissing);
    }
    match client_command(install, args).spawn() {
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
    fn client_does_not_inherit_steam_app_id() {
        let command = client_command(&install(InstallSource::Official), &[]);
        let removed: Vec<_> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect();
        assert!(removed.contains(&"SteamAppId".to_string()), "{removed:?}");
        assert!(removed.contains(&"SteamGameId".to_string()), "{removed:?}");
    }

    #[test]
    fn steam_client_is_told_it_was_started_for_wizard101() {
        let command = client_command(&install(InstallSource::Steam), &["-ST".into()]);
        let set: Vec<_> = command
            .get_envs()
            .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
            .collect();
        assert!(set.contains(&("SteamAppId", "799960")), "{set:?}");
        assert!(set.contains(&("SteamGameId", "799960")), "{set:?}");
    }

    #[test]
    fn steam_account_on_official_install_keeps_in_game_patching() {
        let args = build_args(
            &install(InstallSource::Official),
            AccountKind::Steam,
            true,
            &creds(),
        );
        assert_skeleton(&args);
        assert!(has(&args, &["-ST"]));
        assert!(!has(&args, &["-P"]));
    }

    #[test]
    fn steam_account_prefers_steam_and_falls_back_to_official() {
        let installs = [
            install(InstallSource::Official),
            install(InstallSource::Steam),
        ];
        assert_eq!(
            select_install(&installs, AccountKind::Steam)
                .unwrap()
                .source,
            InstallSource::Steam
        );
        assert_eq!(
            select_install(&installs[..1], AccountKind::Steam)
                .unwrap()
                .source,
            InstallSource::Official
        );
        assert_eq!(
            select_install(&[], AccountKind::Steam),
            Err(LaunchError::ClientMissing)
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
