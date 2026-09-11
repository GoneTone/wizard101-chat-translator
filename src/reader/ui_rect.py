"""遊戲 UI 控件的邏輯座標 → 遊戲視窗 client 像素座標（純函式，不碰 wizwalker）。

遊戲每個控件的矩形都相對於父控件，根視窗則是一塊固定的邏輯畫布（實測 1370×770），
再按 ui_scale 放大到實際 client 尺寸。wizwalker 內建的換算要多掛 RenderContextHook
才讀得到 ui_scale；這裡改用「client 尺寸 ÷ 根視窗尺寸」推算（實機驗證寬高兩邊推出的
比例一致、誤差不到 1 px），維持只掛 root_window hook。
"""


def client_rect(offsets: list[tuple[int, int]], size: tuple[int, int],
                root_size: tuple[int, int],
                client_size: tuple[int, int]) -> tuple[int, int, int, int]:
    """控件在遊戲 client 內的像素矩形 (x, y, w, h)。

    offsets＝控件本身與各層父控件的左上角（相對於各自的父），size＝控件邏輯尺寸，
    root_size＝根視窗邏輯尺寸，client_size＝遊戲視窗 client 像素尺寸。
    根視窗尺寸為 0（尚未渲染）丟 ValueError。
    四捨五入而非截尾：截尾會讓右緣少 1 px（實機截圖與遊戲畫出的邊框逐像素比對）。"""
    root_w, root_h = root_size
    if root_w <= 0 or root_h <= 0:
        raise ValueError(f"degenerate root window size {root_size}")
    scale = client_size[0] / root_w
    x = sum(dx for dx, _ in offsets)
    y = sum(dy for _, dy in offsets)
    return (round(x * scale), round(y * scale), round(size[0] * scale), round(size[1] * scale))
