//! `merge_lines` 與 `recognize` 對照 Python 匯出的 golden fixture。

use crate::common;

use wizard101_chat_translator::region::ocr::{OcrBox, merge_lines, recognize};

#[test]
fn merge_lines_matches_python() {
    for case in common::golden("ocr_merge_lines") {
        let items: Vec<OcrBox> = case["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                let pt = |i: usize| {
                    let p = &item[0][i];
                    (p[0].as_f64().unwrap() as f32, p[1].as_f64().unwrap() as f32)
                };
                OcrBox {
                    points: [pt(0), pt(1), pt(2), pt(3)],
                    text: item[1].as_str().unwrap().to_owned(),
                }
            })
            .collect();
        let expected: Vec<String> = serde_json::from_value(case["output"].clone()).unwrap();
        assert_eq!(merge_lines(&items), expected, "input: {}", case["input"]);
    }
}

#[test]
fn recognize_matches_python() {
    for case in common::golden("ocr_recognize") {
        let png = common::unhex(case["input"]["png"].as_str().unwrap());
        assert_eq!(
            recognize(&png).as_deref(),
            Ok(case["output"].as_str().unwrap()),
            "{}",
            case["input"]["name"]
        );
    }
}
