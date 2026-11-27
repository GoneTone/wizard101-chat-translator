//! 遊戲安裝位置偵測：Steam 收藏庫與官網版各自找，兩者可能同時存在。

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

use super::{reg_string, reg_subkeys};
pub use crate::game::process::PROCESS_NAME;
use crate::logs::log;

const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
const UNINSTALL_WOW64: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";

/// Steam 版用戶端才有的 Steamworks DLL，放在 `Bin` 底下。
pub const STEAM_API_DLL: &str = "steam_api64.dll";

/// 安裝來源；決定啟動參數與 Steam 票據能不能取得。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallSource {
    Steam,
    Official,
}

/// 一處有效的遊戲安裝；`root` 底下有 `Bin\WizardGraphicalClient.exe`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    pub source: InstallSource,
    pub root: PathBuf,
}

/// 安裝根目錄是否完整（用戶端主程式與遊戲資料都在）。
pub fn is_valid_install(root: &Path) -> bool {
    root.join("Bin").join(PROCESS_NAME).is_file() && root.join("Data").join("GameData").is_dir()
}

/// 解析 Steam 的 `libraryfolders.vdf`，回各收藏庫根目錄。
pub fn parse_steam_libraries(vdf: &str) -> Vec<PathBuf> {
    static PATH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#""path"\s+"((?:[^"\\]|\\.)*)""#).expect("valid regex"));
    PATH.captures_iter(vdf)
        .map(|c| PathBuf::from(c[1].replace(r"\\", r"\")))
        .collect()
}

fn steam_path() -> Option<PathBuf> {
    reg_string(HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath").map(PathBuf::from)
}

fn steam_install() -> Option<PathBuf> {
    let steam = steam_path()?;
    let vdf = std::fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")).ok();
    // 讀不到 vdf 時至少找 Steam 本身目錄下的預設收藏庫
    let libraries = vdf.map_or_else(|| vec![steam.clone()], |v| parse_steam_libraries(&v));
    libraries
        .into_iter()
        .map(|lib| lib.join("steamapps").join("common").join("Wizard101"))
        .find(|root| is_valid_install(root))
}

/// 依有無 [`STEAM_API_DLL`] 判斷安裝來源。
fn source_of(root: &Path) -> InstallSource {
    if root.join("Bin").join(STEAM_API_DLL).is_file() {
        InstallSource::Steam
    } else {
        InstallSource::Official
    }
}

/// 官網安裝程式登記在「應用程式」清單的安裝位置，使用者自選的路徑也找得到。
fn official_install() -> Option<PathBuf> {
    [
        (HKEY_CURRENT_USER, UNINSTALL),
        (HKEY_LOCAL_MACHINE, UNINSTALL),
        (HKEY_LOCAL_MACHINE, UNINSTALL_WOW64),
    ]
    .into_iter()
    .flat_map(|(root, list)| {
        reg_subkeys(root, list)
            .into_iter()
            .map(move |name| (root, format!(r"{list}\{name}")))
    })
    .filter(|(root, key)| reg_string(*root, key, "DisplayName").as_deref() == Some("Wizard101"))
    .filter_map(|(root, key)| reg_string(root, &key, "InstallLocation"))
    .map(PathBuf::from)
    // Steam 版也會登記同名項目
    .find(|root| is_valid_install(root) && source_of(root) == InstallSource::Official)
}

/// 偵測本機的遊戲安裝；Steam 版在前，可能兩者都有或都沒有。
pub fn detect_installs() -> Vec<Install> {
    let found: Vec<Install> = [
        (InstallSource::Steam, steam_install()),
        (InstallSource::Official, official_install()),
    ]
    .into_iter()
    .filter_map(|(source, root)| {
        Some(Install {
            source,
            root: root?,
        })
    })
    .collect();
    for install in &found {
        log(format!(
            "[launch] found {:?} install at {}",
            install.source,
            install.root.display()
        ));
    }
    if found.is_empty() {
        log("[launch] no game install found (Steam libraries and uninstall registry)");
    }
    found
}

/// 偵測結果併入使用者指定的 `manual`：有效時依有無 [`STEAM_API_DLL`] 判斷來源，取代同來源的偵測結果（沒有就加在後面）。
pub fn installs_with(mut detected: Vec<Install>, manual: Option<&Path>) -> Vec<Install> {
    let Some(root) = manual.filter(|root| is_valid_install(root)) else {
        return detected;
    };
    let source = source_of(root);
    let manual = Install {
        source,
        root: root.to_path_buf(),
    };
    match detected.iter_mut().find(|i| i.source == source) {
        Some(slot) => *slot = manual,
        None => detected.push(manual),
    }
    detected
}

#[cfg(test)]
mod tests {
    use super::*;

    const VDF: &str = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"apps"
		{
			"228980"		"0"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}"#;

    #[test]
    fn parse_steam_libraries_extracts_paths() {
        assert_eq!(
            parse_steam_libraries(VDF),
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"D:\SteamLibrary"),
            ]
        );
        assert!(parse_steam_libraries("").is_empty());
    }

    #[test]
    fn is_valid_install_requires_client_exe() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(!is_valid_install(root));
        std::fs::create_dir_all(root.join("Data").join("GameData")).unwrap();
        assert!(!is_valid_install(root));
        std::fs::create_dir_all(root.join("Bin")).unwrap();
        std::fs::write(root.join("Bin").join(PROCESS_NAME), b"").unwrap();
        assert!(is_valid_install(root));
    }

    fn fake_install(steam: bool) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("Data").join("GameData")).unwrap();
        std::fs::create_dir_all(root.join("Bin")).unwrap();
        std::fs::write(root.join("Bin").join(PROCESS_NAME), b"").unwrap();
        if steam {
            std::fs::write(root.join("Bin").join(STEAM_API_DLL), b"").unwrap();
        }
        dir
    }

    fn install(source: InstallSource, root: &str) -> Install {
        Install {
            source,
            root: PathBuf::from(root),
        }
    }

    #[test]
    fn installs_with_replaces_detected_entry_of_the_same_source() {
        let manual = fake_install(true);
        let detected = vec![
            install(InstallSource::Steam, r"C:\Steam\Wizard101"),
            install(InstallSource::Official, r"C:\ProgramData\Wizard101"),
        ];
        assert_eq!(
            installs_with(detected, Some(manual.path())),
            vec![
                Install {
                    source: InstallSource::Steam,
                    root: manual.path().to_path_buf(),
                },
                install(InstallSource::Official, r"C:\ProgramData\Wizard101"),
            ]
        );
    }

    #[test]
    fn installs_with_adds_an_official_install_without_steam_dll() {
        let manual = fake_install(false);
        let detected = vec![install(InstallSource::Steam, r"C:\Steam\Wizard101")];
        assert_eq!(
            installs_with(detected, Some(manual.path())),
            vec![
                install(InstallSource::Steam, r"C:\Steam\Wizard101"),
                Install {
                    source: InstallSource::Official,
                    root: manual.path().to_path_buf(),
                },
            ]
        );
    }

    #[test]
    fn installs_with_ignores_missing_or_invalid_manual_path() {
        let detected = vec![install(
            InstallSource::Official,
            r"C:\ProgramData\Wizard101",
        )];
        let invalid = tempfile::tempdir().unwrap();
        assert_eq!(installs_with(detected.clone(), None), detected);
        assert_eq!(
            installs_with(detected.clone(), Some(invalid.path())),
            detected
        );
    }
}
