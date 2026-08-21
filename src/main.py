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
from src.reader.mem_reader import GameNotRunning, read_visible_windows
from src.reader.overlay import OverlayWindow
from src.translator import Translator

CONFIG_PATH = Path("config.json")
BACKOFF_STEPS = [5, 15, 30]  # 翻譯伺服器離線時的重試間隔(秒)
GAME_MISSING_INTERVAL = 5.0  # 找不到遊戲時的重試間隔(秒)
REANCHOR_AFTER = 2           # 連續幾輪所有候選都對不齊後,重新定錨(不翻)


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


def align_append(prev_window: list[str], cur_window: list[str]) -> list[str] | None:
    """視窗滾動對齊:找最短的『prev 去掉前 k 行(非空)』正好是 cur 的前綴,
    回傳 cur 尾端多出來的行(= 新訊息,含重複、依序);對不齊回傳 None。
    過期快照(缺最新訊息的舊內容)必然對不齊 → 呼叫端據此排除它,錨點不被污染。"""
    if not prev_window:
        return None
    for k in range(len(prev_window)):  # 只接受非空重疊,避免「空重疊」誤判整窗皆新
        overlap = prev_window[k:]
        if cur_window[:len(overlap)] == overlap:
            return cur_window[len(overlap):]
    return None


def appended_lines(prev_window: list[str], cur_window: list[str]) -> list[str]:
    """可視視窗結尾新增的行;對不齊時回傳空(不整窗重譯)。"""
    appended = align_append(prev_window, cur_window)
    return [] if appended is None else appended


def choose_window(prev_window: list[str],
                  candidates: list[list[str]]) -> tuple[list[str], list[str]] | None:
    """從候選視窗中挑「與上一輪滾動連續」的那個,回傳 (選中的視窗, 新增的行)。
    過期快照對不齊會被排除;多個對齊者取新增最多的(= 最即時的副本)。
    全部對不齊回傳 None(該輪不翻,由呼叫端決定何時重新定錨)。"""
    best: tuple[list[str], list[str]] | None = None
    for cand in candidates:
        appended = align_append(prev_window, cand)
        if appended is not None and (best is None or len(appended) > len(best[1])):
            best = (cand, appended)
    return best


def reader_loop(cfg: dict, translator: Translator, overlay: OverlayWindow,
                ui_queue: queue.Queue, stop: threading.Event) -> None:
    # 讀「可視聊天視窗」候選,以滾動連續性挑正確副本,翻結尾新增的行 —— 含重複、依序、不去重。
    prev_window: list[str] | None = None
    misaligned_polls = 0
    backoff_index = 0
    game_missing = False
    while not stop.is_set():
        interval = cfg["poll_interval"]
        translated_ok = False
        went_offline = False

        try:
            candidates = read_visible_windows()
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

        chosen: list[str] | None = None
        lines: list[str] = []
        if prev_window is None:
            if candidates:
                prev_window = candidates[0]  # 啟動:只定錨,不翻既有
        else:
            best = choose_window(prev_window, candidates)
            if best is not None:
                chosen, lines = best
                misaligned_polls = 0
            elif candidates:
                # 沒有任何候選能與上一輪對齊(視窗劇烈變動/錨點失效):
                # 該輪不翻;連續對不齊幾輪後重新定錨(仍不翻),避免卡死。
                misaligned_polls += 1
                if misaligned_polls >= REANCHOR_AFTER:
                    prev_window = candidates[0]
                    misaligned_polls = 0

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

        # 推進錨點:只在選到「連續」的視窗時推進 —— 過期快照永遠推不動錨點。
        # 離線且有翻成功→錨點縮到最後一則成功的行,失敗那行下輪重試;
        # 離線且一則都沒成功→維持原錨點,整批下輪重試。
        if not went_offline:
            if chosen is not None:
                prev_window = chosen
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
