"""進入點:reader 執行緒(wizwalker 收訊)+ 全域熱鍵 + tkinter 主迴圈(UI 事件經 ui_queue 序列化)。"""
import queue
import sys
import threading
import tkinter as tk
from collections import deque
from pathlib import Path

import httpx
import keyboard

from src.composer.input_box import InputBox
from src.composer.paste import type_into_window
from src.config import load_config, save_config
from src.reader.mem_reader import GameNotRunning, WizChatReader
from src.reader.overlay import OverlayWindow
from src.translator import Translator

CONFIG_PATH = Path("config.json")
BACKOFF_STEPS = [5, 15, 30]  # 翻譯伺服器離線時的重試間隔(秒)
GAME_MISSING_INTERVAL = 5.0  # 找不到遊戲時的重試間隔(秒)

# overlay 標題列狀態指示:(文字, 顏色)
STATUS = {
    "locating": ("●  連線遊戲中…", "#e0b050"),
    "listening": ("●  監聽中", "#7dc87d"),
    "translating": ("●  翻譯中…", "#6fa8dc"),
    "waiting_game": ("●  等待遊戲中…", "#9a9aa8"),
}


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


def reader_loop(cfg: dict, translator: Translator, overlay: OverlayWindow,
                ui_queue: queue.Queue, stop: threading.Event) -> None:
    # 透過 wizwalker 讀遊戲聊天記錄,每輪讀新增的行(依序、含重複)→ 翻譯 → overlay。
    # 翻譯失敗/離線的行留在 pending,下輪從中斷處續翻,不漏不重。
    reader = WizChatReader(game_path=cfg.get("game_path"))
    pending: deque[str] = deque()
    backoff_index = 0
    game_missing = False
    last_status: str | None = None

    def set_status(key: str) -> None:
        nonlocal last_status
        if key == last_status:
            return
        last_status = key
        text, color = STATUS[key]
        ui_queue.put(lambda: overlay.set_status(text, color))

    while not stop.is_set():
        interval = cfg["poll_interval"]
        translated_ok = False
        went_offline = False

        # 讀取前先亮狀態:首輪要連上遊戲並掛入 hook,期間讓使用者知道在連線
        set_status("listening" if reader.anchored else "locating")
        try:
            pending.extend(reader.read_new())
        except GameNotRunning:
            set_status("waiting_game")
            if not game_missing:
                game_missing = True
                ui_queue.put(lambda: overlay.set_error("⚠  遊戲未就緒／連線中斷，等待中…"))
            stop.wait(GAME_MISSING_INTERVAL)
            continue
        except Exception as exc:  # 收訊偶發錯誤:略過該輪,不讓執行緒死掉
            print(f"[reader] 略過此輪：{exc}", file=sys.stderr)
            stop.wait(interval)
            continue

        if game_missing:
            game_missing = False
            ui_queue.put(overlay.clear_error)

        if pending:
            set_status("translating")
        while pending:
            line = pending[0]
            try:
                translated = translator.to_zh(line)
            except httpx.HTTPError:
                went_offline = True  # line 留在 pending,下輪重試
                break
            except Exception as exc:
                # 其他翻譯錯誤(如模型回傳非預期格式):印出、跳過這行,不讓 reader 執行緒死掉。
                print(f"[translate] 略過此行（{exc}）：{line}", file=sys.stderr)
                pending.popleft()
                continue
            translated_ok = True
            pending.popleft()
            ui_queue.put(lambda o=line, t=translated: overlay.add_message(o, t))

        if went_offline:
            interval = BACKOFF_STEPS[min(backoff_index, len(BACKOFF_STEPS) - 1)]
            backoff_index += 1
            ui_queue.put(lambda: overlay.set_error("⚠  翻譯伺服器離線，重試中…"))
            # 狀態維持「翻譯中…」:pending 還有行等著重試
        else:
            set_status("listening" if reader.anchored else "locating")
            if translated_ok and backoff_index:
                # 只有真的翻譯成功過,才代表伺服器已恢復,清除離線橫幅並重置退避。
                backoff_index = 0
                ui_queue.put(overlay.clear_error)

        stop.wait(interval)

    reader.close()  # 停止:解除 wizwalker hook、關閉連線



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
    reader_thread = threading.Thread(
        target=reader_loop, args=(cfg, translator, overlay, ui_queue, stop), daemon=True)
    reader_thread.start()

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
        # 等 reader 執行緒跑完 reader.close()(解除 wizwalker hook、還原遊戲記憶體)再退出;
        # 否則 daemon 執行緒會被直接砍掉,hook 殘留 → 下次掛入 PatternFailed、需重開遊戲。
        reader_thread.join(timeout=8)
        try:
            root.destroy()
        except Exception:
            pass
        print("已結束。")


if __name__ == "__main__":
    main()
