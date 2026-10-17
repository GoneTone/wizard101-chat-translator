//! OCR 新舊對照的 Rust 側：辨識資料夾內的 `*.png`，寫出 `rust.json`。

use std::{env, fs, path::PathBuf};

use serde_json::{Map, Value};
use wizard101_chat_translator::region::ocr::recognize;

fn main() {
    let dir = PathBuf::from(env::args().nth(1).expect("usage: ocr_compare <dir>"));
    let mut pngs: Vec<_> = fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .collect();
    pngs.sort();
    let mut results = Map::new();
    for path in pngs {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = match fs::read(&path) {
            Ok(png) => recognize(&png).map_err(|e| e.0),
            Err(e) => Err(e.to_string()),
        }
        .unwrap_or_else(|msg| {
            eprintln!("failed: {name}: {msg}");
            format!("(error: {msg})")
        });
        results.insert(name, Value::String(text));
    }
    let dest = dir.join("rust.json");
    fs::write(&dest, serde_json::to_string_pretty(&results).unwrap()).expect("write json");
    println!("{}", dest.display());
}
