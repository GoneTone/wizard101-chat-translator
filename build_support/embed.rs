// 建置時把資料夾裡的 JSON 檔列成 `&[(檔名, include_str!(路徑))]` 原始碼，新增語系只要放檔案。

use std::path::Path;

/// `dir` 底下每個 `.json` 一項，依檔名（不含副檔名）字母序；回傳可直接 `include!` 的陣列運算式。
pub fn embedded_json_list(dir: &Path) -> String {
    let mut files: Vec<(String, String)> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| {
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            (stem, path.display().to_string())
        })
        .collect();
    files.sort();
    let items: String = files
        .iter()
        .map(|(stem, path)| format!("    ({stem:?}, include_str!({path:?})),\n"))
        .collect();
    format!("&[\n{items}]\n")
}
