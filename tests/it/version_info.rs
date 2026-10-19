//! exe 版本資訊（VERSIONINFO）產生邏輯的測試。

#[path = "../../build_support/version.rs"]
mod version;

use version::{file_version, version_rc};

#[test]
fn file_version_strips_the_prerelease_suffix() {
    assert_eq!(file_version("1.0.0"), [1, 0, 0, 0]);
    assert_eq!(file_version("1.0.0-rc.1"), [1, 0, 0, 0]);
    assert_eq!(file_version("0.6.12"), [0, 6, 12, 0]);
}

#[test]
fn version_rc_matches_the_pyinstaller_strings() {
    let rc = version_rc("0.6.12-rc.1", r"C:\a\app.manifest", r"C:\a\icon.ico");
    for expected in [
        "FILEVERSION 0,6,12,0",
        "PRODUCTVERSION 0,6,12,0",
        "FILEFLAGSMASK 0x3f",
        "FILEOS 0x40004",
        "FILETYPE 0x1",
        "BLOCK \"040904b0\"",
        "VALUE \"CompanyName\", \"GoneTone\"",
        "VALUE \"FileDescription\", \"Wizard101 Chat Translator\"",
        "VALUE \"FileVersion\", \"0.6.12-rc.1\"",
        "VALUE \"InternalName\", \"Wizard101ChatTranslator\"",
        "VALUE \"LegalCopyright\", \"© 2026 GoneTone\"",
        "VALUE \"OriginalFilename\", \"Wizard101ChatTranslator.exe\"",
        "VALUE \"ProductName\", \"Wizard101 Chat Translator\"",
        "VALUE \"ProductVersion\", \"0.6.12-rc.1\"",
        "VALUE \"Comments\", \"Wizard101 Chat Translator — an AI translator for Wizard101's in-game chat, translating conversations both ways in real time.\"",
        "VALUE \"Translation\", 0x409, 1200",
        "#pragma code_page(65001)",
    ] {
        assert!(rc.contains(expected), "missing {expected:?} in:\n{rc}");
    }
}

#[test]
fn version_rc_escapes_backslashes_in_resource_paths() {
    let rc = version_rc("1.0.0", r"C:\a\app.manifest", r"C:\a\icon.ico");
    assert!(rc.contains(r#"1 RT_MANIFEST "C:\\a\\app.manifest""#));
    assert!(rc.contains(r#"1 ICON "C:\\a\\icon.ico""#));
}
