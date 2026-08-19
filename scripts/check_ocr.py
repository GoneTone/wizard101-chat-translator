"""手動驗證 OCR:讀 config 的 chat_region,截圖並印出辨識行。
用法:遊戲開著、config.json 已有 chat_region,執行 python scripts/check_ocr.py
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from src.config import load_config
from src.reader.capture import grab_region
from src.reader.ocr import recognize_lines

cfg = load_config(Path("config.json"))
if not cfg["chat_region"]:
    sys.exit("config.json 還沒有 chat_region,先跑 python -m src.main --pick-region")
img = grab_region(cfg["chat_region"])
img.save("scripts/last_capture.png")
print("辨識結果:")
for line in recognize_lines(img):
    print(f"  | {line}")
print("(截圖已存 scripts/last_capture.png,可對照)")
