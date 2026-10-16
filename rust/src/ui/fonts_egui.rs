//! 把 Windows 系統字型裝進 egui：主字型依介面語言，其餘 CJK 字型與符號字型當 fallback。

use std::path::Path;
use std::sync::Arc;

use eframe::egui::{Context, FontData, FontDefinitions, FontFamily};

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
    install_from(ctx, Path::new(FONTS_DIR), language_code);
}

fn install_from(ctx: &Context, dir: &Path, language_code: &str) {
    let mut defs = FontDefinitions::default();
    let mut loaded = Vec::new();
    for file in font_files(language_code) {
        let path = dir.join(file);
        match std::fs::read(&path) {
            Ok(bytes) => {
                defs.font_data
                    .insert(file.to_owned(), Arc::new(FontData::from_owned(bytes)));
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
