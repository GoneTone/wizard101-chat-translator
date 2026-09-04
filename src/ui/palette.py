"""疊加視窗（overlay 本體、泡泡、細捲軸）共用的深色配色。

顏色集中在這裡是因為它們彼此有約束：BG 同時是本體的 `-transparentcolor`，任何
「要能被點到」的元件都不能用它當底色（見 BG_UPDATE 的說明）。"""

BG = "#101018"
BAR = "#23233a"
GRIP = "#3a3a55"
# 更新橫幅底色：不可沿用 BG——本體用 BG 當 `-transparentcolor`，符合這個顏色的
# 像素連 hit-test 都會被 Windows 跳過（縮放把手也是靠這點才做出「透明卻可拖曳」
# 以外的效果）。橫幅要整列可點、右側 ✕ 也要能點到，底色必須是不透明的顏色。
BG_UPDATE = "#1b2740"
FG_ORIGINAL = "#c0c0cd"
FG_TRANSLATED = "#f2f2f7"
# 原文對譯文的調暗係數：原文是輔助資訊，壓暗到譯文之下讓視線先落在譯文上
DIM_FACTOR = 0.71
FG_PENDING = "#9398a8"  # 佔位中的譯文：比原文更暗，一眼看出這則還沒翻好
FG_ERROR = "#ff5f5f"
# 更新橫幅用連結藍。fields.py 的 LINK_COLOR 是給淺色設定視窗用的，放在 overlay 的
# 深色底（BG）上會暗到看不出是可點的連結，故另取一個亮一階的藍。
FG_UPDATE = "#6fa8ff"
FG_BAR = "#c8c8d8"
# 字幕描邊色：深色輪廓讓文字（與捲軸滑塊）在任何遊戲畫面上都保有對比
OUTLINE = "#0a0a10"
