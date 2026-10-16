//! 把 Windows 系統字型裝進 egui：主字型依介面語言，其餘 CJK 字型與符號字型當 fallback。

use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use eframe::egui::{Context, FontData, FontDefinitions, FontFamily};
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, MapViewOfFile, PAGE_READONLY,
};

use crate::logs::log;
use crate::ui::fonts::font_family;

const FONTS_DIR: &str = r"C:\Windows\Fonts";
/// CJK 字型檔，依 fallback 順序排列。
const CJK_FILES: [&str; 4] = ["msjh.ttc", "msyh.ttc", "YuGothM.ttc", "malgun.ttf"];
/// 最後的 fallback：⛶⚙─✕ 等符號。
const SYMBOL_FILE: &str = "seguisym.ttf";

fn primary_file(family: &str) -> &'static str {
    match family {
        "Microsoft JhengHei" => "msjh.ttc",
        "Microsoft YaHei" => "msyh.ttc",
        "Yu Gothic UI" => "YuGothM.ttc",
        "Malgun Gothic" => "malgun.ttf",
        _ => "segoeui.ttf",
    }
}

/// 字型集（.ttc）裡要用的字型索引：YuGothM.ttc 的 0 是 Yu Gothic Medium，1 才是 Python 指定的 Yu Gothic UI。
fn face_index(file: &str) -> u32 {
    u32::from(file == "YuGothM.ttc")
}

/// 介面語言要載入的字型檔，依優先順序。
pub fn font_files(language_code: &str) -> Vec<&'static str> {
    let primary = primary_file(font_family(language_code));
    let mut files = vec![primary];
    files.extend(CJK_FILES.iter().filter(|f| **f != primary));
    files.push(SYMBOL_FILE);
    files
}

/// Tk 的字級（pt）換成 egui 的 point（DIP）。
pub fn tk_pt(pt: f32) -> f32 {
    pt * 96.0 / 72.0
}

/// 從 `C:\Windows\Fonts` 載入介面字型；讀不到的檔案略過並記 log。
pub fn install(ctx: &Context, language_code: &str) {
    let started = Instant::now();
    install_from(ctx, Path::new(FONTS_DIR), language_code);
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

fn install_from(ctx: &Context, dir: &Path, language_code: &str) {
    let mut defs = FontDefinitions::default();
    let mut loaded = Vec::new();
    for file in font_files(language_code) {
        let path = dir.join(file);
        match map_file(&path) {
            Ok(bytes) => {
                let mut data = FontData::from_static(bytes);
                data.index = face_index(file);
                defs.font_data.insert(file.to_owned(), Arc::new(data));
                loaded.push(file.to_owned());
            }
            Err(err) => log(format!(
                "[ui] font skipped: path={} error={err}",
                path.display()
            )),
        }
    }
    // egui 內建字型留在最後，系統字型全缺時仍顯示得出拉丁字
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let list = defs.families.entry(family).or_default();
        list.splice(0..0, loaded.iter().cloned());
    }
    ctx.set_fonts(defs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_font_first_then_other_cjk_then_symbols() {
        assert_eq!(
            font_files("zh-TW"),
            [
                "msjh.ttc",
                "msyh.ttc",
                "YuGothM.ttc",
                "malgun.ttf",
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
                "seguisym.ttf"
            ]
        );
    }

    #[test]
    fn yu_gothic_uses_the_ui_face() {
        assert_eq!(face_index("YuGothM.ttc"), 1);
        assert_eq!(face_index("msjh.ttc"), 0);
        assert_eq!(face_index("msyh.ttc"), 0);
        assert_eq!(face_index("malgun.ttf"), 0);
    }

    #[test]
    fn mapped_file_matches_its_bytes() {
        let path = Path::new(FONTS_DIR).join("segoeui.ttf");
        assert_eq!(map_file(&path).unwrap(), std::fs::read(&path).unwrap());
        assert!(map_file(&Path::new(FONTS_DIR).join("no-such-font.ttf")).is_err());
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
}
