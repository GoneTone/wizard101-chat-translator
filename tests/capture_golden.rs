//! `window_region` 對照 Python 匯出的 golden fixture。

mod common;

use serde_json::Value;
use wizard101_chat_translator::region::capture::window_region;

fn ints<const N: usize>(v: &Value) -> [i32; N] {
    std::array::from_fn(|i| v[i].as_i64().unwrap() as i32)
}

#[test]
fn window_region_matches_python() {
    for case in common::golden("region_window_region") {
        let [x, y, w, h] = ints(&case["input"]["rect"]);
        let [ox, oy] = ints(&case["input"]["origin"]);
        let [cw, ch] = ints(&case["input"]["size"]);
        let expected = case["output"].as_array().map(|_| {
            let [a, b, c, d] = ints(&case["output"]);
            (a, b, c, d)
        });
        assert_eq!(
            window_region((x, y, w, h), (ox, oy), (cw, ch)),
            expected,
            "input: {}",
            case["input"]
        );
    }
}
