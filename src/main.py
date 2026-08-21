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
from src.reader.dedup import LineDeduper
from src.reader.mem_reader import GameNotRunning, read_ordered_tail
from src.reader.overlay import OverlayWindow
from src.translator import Translator

CONFIG_PATH = Path("config.json")
BACKOFF_STEPS = [5, 15, 30]  # 翻譯伺服器離線時的重試間隔(秒)
GAME_MISSING_INTERVAL = 5.0  # 找不到遊戲時的重試間隔(秒)
STARTUP_TAIL_DEFAULT = 0  # 啟動時翻譯幾句既有歷史聊天(預設 0:只翻啟動後的新訊息)


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


def appended_lines(prev_tail: list[str], cur_tail: list[str]) -> list[str] | None:
    """本輪尾段中、上輪尾段最後一行(錨點)之後的行 = 新增訊息。
    錨點找不到(捲太快/剛啟動)回傳 None,由呼叫端改用備援(沒看過的行)。"""
    if not prev_tail:
        return None
    anchor = prev_tail[-1]
    for i in range(len(cur_tail) - 1, -1, -1):
        if cur_tail[i] == anchor:
            return cur_tail[i + 1:]
    return None


def reader_loop(cfg: dict, translator: Translator, overlay: OverlayWindow,
                ui_queue: queue.Queue, stop: threading.Event) -> None:
    # 讀「最完整聊天文件」的有序尾段;新訊息接在結尾。以「結尾新增偵測」判斷新訊息:
    # 上輪尾段的最後一行為錨點,本輪尾段中該錨點之後的行即為新訊息(依序、不冒舊訊息)。
    # deduper(精確、不設上限)為備援:錨點捲出視窗時改用「沒看過的行」。
    deduper = LineDeduper(max_seen=None, similarity=1.0)
    last_tail: list[str] | None = None
    backoff_index = 0
    game_missing = False
    while not stop.is_set():
        interval = cfg["poll_interval"]
        translated_ok = False
        went_offline = False

        try:
            current = read_ordered_tail()
        except GameNotRunning:
            if not game_missing:
                game_missing = True
                ui_queue.put(lambda: overlay.set_error("⚠ 找不到遊戲程序，等待中…"))
            stop.wait(GAME_MISSING_INTERVAL)
            continue
        except Exception as exc:  # 掃描偶發錯誤:略過該輪,不讓執行緒死掉
            print(f"[reader] 略過此輪：{exc}", file=sys.stderr)
            stop.wait(interval)
            continue

        if game_missing:
            game_missing = False
            ui_queue.put(overlay.clear_error)

        if last_tail is None:
            # 啟動:既有歷史全標記看過;startup_tail>0 時翻「有序尾段的最後 N 句」(現在順序正確)。
            deduper.new_lines(current)
            tail = cfg.get("startup_tail", STARTUP_TAIL_DEFAULT)
            lines = current[-tail:] if tail > 0 else []
        else:
            appended = appended_lines(last_tail, current)
            candidates = appended if appended is not None else current
            lines = deduper.new_lines(candidates)  # 過濾看過的 + 標記,備援去重

        for idx, line in enumerate(lines):
            try:
                translated = translator.to_zh(line)
            except httpx.HTTPError:
                # 這行與這批剩下未試的行放回「未見過」;last_tail 不前進,下輪重新偵測重試。
                deduper.forget(lines[idx:])
                went_offline = True
                break
            translated_ok = True
            ui_queue.put(lambda o=line, t=translated: overlay.add_message(o, t))

        # 只有全部翻完(沒離線)才推進錨點;離線時保留舊錨點,讓失敗的行下輪重試。
        if not went_offline:
            last_tail = current

        if went_offline:
            interval = BACKOFF_STEPS[min(backoff_index, len(BACKOFF_STEPS) - 1)]
            backoff_index += 1
            ui_queue.put(lambda: overlay.set_error("⚠ 翻譯伺服器離線，重試中…"))
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
