"""手動驗證 overlay:應看到置頂半透明視窗,訊息逐則出現,且滑鼠點擊會穿透到底下視窗。"""
import sys
import tkinter as tk
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from src.reader.overlay import OverlayWindow

root = tk.Tk()
root.withdraw()
ov = OverlayWindow(root, x=100, y=100, fade_seconds=15)

samples = [
    ("Wolf: anyone need help with Rattlebones?", "Wolf:有人需要幫忙打 Rattlebones 嗎?"),
    ("Amber: selling seeds at bazaar", "Amber:我在 bazaar 賣種子"),
    ("Duncan: brb 5 min", "Duncan:離開一下,5 分鐘回來"),
]

def feed(i=0):
    if i < len(samples):
        ov.add_message(*samples[i])
        root.after(2000, feed, i + 1)

def tick():
    ov.prune()
    root.after(1000, tick)

feed()
tick()
root.after(30000, root.destroy)  # 30 秒後自動關閉
root.mainloop()
