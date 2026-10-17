//! 把 Windows 系統字型裝進 egui：主字型依介面語言，其餘 CJK 字型與符號字型當 fallback；另有粗體與圖示字族。

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use eframe::egui::{Context, FontData, FontDefinitions, FontFamily};
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, MapViewOfFile, PAGE_READONLY,
};

use crate::logs::log;
use crate::ui::fonts::font_family;

/// CJK 字型檔，依 fallback 順序排列。
const CJK_FILES: [&str; 4] = ["msjh.ttc", "msyh.ttc", "YuGothM.ttc", "malgun.ttf"];
/// 泰文的 fallback：翻譯目標語言的說明以「ภาษาไทย」舉例，Tk 靠系統字型連結畫得出來。
const THAI_FILE: &str = "LeelawUI.ttf";
/// 最後的 fallback：⛶⚙─✕ 等符號。
const SYMBOL_FILE: &str = "seguisym.ttf";
/// 圖示字型：Win11 的 Segoe Fluent Icons，沒有就用 Win10 的 Segoe MDL2 Assets。
const ICON_FILES: [&str; 2] = ["SegoeIcons.ttf", "segmdl2.ttf"];

/// 粗體字族（`FontFamily::Name`）；字型檔讀不到時與一般字型相同。
pub const BOLD: &str = "bold";
/// 圖示字族（`FontFamily::Name`）；字型檔讀不到時與一般字型相同。
pub const ICONS: &str = "icons";

fn primary_file(family: &str) -> &'static str {
    match family {
        "Microsoft JhengHei" => "msjh.ttc",
        "Microsoft YaHei" => "msyh.ttc",
        "Yu Gothic UI" => "YuGothM.ttc",
        "Malgun Gothic" => "malgun.ttf",
        _ => "segoeui.ttf",
    }
}

/// 主字型對應的粗體檔。
fn bold_file(primary: &str) -> &'static str {
    match primary {
        "msjh.ttc" => "msjhbd.ttc",
        "msyh.ttc" => "msyhbd.ttc",
        "YuGothM.ttc" => "YuGothB.ttc",
        "malgun.ttf" => "malgunbd.ttf",
        _ => "segoeuib.ttf",
    }
}

/// 字型集（.ttc）裡要用的字型索引：YuGoth*.ttc 的 0 是 Yu Gothic，1 才是 Python 指定的 Yu Gothic UI。
fn face_index(file: &str) -> u32 {
    u32::from(matches!(file, "YuGothM.ttc" | "YuGothB.ttc"))
}

/// 介面語言要載入的字型檔，依優先順序。
pub fn font_files(language_code: &str) -> Vec<&'static str> {
    let primary = primary_file(font_family(language_code));
    let mut files = vec![primary];
    files.extend(CJK_FILES.iter().filter(|f| **f != primary));
    files.push(THAI_FILE);
    files.push(SYMBOL_FILE);
    files
}

/// 系統字型資料夾：`%WINDIR%\Fonts`（Windows 不一定裝在 C 槽），讀不到環境變數才用 `C:\Windows`。
fn fonts_dir(windir: Option<OsString>) -> PathBuf {
    windir
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join("Fonts")
}

/// Tk 的字級（pt）換成 egui 的 point（DIP）。
pub fn tk_pt(pt: f32) -> f32 {
    pt * 96.0 / 72.0
}

/// 從系統字型資料夾載入介面字型；讀不到的檔案略過並記 log。
pub fn install(ctx: &Context, language_code: &str) {
    let started = Instant::now();
    install_from(ctx, &fonts_dir(std::env::var_os("WINDIR")), language_code);
    log(format!(
        "[ui] fonts installed in {} ms",
        started.elapsed().as_millis()
    ));
}

/// 唯讀映射整個檔案且永不解除：字型位元組走系統的檔案快取，不佔數十 MB 私有記憶體。
///
/// shortcut: each call maps again (language changes leak address space, not RAM); cache per path if installs become frequent.
fn map_file(path: &Path) -> io::Result<&'static [u8]> {
    let file = File::open(path)?;
    let len = file.metadata()?.len() as usize;
    unsafe {
        let mapping = CreateFileMappingW(
            file.as_raw_handle(),
            std::ptr::null(),
            PAGE_READONLY,
            0,
            0,
            std::ptr::null(),
        );
        if mapping.is_null() {
            return Err(io::Error::last_os_error());
        }
        let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
        let error = io::Error::last_os_error();
        CloseHandle(mapping);
        if view.Value.is_null() {
            return Err(error);
        }
        Ok(std::slice::from_raw_parts(view.Value as *const u8, len))
    }
}

/// 載入一個字型檔進 `defs`；讀不到記 log 並回 `false`。
fn load(defs: &mut FontDefinitions, dir: &Path, file: &str) -> bool {
    let path = dir.join(file);
    match map_file(&path) {
        Ok(bytes) => {
            let mut data = FontData::from_static(bytes);
            data.index = face_index(file);
            defs.font_data.insert(file.to_owned(), Arc::new(data));
            true
        }
        Err(err) => {
            log(format!(
                "[ui] font skipped: path={} error={err}",
                path.display()
            ));
            false
        }
    }
}

fn install_from(ctx: &Context, dir: &Path, language_code: &str) {
    let mut defs = FontDefinitions::default();
    let files = font_files(language_code);
    let loaded: Vec<String> = files
        .iter()
        .filter(|file| load(&mut defs, dir, file))
        .map(|file| (*file).to_owned())
        .collect();
    // egui 內建字型留在最後，系統字型全缺時仍顯示得出拉丁字
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let list = defs.families.entry(family).or_default();
        list.splice(0..0, loaded.iter().cloned());
    }
    let regular = defs.families[&FontFamily::Proportional].clone();
    let bold = Some(bold_file(files[0])).filter(|file| load(&mut defs, dir, file));
    let icons = ICON_FILES
        .into_iter()
        .find(|file| load(&mut defs, dir, file));
    for (name, file) in [(BOLD, bold), (ICONS, icons)] {
        if file.is_none() {
            log(format!(
                "[ui] {name} font unavailable; using the regular font"
            ));
        }
        let list = file.map(str::to_owned).into_iter().chain(regular.clone());
        defs.families
            .insert(FontFamily::Name(name.into()), list.collect());
    }
    ctx.set_fonts(defs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_font_first_then_other_cjk_then_thai_and_symbols() {
        assert_eq!(
            font_files("zh-TW"),
            [
                "msjh.ttc",
                "msyh.ttc",
                "YuGothM.ttc",
                "malgun.ttf",
                "LeelawUI.ttf",
                "seguisym.ttf"
            ]
        );
        assert_eq!(
            font_files("ja"),
            [
                "YuGothM.ttc",
                "msjh.ttc",
                "msyh.ttc",
                "malgun.ttf",
                "LeelawUI.ttf",
                "seguisym.ttf"
            ]
        );
        assert_eq!(
            font_files("en-US"),
            [
                "segoeui.ttf",
                "msjh.ttc",
                "msyh.ttc",
                "YuGothM.ttc",
                "malgun.ttf",
                "LeelawUI.ttf",
                "seguisym.ttf"
            ]
        );
    }

    #[test]
    fn bold_file_follows_the_primary_font() {
        let bold: Vec<_> = ["zh-TW", "zh-CN", "ja", "ko", "en-US"]
            .iter()
            .map(|code| bold_file(font_files(code)[0]))
            .collect();
        assert_eq!(
            bold,
            [
                "msjhbd.ttc",
                "msyhbd.ttc",
                "YuGothB.ttc",
                "malgunbd.ttf",
                "segoeuib.ttf"
            ]
        );
    }

    #[test]
    fn yu_gothic_uses_the_ui_face() {
        assert_eq!(face_index("YuGothM.ttc"), 1);
        assert_eq!(face_index("YuGothB.ttc"), 1);
        assert_eq!(face_index("msjh.ttc"), 0);
        assert_eq!(face_index("msyh.ttc"), 0);
        assert_eq!(face_index("malgun.ttf"), 0);
    }

    #[test]
    fn mapped_file_matches_its_bytes() {
        let dir = fonts_dir(std::env::var_os("WINDIR"));
        let path = dir.join("segoeui.ttf");
        assert_eq!(map_file(&path).unwrap(), std::fs::read(&path).unwrap());
        assert!(map_file(&dir.join("no-such-font.ttf")).is_err());
    }

    #[test]
    fn fonts_dir_follows_windir() {
        assert_eq!(
            fonts_dir(Some(r"D:\WINNT".into())),
            Path::new(r"D:\WINNT\Fonts")
        );
        assert_eq!(fonts_dir(None), Path::new(r"C:\Windows\Fonts"));
    }

    #[test]
    fn tk_point_sizes_convert_to_dip() {
        assert_eq!(tk_pt(9.0), 12.0);
        assert!((tk_pt(11.0) - 14.67).abs() < 0.01);
    }

    #[test]
    fn missing_font_files_keep_builtin_fonts() {
        let ctx = Context::default();
        let empty = tempfile::tempdir().unwrap();
        install_from(&ctx, empty.path(), "zh-TW");
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
        let builtin = FontDefinitions::default().families[&FontFamily::Proportional].clone();
        let installed = ctx.fonts(|f| f.definitions().families[&FontFamily::Proportional].clone());
        assert_eq!(installed, builtin);
    }

    #[test]
    fn missing_bold_and_icon_fonts_fall_back_to_the_regular_family() {
        let ctx = Context::default();
        let empty = tempfile::tempdir().unwrap();
        install_from(&ctx, empty.path(), "zh-TW");
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
        let families = ctx.fonts(|f| f.definitions().families.clone());
        for name in [BOLD, ICONS] {
            assert_eq!(
                families[&FontFamily::Name(name.into())],
                families[&FontFamily::Proportional]
            );
        }
    }

    #[test]
    fn system_fonts_put_bold_and_icon_files_first() {
        let ctx = Context::default();
        install_from(&ctx, &fonts_dir(std::env::var_os("WINDIR")), "zh-TW");
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
        let families = ctx.fonts(|f| f.definitions().families.clone());
        assert_eq!(families[&FontFamily::Name(BOLD.into())][0], "msjhbd.ttc");
        assert_eq!(families[&FontFamily::Name(BOLD.into())][1], "msjh.ttc");
        assert!(ICON_FILES.contains(&families[&FontFamily::Name(ICONS.into())][0].as_str()));
    }
}
