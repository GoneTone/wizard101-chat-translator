"""進入點:reader 執行緒 + 全域熱鍵 + tkinter 主迴圈(UI 事件經 ui_queue 序列化)。"""
import argparse
import queue
import sys
import threading
import time
import tkinter as tk
from pathlib import Path

import httpx
import keyboard
import win32gui

from src.composer.input_box import InputBox
from src.composer.paste import paste_into_window, set_clipboard
from src.config import load_config, save_config
from src.reader.capture import grab_region
from src.reader.dedup import LineDeduper
from src.reader.ocr import recognize_lines
from src.reader.overlay import OverlayWindow
from src.region_picker import pick_region
from src.translator import Translator

CONFIG_PATH = Path("config.json")
BACKOFF_STEPS = [5, 15, 30]  # 翻譯伺服器離線時的重試間隔(秒)
GAME_WINDOW_TITLE = "Wizard101"


def game_window_hidden() -> bool:
    """遊戲視窗最小化時回傳 True(暫停截圖)。找不到視窗時不暫停,照常截圖。"""
    hwnd = win32gui.FindWindow(None, GAME_WINDOW_TITLE)
    return bool(hwnd) and bool(win32gui.IsIconic(hwnd))


def reader_loop(cfg: dict, translator: Translator, overlay: OverlayWindow,
                ui_queue: queue.Queue, stop: threading.Event) -> None:
    deduper = LineDeduper()
    backoff_index = 0
    while not stop.is_set():
        interval = cfg["poll_interval"]
        try:
            if not game_window_hidden():
                img = grab_region(cfg["chat_region"])
                for line in deduper.new_lines(recognize_lines(img)):
                    translated = translator.to_zh(line)
                    ui_queue.put(lambda o=line, t=translated: overlay.add_message(o, t))
                if backoff_index:
                    backoff_index = 0
                    ui_queue.put(overlay.clear_error)
        except httpx.HTTPError:
            interval = BACKOFF_STEPS[min(backoff_index, len(BACKOFF_STEPS) - 1)]
            backoff_index += 1
            ui_queue.put(lambda: overlay.set_error("⚠ 翻譯伺服器離線,重試中…"))
        except Exception as exc:  # OCR/截圖偶發錯誤:略過該輪,不讓執行緒死掉
            print(f"[reader] 略過此輪:{exc}", file=sys.stderr)
        stop.wait(interval)


def main() -> None:
    parser = argparse.ArgumentParser(description="Wiz101 聊天翻譯助手")
    parser.add_argument("--pick-region", action="store_true", help="重新框選聊天框區域")
    args = parser.parse_args()

    cfg = load_config(CONFIG_PATH)
    if args.pick_region or not cfg["chat_region"]:
        region = pick_region()
        if region is None:
            sys.exit("已取消框選,離開。")
        cfg["chat_region"] = region
        save_config(CONFIG_PATH, cfg)
        print(f"聊天框區域已存檔:{region}")

    if not cfg["api"]["model"]:
        sys.exit("請先把 config.example.json 複製為 config.json,填入 api.base_url 與 api.model。")

    translator = Translator(**cfg["api"])
    ui_queue: queue.Queue = queue.Queue()

    root = tk.Tk()
    root.withdraw()
    overlay = OverlayWindow(
        root,
        x=cfg["overlay_position"]["x"], y=cfg["overlay_position"]["y"],
        fade_seconds=cfg["fade_seconds"],
    )

    def on_translated(english: str, hwnd: int | None) -> None:
        set_clipboard(english)
        paste_into_window(hwnd)

    input_box = InputBox(root, translator.to_en, ui_queue, on_translated)
    keyboard.add_hotkey(cfg["hotkey"], lambda: ui_queue.put(input_box.show))

    stop = threading.Event()
    threading.Thread(target=reader_loop, args=(cfg, translator, overlay, ui_queue, stop),
                     daemon=True).start()

    def pump() -> None:
        while True:
            try:
                ui_queue.get_nowait()()
            except queue.Empty:
                break
        overlay.prune()
        root.after(50, pump)

    print(f"執行中:熱鍵 {cfg['hotkey']} 呼出輸入框;Ctrl+C 結束。")
    pump()
    try:
        root.mainloop()
    finally:
        stop.set()
        keyboard.unhook_all()


if __name__ == "__main__":
    main()
