//! 遊戲安裝位置偵測：Steam 收藏庫與官網版各自找，兩者可能同時存在。

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};

use super::hkcu_string;
pub use crate::game::process::PROCESS_NAME;
use crate::logs::log;

// GetDriveTypeW 的回傳值：只掃本機與卸除式磁碟，網路磁碟斷線時檢查路徑會卡住
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;

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
    hkcu_string(r"Software\Valve\Steam", "SteamPath").map(PathBuf::from)
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

fn official_install() -> Option<PathBuf> {
    // SAFETY：無參數的查詢。
    let drives = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| drives & (1 << i) != 0)
        .map(|i| format!("{}:\\", (b'A' + i) as char))
        .filter(|drive| {
            let wide: Vec<u16> = drive.encode_utf16().chain([0]).collect();
            // SAFETY：wide 以 NUL 結尾。
            let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
            kind == DRIVE_FIXED || kind == DRIVE_REMOVABLE
        })
        .map(|drive| {
            Path::new(&drive)
                .join("ProgramData")
                .join("KingsIsle Entertainment")
                .join("Wizard101")
        })
        .find(|root| is_valid_install(root))
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
        log("[launch] no game install found (Steam libraries and ProgramData on local drives)");
    }
    found
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
}
