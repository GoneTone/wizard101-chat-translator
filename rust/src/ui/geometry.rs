//! 無邊框視窗拖曳、縮放、命中判定與貼齊擺放的純幾何函式（overlay、泡泡、輸入框共用，不碰視窗）。

/// 螢幕矩形 `(x, y, w, h)`。
pub type Rect = (i32, i32, i32, i32);

/// 四邊的縮放感應寬度（px）。
pub const EDGE: i32 = 6;
/// 四角的縮放感應範圍（px）：比邊寬，角落才好抓。
pub const CORNER: i32 = 14;
/// 按下到放開的位移小於這個值算點擊。
pub const CLICK_THRESHOLD: i32 = 5;
/// 夾在螢幕內時保留的餘裕。
pub const EDGE_MARGIN: i32 = 4;

/// 游標壓在視窗的哪一條邊／哪個角。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// 上緣。
    N,
    /// 下緣。
    S,
    /// 左緣。
    W,
    /// 右緣。
    E,
    /// 左上角。
    Nw,
    /// 右上角。
    Ne,
    /// 左下角。
    Sw,
    /// 右下角。
    Se,
}

impl Edge {
    /// 與 Python 相同的方位字串（`"n"`、`"se"`…），log 用。
    pub fn as_str(self) -> &'static str {
        match self {
            Edge::N => "n",
            Edge::S => "s",
            Edge::W => "w",
            Edge::E => "e",
            Edge::Nw => "nw",
            Edge::Ne => "ne",
            Edge::Sw => "sw",
            Edge::Se => "se",
        }
    }
}

/// 拖曳位移後的新左上角座標。
pub fn moved_to(start_x: i32, start_y: i32, dx: i32, dy: i32) -> (i32, i32) {
    (start_x + dx, start_y + dy)
}

/// 把 w×h 的視窗擺在螢幕正中央時的左上角座標。
pub fn centered_position(screen_w: i32, screen_h: i32, w: i32, h: i32) -> (i32, i32) {
    ((screen_w - w).div_euclid(2), (screen_h - h).div_euclid(2))
}

/// 把彈出位置夾進螢幕內：貼近右／下緣時往回收，免得選單有一半在畫面外。
pub fn clamped_position(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    screen_w: i32,
    screen_h: i32,
) -> (i32, i32) {
    (
        EDGE_MARGIN.max(x.min(screen_w - w - EDGE_MARGIN)),
        EDGE_MARGIN.max(y.min(screen_h - h - EDGE_MARGIN)),
    )
}

/// (px, py) 是否落在左上角 (x, y)、寬 w 高 h 的矩形內（含邊界）。
pub fn point_in_rect(px: i32, py: i32, x: i32, y: i32, w: i32, h: i32) -> bool {
    x <= px && px < x + w && y <= py && py < y + h
}

/// 按下到放開的位移是否算點擊（否則視為拖曳）。
pub fn is_click(dx: i32, dy: i32) -> bool {
    dx.abs() < CLICK_THRESHOLD && dy.abs() < CLICK_THRESHOLD
}

/// 游標壓在視窗的哪一條邊／哪個角，都不是則 `None`。
///
/// 角落的判定帶比邊寬，且兩軸都落在角落帶內才算角 —— 否則靠近角的邊會很難單軸縮放。
pub fn edge_at(px: i32, py: i32, x: i32, y: i32, w: i32, h: i32) -> Option<Edge> {
    if !point_in_rect(px, py, x, y, w, h) {
        return None;
    }
    let (left, right) = (px - x, x + w - 1 - px);
    let (top, bottom) = (py - y, y + h - 1 - py);
    let north = top < CORNER;
    let south = !north && bottom < CORNER;
    let west = left < CORNER;
    let east = !west && right < CORNER;
    Some(match () {
        _ if north && west => Edge::Nw,
        _ if north && east => Edge::Ne,
        _ if south && west => Edge::Sw,
        _ if south && east => Edge::Se,
        _ if top < EDGE => Edge::N,
        _ if bottom < EDGE => Edge::S,
        _ if left < EDGE => Edge::W,
        _ if right < EDGE => Edge::E,
        _ => return None,
    })
}

/// 從某條邊／角拖曳 (dx, dy) 後的新幾何 (x, y, w, h)。
///
/// 拉左緣／上緣要同時改位置與尺寸，對邊才會留在原處；寬高撞到最小值後位置就凍住，
/// 否則游標繼續往內移會把整個視窗一起拖走。
#[allow(clippy::too_many_arguments)]
pub fn resized_edge(
    edge: Edge,
    mut x: i32,
    mut y: i32,
    mut w: i32,
    mut h: i32,
    dx: i32,
    dy: i32,
    min_w: i32,
    min_h: i32,
) -> Rect {
    let edge = edge.as_str();
    if edge.contains('e') {
        w = min_w.max(w + dx);
    } else if edge.contains('w') {
        let new_w = min_w.max(w - dx);
        x += w - new_w;
        w = new_w;
    }
    if edge.contains('s') {
        h = min_h.max(h + dy);
    } else if edge.contains('n') {
        let new_h = min_h.max(h - dy);
        y += h - new_h;
        h = new_h;
    }
    (x, y, w, h)
}

/// 把 w×h 的視窗貼在錨點矩形正下方、左緣對齊，回傳左上角座標。
///
/// 下方放不下 area（螢幕工作區）就翻到錨點上方；最後把座標夾進 area 內。
pub fn anchored_position(anchor: Rect, w: i32, h: i32, area: Rect, gap: i32) -> (i32, i32) {
    let (ax, ay, _, ah) = anchor;
    let (area_x, area_y, area_w, area_h) = area;
    let mut y = ay + ah + gap;
    if y + h > area_y + area_h {
        y = ay - h - gap;
    }
    let x = area_x.max(ax.min(area_x + area_w - w));
    let y = area_y.max(y.min(area_y + area_h - h));
    (x, y)
}

/// 貼齊擺放落在錨點的哪一側。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// 正下方。
    Below,
    /// 正上方。
    Above,
    /// 左側。
    Left,
    /// 右側。
    Right,
}

/// 貼齊擺放的結果：視窗幾何加上落在錨點的哪一側。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// 左上角 x。
    pub x: i32,
    /// 左上角 y。
    pub y: i32,
    /// 寬。
    pub w: i32,
    /// 高。
    pub h: i32,
    /// 落在錨點的哪一側。
    pub side: Side,
}

/// 把視窗貼在錨點矩形旁。順序：正下方 → 正上方 → 左右較寬的一側。
///
/// 貼上下時寬 w（隨錨點）、高隨內容 h 但不低於 min_h；上下都放不下才走側邊，
/// 改用內容不換行的寬度 side_w（見 `beside_geometry`）。
#[allow(clippy::too_many_arguments)]
pub fn anchored_geometry(
    anchor: Rect,
    w: i32,
    h: i32,
    area: Rect,
    gap: i32,
    side_w: i32,
    min_w: i32,
    min_h: i32,
) -> Placement {
    let (ax, ay, _, ah) = anchor;
    let (area_x, area_y, area_w, area_h) = area;
    let (area_right, area_bottom) = (area_x + area_w, area_y + area_h);
    let h = min_h.max(h);
    let x = area_x.max(ax.min(area_right - w));
    let below = ay + ah + gap;
    if below + h <= area_bottom {
        return Placement {
            x,
            y: below,
            w,
            h,
            side: Side::Below,
        };
    }
    let above = ay - h - gap;
    if above >= area_y {
        return Placement {
            x,
            y: above,
            w,
            h,
            side: Side::Above,
        };
    }
    beside_geometry(anchor, side_w, h, area, gap, min_w, min_h)
}

/// 把視窗放在錨點左右較寬的一側、頂端對齊錨點。
///
/// 寬度取 w 與該側剩餘空間較小者、不低於 min_w；高度夾成工作區高、不低於 min_h，
/// 內容多出來的部分交給呼叫端捲動；最後夾進工作區，該側空間不足就蓋到錨點一角。
pub fn beside_geometry(
    anchor: Rect,
    w: i32,
    h: i32,
    area: Rect,
    gap: i32,
    min_w: i32,
    min_h: i32,
) -> Placement {
    let (ax, ay, aw, _) = anchor;
    let (area_x, area_y, area_w, area_h) = area;
    let (area_right, area_bottom) = (area_x + area_w, area_y + area_h);
    let left_room = ax - gap - area_x;
    let right_room = area_right - (ax + aw + gap);
    let side = if right_room >= left_room {
        Side::Right
    } else {
        Side::Left
    };
    let w = min_w.max(w.min(left_room.max(right_room)));
    let x = match side {
        Side::Right => ax + aw + gap,
        _ => ax - gap - w,
    };
    let h = min_h.max(h.min(area_h));
    let x = area_x.max(x.min(area_right - w));
    let y = area_y.max(ay.min(area_bottom - h));
    Placement { x, y, w, h, side }
}

/// 整個視窗都不在任何螢幕內時，拉回最近的螢幕（只改位置，不改大小）；部分可見就不動。
///
/// 舊版設定在高 DPI 螢幕上存的可能是邏輯座標，換成實體像素解讀後會落到畫面外。
pub fn pull_onscreen(x: i32, y: i32, w: i32, h: i32, monitors: &[Rect]) -> (i32, i32) {
    let visible = monitors
        .iter()
        .any(|&(mx, my, mw, mh)| x < mx + mw && mx < x + w && y < my + mh && my < y + h);
    let gap = |&&(mx, my, mw, mh): &&Rect| {
        let dx = i64::from((mx - (x + w)).max(x - (mx + mw)).max(0));
        let dy = i64::from((my - (y + h)).max(y - (my + mh)).max(0));
        dx * dx + dy * dy
    };
    match monitors.iter().min_by_key(gap) {
        Some(&(mx, my, mw, mh)) if !visible => {
            (mx.max(x.min(mx + mw - w)), my.max(y.min(my + mh - h)))
        }
        _ => (x, y),
    }
}
