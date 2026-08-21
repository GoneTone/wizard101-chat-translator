"""進入點:reader 執行緒(記憶體收訊)+ 全域熱鍵 + tkinter 主迴圈(UI 事件經 ui_queue 序列化)。"""
import queue
import sys
import threading
import tkinter as tk
from pathlib import Path

import httpx
import keyboard

from src.composer.input_box import InputBox
from src.composer.paste import type_into_window
from src.config import load_config, save_config
from src.reader.mem_reader import GameNotRunning, read_visible_chat
from src.reader.overlay import OverlayWindow
from src.translator import Translator

CONFIG_PATH = Path("config.json")
BACKOFF_STEPS = [5, 15, 30]  # 翻譯伺服器離線時的重試間隔(秒)
GAME_MISSING_INTERVAL = 5.0  # 找不到遊戲時的重試間隔(秒)


def drain_ui_queue(ui_queue: queue.Queue) -> None:
    """依序取出並執行 ui_queue 裡的回呼;單一回呼拋錯不影響其餘回呼或呼叫端。"""
    while True:
        try:
            callback = ui_queue.get_nowait()
        except queue.Empty:
            break
        try:
            callback()
        except Exception as exc:  # 避免單一 UI 回呼失敗就讓整個 pump 迴圈停擺
            print(f"[ui] 回呼失敗：{exc}", file=sys.stderr)


def appended_lines(prev_window: list[str], cur_window: list[str]) -> list[str]:
    """可視視窗滾動後、結尾新增的行(對應遊戲聊天室新出現的訊息,含重複、依序)。
    視窗會從前端捨棄舊行、後端接上新行;找最小 k 使『prev 去掉前 k 行』正好是 cur 的前綴
    (對齊滾動),回傳 cur 尾端多出來的行。完全對不上(一次捲太多)時整個 cur 當作全新。"""
    for k in range(len(prev_window) + 1):
        overlap = prev_window[k:]
        if cur_window[:len(overlap)] == overlap:
            return cur_window[len(overlap):]
    return list(cur_window)


def reader_loop(cfg: dict, translator: Translator, overlay: OverlayWindow,
                ui_queue: queue.Queue, stop: threading.Event) -> None:
    # 讀「可視聊天視窗」(對應遊戲聊天室),翻它結尾新增的行 —— 含重複、依序、不去重。
    prev_window: list[str] | None = None
    backoff_index = 0
    game_missing = False
    while not stop.is_set():
        interval = cfg["poll_interval"]
        translated_ok = False
        went_offline = False

        try:
            window = read_visible_chat()
        except GameNotRunning:
            if not game_missing:
                game_missing = True
                ui_queue.put(lambda: overlay.set_error("⚠ 找不到遊戲程序,等待中…"))
            stop.wait(GAME_MISSING_INTERVAL)
            continue
        except Exception as exc:  # 掃描偶發錯誤:略過該輪,不讓執行緒死掉
            print(f"[reader] 略過此輪:{exc}", file=sys.stderr)
            stop.wait(interval)
            continue

        if game_missing:
            game_missing = False
            ui_queue.put(overlay.clear_error)

        if prev_window is None:
            lines = []  # 啟動:只記錄可視內容,不翻既有
        else:
            lines = appended_lines(prev_window, window)  # 結尾新增的行

        translated_count = 0
        for idx, line in enumerate(lines):
            try:
                translated = translator.to_zh(line)
            except httpx.HTTPError:
                went_offline = True
                break
            except Exception as exc:
                # 其他翻譯錯誤(如模型回傳非預期格式):印出、跳過這行,不讓 reader 執行緒死掉。
                print(f"[translate] 略過此行({exc}):{line}", file=sys.stderr)
                translated_count += 1
                continue
            translated_ok = True
            translated_count += 1
            ui_queue.put(lambda o=line, t=translated: overlay.add_message(o, t))

        # 推進錨點(appended_lines 只看 prev_window[-1]):
        # 正常→推進到本輪視窗;離線且有翻成功→錨點推進到最後一則成功的,失敗那行下輪重試;
        # 離線且一則都沒成功→維持原錨點,整批下輪重試。
        if not went_offline:
            prev_window = window
        elif translated_count > 0:
            prev_window = [lines[translated_count - 1]]

        if went_offline:
            interval = BACKOFF_STEPS[min(backoff_index, len(BACKOFF_STEPS) - 1)]
            backoff_index += 1
            ui_queue.put(lambda: overlay.set_error("⚠ 翻譯伺服器離線,重試中…"))
        elif translated_ok and backoff_index:
            # 只有真的翻譯成功過,才代表伺服器已恢復,清除離線橫幅並重置退避。
            backoff_index = 0
            ui_queue.put(overlay.clear_error)

        stop.wait(interval)



def main() -> None:
    cfg = load_config(CONFIG_PATH)
    if not cfg["api"]["model"]:
        sys.exit("請編輯 config.json 填入 api.base_url 與 api.model（格式參考 config.example.json）。")

    translator = Translator(**cfg["api"])
    ui_queue: queue.Queue = queue.Queue()

    root = tk.Tk()
    root.withdraw()

    ov = cfg["overlay"]

    def save_geometry(x: int, y: int, w: int, h: int) -> None:
        cfg["overlay"] = {"x": x, "y": y, "width": w, "height": h}
        save_config(CONFIG_PATH, cfg)

    overlay = OverlayWindow(
        root,
        x=ov["x"], y=ov["y"], width=ov["width"], height=ov["height"],
        max_messages=cfg["max_messages"],
        fade_seconds=cfg["fade_seconds"],
        on_geometry_change=save_geometry,
    )

    def on_translated(english: str, hwnd: int | None) -> None:
        type_into_window(hwnd, english, delay=cfg["type_delay"])

    def save_input_position(x: int, y: int) -> None:
        cfg["input_position"] = {"x": x, "y": y}
        save_config(CONFIG_PATH, cfg)

    input_box = InputBox(root, translator.to_en, ui_queue, on_translated,
                         position=cfg["input_position"], on_move=save_input_position)
    keyboard.add_hotkey(cfg["hotkey"], lambda: ui_queue.put(input_box.show))

    stop = threading.Event()
    threading.Thread(target=reader_loop, args=(cfg, translator, overlay, ui_queue, stop),
                     daemon=True).start()

    def pump() -> None:
        drain_ui_queue(ui_queue)
        overlay.prune()
        root.after(50, pump)

    print(f"執行中：熱鍵 {cfg['hotkey']} 呼出輸入框；Ctrl+C 結束。")
    pump()
    try:
        root.mainloop()
    except KeyboardInterrupt:
        pass  # Ctrl+C:安靜結束,不印 traceback
    finally:
        stop.set()
        keyboard.unhook_all()
        try:
            root.destroy()
        except Exception:
            pass
        print("已結束。")


if __name__ == "__main__":
    main()
