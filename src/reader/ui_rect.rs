//! 遊戲 UI 控件的邏輯座標 → 遊戲視窗 client 像素座標（純函式）。
//!
//! 根視窗是固定的邏輯畫布，client 尺寸 ÷ 根視窗尺寸即縮放比例；寬高兩邊推出的比例一致，故兩軸都用寬度比例。

/// 控件在遊戲 client 內的像素矩形 `(x, y, w, h)`；根視窗尺寸為 0（尚未渲染）回 None。
///
/// `offsets` 是控件本身與各層父控件的左上角（相對於各自的父），`size` 是控件邏輯尺寸。
/// 四捨五入（同 Python `round`，.5 取偶數）而非截尾：截尾會讓右緣少 1 px。
pub fn client_rect(
    offsets: &[(i32, i32)],
    size: (i32, i32),
    root_size: (i32, i32),
    client_size: (i32, i32),
) -> Option<(i32, i32, i32, i32)> {
    let (root_w, root_h) = root_size;
    if root_w <= 0 || root_h <= 0 {
        return None;
    }
    let scale = f64::from(client_size.0) / f64::from(root_w);
    let x: i64 = offsets.iter().map(|&(dx, _)| i64::from(dx)).sum();
    let y: i64 = offsets.iter().map(|&(_, dy)| i64::from(dy)).sum();
    let px = |v: i64| (v as f64 * scale).round_ties_even() as i32;
    Some((px(x), px(y), px(size.0.into()), px(size.1.into())))
}
