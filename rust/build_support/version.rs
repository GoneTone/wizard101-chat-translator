// 產生內嵌進 exe 的 rc 內容（manifest、圖示、VERSIONINFO），供 build.rs 與測試共用。

/// 取 VERSIONINFO 的四段數字版本，預發布後綴不計、不足補 0。
pub fn file_version(version: &str) -> [u16; 4] {
    let mut parts = [0u16; 4];
    let numbers = version.split('-').next().unwrap_or_default().split('.');
    for (slot, number) in parts.iter_mut().zip(numbers) {
        *slot = number.parse().unwrap_or(0);
    }
    parts
}

fn rc_path(path: &str) -> String {
    path.replace('\\', "\\\\")
}

/// 產生完整 rc 內容；manifest 與 icon 傳絕對路徑。
pub fn version_rc(version: &str, manifest: &str, icon: &str) -> String {
    let [a, b, c, d] = file_version(version);
    format!(
        r#"#pragma code_page(65001)
#define RT_MANIFEST 24
1 RT_MANIFEST "{manifest}"
1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {a},{b},{c},{d}
PRODUCTVERSION {a},{b},{c},{d}
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "GoneTone"
      VALUE "FileDescription", "Wizard101 Chat Translator"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "Wizard101ChatTranslator"
      VALUE "LegalCopyright", "© 2026 GoneTone"
      VALUE "OriginalFilename", "Wizard101ChatTranslator.exe"
      VALUE "ProductName", "Wizard101 Chat Translator"
      VALUE "ProductVersion", "{version}"
      VALUE "Comments", "Wizard101 Chat Translator — an AI translator for Wizard101's in-game chat, translating conversations both ways in real time."
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        manifest = rc_path(manifest),
        icon = rc_path(icon),
    )
}
