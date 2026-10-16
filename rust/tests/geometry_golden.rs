//! 視窗幾何函式對照 Python 匯出的 golden fixture，外加 Rust 新增的拉回螢幕。

mod common;

use serde_json::{Value, json};
use wizard101_chat_translator::ui::geometry::{self, Edge, Placement, Rect};

fn int(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}

fn rect(v: &Value) -> Rect {
    (int(&v[0]), int(&v[1]), int(&v[2]), int(&v[3]))
}

fn edge(s: &str) -> Edge {
    [
        Edge::N,
        Edge::S,
        Edge::W,
        Edge::E,
        Edge::Nw,
        Edge::Ne,
        Edge::Sw,
        Edge::Se,
    ]
    .into_iter()
    .find(|e| e.as_str() == s)
    .unwrap()
}

fn placement(p: Placement) -> Value {
    json!([p.x, p.y, p.w, p.h, format!("{:?}", p.side).to_lowercase()])
}

#[test]
fn geometry_matches_python() {
    let cases = common::golden("geometry");
    for fname in [
        "moved_to",
        "centered_position",
        "clamped_position",
        "point_in_rect",
        "is_click",
        "edge_at",
        "resized_edge",
        "anchored_position",
        "anchored_geometry",
        "beside_geometry",
    ] {
        let count = cases.iter().filter(|c| c["input"]["fn"] == fname).count();
        assert!(count >= 30, "{fname}: only {count} cases");
    }
    for c in cases {
        let a = &c["input"]["args"];
        let i = |k: usize| int(&a[k]);
        let got = match c["input"]["fn"].as_str().unwrap() {
            "moved_to" => json!(geometry::moved_to(i(0), i(1), i(2), i(3))),
            "centered_position" => json!(geometry::centered_position(i(0), i(1), i(2), i(3))),
            "clamped_position" => {
                json!(geometry::clamped_position(
                    i(0),
                    i(1),
                    i(2),
                    i(3),
                    i(4),
                    i(5)
                ))
            }
            "point_in_rect" => json!(geometry::point_in_rect(i(0), i(1), i(2), i(3), i(4), i(5))),
            "is_click" => json!(geometry::is_click(i(0), i(1))),
            "edge_at" => json!(
                geometry::edge_at(i(0), i(1), i(2), i(3), i(4), i(5)).map_or("", Edge::as_str)
            ),
            "resized_edge" => json!(geometry::resized_edge(
                edge(a[0].as_str().unwrap()),
                i(1),
                i(2),
                i(3),
                i(4),
                i(5),
                i(6),
                i(7),
                i(8)
            )),
            "anchored_position" => json!(geometry::anchored_position(
                rect(&a[0]),
                i(1),
                i(2),
                rect(&a[3]),
                i(4)
            )),
            "anchored_geometry" => placement(geometry::anchored_geometry(
                rect(&a[0]),
                i(1),
                i(2),
                rect(&a[3]),
                i(4),
                i(5),
                i(6),
                i(7),
            )),
            "beside_geometry" => placement(geometry::beside_geometry(
                rect(&a[0]),
                i(1),
                i(2),
                rect(&a[3]),
                i(4),
                i(5),
                i(6),
            )),
            other => panic!("unknown fn {other}"),
        };
        assert_eq!(got, c["output"], "{}", c["input"]);
    }
}

#[test]
fn offscreen_geometry_is_pulled_back() {
    let screen = [(0, 0, 1920, 1080)];
    let (x, y) = geometry::pull_onscreen(2500, 300, 640, 420, &screen);
    assert!(
        x >= 0 && y >= 0 && x + 640 <= 1920 && y + 420 <= 1080,
        "({x}, {y})"
    );

    // 部分可見：留在原處，使用者自己拖得回來
    assert_eq!(
        geometry::pull_onscreen(1800, 900, 640, 420, &screen),
        (1800, 900)
    );
    assert_eq!(
        geometry::pull_onscreen(-600, -400, 640, 420, &screen),
        (-600, -400)
    );
}

#[test]
fn pull_onscreen_picks_the_nearest_monitor() {
    let monitors = [(0, 0, 1920, 1080), (1920, 0, 2560, 1440)];
    assert_eq!(
        geometry::pull_onscreen(5000, 200, 640, 420, &monitors),
        (1920 + 2560 - 640, 200)
    );
    assert_eq!(
        geometry::pull_onscreen(-3000, 2000, 640, 420, &monitors),
        (0, 1080 - 420)
    );
    // 視窗比螢幕大：貼齊左上角
    assert_eq!(
        geometry::pull_onscreen(5000, 5000, 3000, 2000, &monitors[..1]),
        (0, 0)
    );
    // 列不出螢幕時不動
    assert_eq!(
        geometry::pull_onscreen(5000, 5000, 640, 420, &[]),
        (5000, 5000)
    );
}
