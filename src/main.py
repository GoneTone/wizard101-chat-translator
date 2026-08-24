"""進入點：reader 執行緒（wizwalker 收訊）+ 全域熱鍵 + tkinter 主迴圈（UI 事件經 ui_queue 序列化）。"""
import os
import queue
import sys
import threading
import tkinter as tk
from collections import deque
from datetime import datetime, timedelta, timezone

import keyboard

from src import __version__
from src.composer.input_box import InputBox
from src.composer.paste import type_into_window
from src.config import CONFIG_PATH, app_dir, is_configured, load_config, save_config
from src.reader.mem_reader import GameNotRunning, WizChatReader
from src.reader.overlay import OverlayWindow
from src.translator import (
    Translator, TranslatorBadOutput, TranslatorConfigError, TranslatorOffline,
)
from src.ui.settings import SettingsWindow

BACKOFF_STEPS = [5, 15, 30]  # 翻譯伺服器離線時的重試間隔（秒）
GAME_MISSING_INTERVAL = 5.0  # 找不到遊戲時的重試間隔（秒）
CONFIG_ERROR_INTERVAL = 15.0  # API 設定錯誤時的重試間隔（秒）；使用者修正後自動恢復
BAD_OUTPUT_NOTICE = "⚠  翻譯失敗（模型輸出異常）"  # 譯文被截斷時代替譯文顯示

# overlay 標題列狀態指示：（文字， 顏色）
STATUS = {
    "locating": ("●  連線遊戲中…", "#e0b050"),
    "listening": ("●  監聽中", "#7dc87d"),
    "translating": ("●  翻譯中…", "#6fa8dc"),
    "waiting_game": ("●  等待遊戲中…", "#9a9aa8"),
}


def drain_ui_queue(ui_queue: queue.Queue) -> None:
    """依序取出並執行 ui_queue 裡的回呼；單一回呼拋錯不影響其餘回呼或呼叫端。"""
    while True:
        try:
            callback = ui_queue.get_nowait()
        except queue.Empty:
            break
        try:
            callback()
        except Exception as exc:  # 避免單一 UI 回呼失敗就讓整個 pump 迴圈停擺
            print(f"[ui] callback failed: {exc}", file=sys.stderr)


def reader_loop(cfg: dict, translator: Translator, overlay: OverlayWindow,
                ui_queue: queue.Queue, stop: threading.Event,
                on_input_open=None, on_input_close=None) -> None:
    # 透過 wizwalker 讀遊戲聊天記錄，每輪讀新增的行（依序、含重複）→ 翻譯 → overlay。
    # 翻譯失敗/離線的行留在 pending，下輪從中斷處續翻，不漏不重。
    reader = WizChatReader(game_path=cfg.get("game_path"))
    pending: deque[str] = deque()
    backoff_index = 0
    error_state: str | None = None  # None／"offline"／"config"：供橫幅清除與轉換時記 log
    game_missing = False
    game_input_open = False  # 遊戲聊天輸入框狀態：邊緣觸發自動呼出／收回翻譯輸入
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
        config_error = False

        # 讀取前先亮狀態：首輪要連上遊戲並掛入 hook，期間讓使用者知道在連線
        set_status("listening" if reader.anchored else "locating")
        try:
            pending.extend(reader.read_new())
        except GameNotRunning as exc:
            set_status("waiting_game")
            if not game_missing:
                game_missing = True
                print(f"[reader] game not ready: {exc}", file=sys.stderr)
                ui_queue.put(lambda: overlay.set_error("⚠  遊戲未就緒／連線中斷，等待中…"))
            if game_input_open:
                game_input_open = False  # 遊戲斷線＝輸入框已不存在,同步收回
                if on_input_close is not None:
                    on_input_close()
            stop.wait(GAME_MISSING_INTERVAL)
            continue
        except Exception as exc:  # 收訊偶發錯誤：略過該輪，不讓執行緒死掉
            print(f"[reader] poll skipped: {exc}", file=sys.stderr)
            stop.wait(interval)
            continue

        if game_missing:
            game_missing = False
            print("[reader] game back, resuming", file=sys.stderr)
            ui_queue.put(overlay.clear_error)

        if pending:
            set_status("translating")
        while pending:
            line = pending[0]
            try:
                translated = translator.translate_incoming(line)
            except TranslatorOffline as exc:
                went_offline = True  # line 留在 pending，下輪重試
                offline_exc = exc
                break
            except TranslatorConfigError as exc:
                config_error = True  # 設定錯誤：行留在 pending，等使用者修正後自動恢復
                config_exc = exc
                break
            except TranslatorBadOutput as exc:
                # 譯文被截斷（模型 repetition loop）：temperature=0 下重試必得同一結果，
                # 留在 pending 只會每輪再燒一次生成時間並堵住後續訊息 → 跳過，但仍把
                # 原文送上 overlay，讓使用者看得到這行說了什麼而不是無聲消失。
                print(f"[translate] line dropped, bad model output ({exc}): {line}",
                      file=sys.stderr)
                pending.popleft()
                ui_queue.put(lambda o=line: overlay.add_message(o, BAD_OUTPUT_NOTICE))
                continue
            except Exception as exc:
                # 其他翻譯錯誤（如模型回傳非預期格式）：印出、跳過這行，不讓 reader 執行緒死掉。
                print(f"[translate] line skipped ({exc}): {line}", file=sys.stderr)
                pending.popleft()
                continue
            translated_ok = True
            pending.popleft()
            ui_queue.put(lambda o=line, t=translated: overlay.add_message(o, t))

        if config_error:
            interval = CONFIG_ERROR_INTERVAL
            if error_state != "config":
                print(f"[translate] config error (status={config_exc.status}), "
                      f"waiting for user to fix settings", file=sys.stderr)
            error_state = "config"
            ui_queue.put(lambda: overlay.set_error("⚠  API 設定有誤，請開啟設定（⚙）檢查"))
        elif went_offline:
            interval = BACKOFF_STEPS[min(backoff_index, len(BACKOFF_STEPS) - 1)]
            backoff_index += 1
            if error_state != "offline":
                print(f"[translate] provider offline: {offline_exc}; retrying with backoff "
                      f"(pending={len(pending)})", file=sys.stderr)
            error_state = "offline"
            ui_queue.put(lambda: overlay.set_error("⚠  翻譯伺服器離線，重試中…"))
            # 狀態維持「翻譯中…」：pending 還有行等著重試
        else:
            set_status("listening" if reader.anchored else "locating")
            if translated_ok and error_state is not None:
                # 真的翻譯成功 → 伺服器/設定已恢復，清橫幅並重置退避
                error_state = None
                backoff_index = 0
                print("[translate] recovered, error banner cleared", file=sys.stderr)
                ui_queue.put(overlay.clear_error)

        # 遊戲聊天輸入框開／關的邊緣觸發：開 → 呼出翻譯輸入；關 → 收回
        if on_input_open is not None and cfg.get("auto_show_input", True):
            now_open = reader.input_open()
            if now_open != game_input_open:
                game_input_open = now_open
                print(f"[reader] game chat input {'opened' if now_open else 'closed'}",
                      file=sys.stderr)
                if now_open:
                    on_input_open()
                elif on_input_close is not None:
                    on_input_close()

        stop.wait(interval)

    reader.close()  # 停止：解除 wizwalker hook、關閉連線



SESSION_HEADER_PREFIX = "===== session started "
LOG_RETENTION_DAYS = 7        # app.log 保留天數（以 session 標頭日期判斷）
_LOG_HARD_CAP = 5 * 1024 * 1024   # 異常灌爆保險絲：超過就先砍到尾端再清理
_LOG_KEEP_TAIL = 1 * 1024 * 1024
_HEADER_TS_FORMAT = "%Y-%m-%dT%H:%M:%SZ"


def session_header(now: datetime) -> str:
    """app.log 的啟動分段標頭（UTC＋0）。"""
    return f"{SESSION_HEADER_PREFIX}{now.strftime(_HEADER_TS_FORMAT)} ====="


def trim_log_sessions(text: str, now: datetime) -> str:
    """以 session 標頭把 log 切段，只保留 LOG_RETENTION_DAYS 內開始的段落。
    無標頭的開頭內容（舊格式）與標頭解析失敗的段落一併視為過期丟棄。"""
    cutoff = now - timedelta(days=LOG_RETENTION_DAYS)
    keep: list[str] = []
    keeping = False
    for line in text.splitlines(keepends=True):
        if line.startswith(SESSION_HEADER_PREFIX):
            token = line[len(SESSION_HEADER_PREFIX):].split(" ")[0]
            try:
                ts = datetime.strptime(token, _HEADER_TS_FORMAT).replace(
                    tzinfo=timezone.utc)
            except ValueError:
                keeping = False
            else:
                keeping = ts >= cutoff
        if keeping:
            keep.append(line)
    return "".join(keep)


def _prepare_log(path, now: datetime) -> None:
    """開檔前清理過期段落；檔案異常肥大時先砍到尾端再清理，避免拖慢啟動。"""
    if not path.exists():
        return
    raw = path.read_bytes()
    if len(raw) > _LOG_HARD_CAP:
        raw = raw[-_LOG_KEEP_TAIL:]
    text = raw.decode("utf-8", errors="replace")
    trimmed = trim_log_sessions(text, now)
    if trimmed != text:
        path.write_text(trimmed, encoding="utf-8")


def main() -> None:
    if getattr(sys, "frozen", False):
        # windowed exe 沒有 stdout/stderr（為 None）；全部導到 exe 旁的 app.log，
        # 使用者回報問題時附上此檔即可（附加模式、保留近 LOG_RETENTION_DAYS 天，
        # 每次啟動寫一行 UTC 分段標頭）。
        log_path = app_dir() / "app.log"
        now = datetime.now(timezone.utc)
        try:
            _prepare_log(log_path, now)
            log = open(log_path, "a", encoding="utf-8", buffering=1)
            log.write(session_header(now) + "\n")
        except OSError:
            # exe 所在資料夾沒有寫入權限時開檔會拋例外；windowed 模式沒有主控台可看錯誤，
            # 退回丟棄輸出而非讓程式在使用者看不到任何訊息的情況下當掉。
            log = open(os.devnull, "w", encoding="utf-8")
        sys.stdout = sys.stderr = log

    # 版本先印：使用者回報問題時，app.log 分段標頭後第一行就看得到版本
    print(f"[app] version={__version__}", file=sys.stderr)

    cfg = load_config(CONFIG_PATH)

    root = tk.Tk()
    root.withdraw()

    if not is_configured(cfg):
        from src.ui.wizard import run_wizard
        print("[app] config incomplete, launching first-run wizard", file=sys.stderr)
        if not run_wizard(root, cfg):
            print("[app] wizard cancelled, exiting", file=sys.stderr)
            root.destroy()
            return  # 使用者取消首次設定
        print("[app] wizard completed, config saved", file=sys.stderr)
        save_config(CONFIG_PATH, cfg)

    # 啟動摘要：回報問題時第一眼掌握環境；金鑰絕不記錄
    print(f"[app] startup; frozen={getattr(sys, 'frozen', False)}, "
          f"provider={cfg['api']['provider']}, model={cfg['api']['model']}, "
          f"target_language={cfg['target_language']}, hotkey={cfg['hotkey']}, "
          f"poll_interval={cfg['poll_interval']}", file=sys.stderr)

    translator = Translator(**cfg["api"], target_language=cfg["target_language"])
    ui_queue: queue.Queue = queue.Queue()

    ov = cfg["overlay"]

    def save_geometry(x: int, y: int, w: int, h: int) -> None:
        cfg["overlay"] = {"x": x, "y": y, "width": w, "height": h}
        save_config(CONFIG_PATH, cfg)

    def save_bubble_position(x: int, y: int) -> None:
        cfg["bubble_position"] = {"x": x, "y": y}
        save_config(CONFIG_PATH, cfg)

    overlay = OverlayWindow(
        root,
        x=ov["x"], y=ov["y"], width=ov["width"], height=ov["height"],
        max_messages=cfg["max_messages"],
        fade_seconds=cfg["fade_seconds"],
        on_geometry_change=save_geometry,
        on_settings=lambda: ui_queue.put(lambda: settings.open()),
        on_close=root.quit,  # ✕ 結束 mainloop → 走 finally 的乾淨關閉（停 reader、解 hook）
        bubble_position=cfg["bubble_position"],
        on_bubble_move=save_bubble_position,
        alpha=cfg["overlay_alpha"],
    )

    def on_translated(translated: str, hwnd: int | None) -> None:
        type_into_window(hwnd, translated, delay=cfg["type_delay"])

    def save_input_position(x: int, y: int) -> None:
        cfg["input_position"] = {"x": x, "y": y}
        save_config(CONFIG_PATH, cfg)

    input_box = InputBox(root, translator.translate_outgoing, ui_queue, on_translated,
                         position=cfg["input_position"], on_move=save_input_position)
    hotkey_handle = keyboard.add_hotkey(cfg["hotkey"], lambda: ui_queue.put(input_box.show))

    def apply_settings() -> None:
        nonlocal hotkey_handle
        save_config(CONFIG_PATH, cfg)
        translator.reconfigure(**cfg["api"], target_language=cfg["target_language"])
        keyboard.remove_hotkey(hotkey_handle)
        hotkey_handle = keyboard.add_hotkey(cfg["hotkey"],
                                            lambda: ui_queue.put(input_box.show))
        overlay.set_limits(cfg["max_messages"], cfg["fade_seconds"])
        overlay.set_alpha(cfg["overlay_alpha"])
        print(f"[settings] applied; provider={cfg['api']['provider']}, "
              f"model={cfg['api']['model']}, hotkey={cfg['hotkey']}", file=sys.stderr)

    settings = SettingsWindow(root, cfg, on_save=apply_settings,
                              on_alpha_preview=overlay.set_alpha)

    stop = threading.Event()
    reader_thread = threading.Thread(
        target=reader_loop, args=(cfg, translator, overlay, ui_queue, stop),
        kwargs={"on_input_open": lambda: ui_queue.put(input_box.show),
                "on_input_close": lambda: ui_queue.put(input_box.close)},
        daemon=True)
    reader_thread.start()

    def pump() -> None:
        drain_ui_queue(ui_queue)
        overlay.prune()
        root.after(50, pump)

    print(f"[app] running; hotkey={cfg['hotkey']} opens the input box; quit via the overlay ✕")
    pump()
    try:
        root.mainloop()
    except KeyboardInterrupt:
        pass  # Ctrl+C：安靜結束，不印 traceback
    finally:
        print("[app] shutting down, waiting for reader to unhook", file=sys.stderr)
        stop.set()
        keyboard.unhook_all()
        # 等 reader 執行緒跑完 reader.close()（解除 wizwalker hook、還原遊戲記憶體）再退出；
        # 否則 daemon 執行緒會被直接砍掉，hook 殘留 → 下次掛入 PatternFailed、需重開遊戲。
        reader_thread.join(timeout=8)
        try:
            root.destroy()
        except Exception:
            pass
        print("[app] shutdown complete")


if __name__ == "__main__":
    main()
