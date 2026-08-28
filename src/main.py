"""進入點：reader 執行緒（wizwalker 收訊）+ 全域熱鍵 + tkinter 主迴圈（UI 事件經 ui_queue 序列化）。"""
import itertools
import os
import queue
import sys
import threading
import time
import tkinter as tk

import keyboard

from src import __version__
from src.composer.input_box import InputBox
from src.composer.paste import type_into_window
from src.config import CONFIG_PATH, is_configured, load_config, save_config
from src.context import ChatContext
from src.i18n import current_language, detect_system_language, set_language
from src.logfiles import TimestampedStream, open_session_log
from src.reader.mem_reader import GameNotRunning, WizChatReader
from src.reader.message_log import MessageLog
from src.reader.overlay import OverlayWindow
from src.translation_pool import TranslationPool
from src.translator import Translator
from src.ui.settings import SettingsWindow

GAME_MISSING_INTERVAL = 5.0  # 找不到遊戲時的重試間隔（秒）
# 遊戲聊天輸入框的取樣間隔（秒）：只讀一個可見性旗標，可比 poll_interval 密得多，
# 讓翻譯輸入框幾乎在聊天欄打開的當下就彈出
INPUT_POLL_INTERVAL = 0.05

PENDING_NOTICE = "翻譯中…"                        # 佔位期間顯示於譯文位置
TRANSLATE_FAILED_NOTICE = "⚠  這則訊息翻譯不出來"   # 放棄該行時代替譯文顯示
GAME_MISSING_NOTICE = "⚠  遊戲未就緒／連線中斷，等待中…"
OFFLINE_NOTICE = "⚠  翻譯伺服器離線，重試中…"
CONFIG_ERROR_NOTICE = "⚠  API 設定有誤，請開啟設定（⚙）檢查"

# overlay 標題列狀態指示：（文字， 顏色）
STATUS = {
    "locating": ("●  連線遊戲中…", "#e0b050"),
    "listening": ("●  監聽中", "#7dc87d"),
    "translating": ("●  翻譯中…", "#6fa8dc"),
    "waiting_game": ("●  等待遊戲中…", "#9a9aa8"),
}


def banner_for(game_missing: bool, error_state: str | None) -> str | None:
    """依目前狀況決定該顯示哪一條錯誤橫幅（None＝不顯示）。
    遊戲未就緒優先於翻譯錯誤：連不上遊戲時翻譯狀態已無意義。"""
    if game_missing:
        return GAME_MISSING_NOTICE
    if error_state == "config":
        return CONFIG_ERROR_NOTICE
    if error_state == "offline":
        return OFFLINE_NOTICE
    return None


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


def reader_loop(cfg: dict, overlay: OverlayWindow, ui_queue: queue.Queue,
                stop: threading.Event, context: ChatContext, pool: TranslationPool,
                on_input_open=None, on_input_close=None,
                message_log: MessageLog | None = None) -> None:
    # 讀遊戲聊天記錄 → 依序推進上下文、在 overlay 佔位 → 交給 pool 平行翻譯。
    # 本迴圈不做翻譯，因此單則翻譯卡住不會延誤後續訊息的讀取與顯示。
    reader = WizChatReader(game_path=cfg.get("game_path"), message_log=message_log)
    msg_ids = itertools.count(1)
    game_missing = False
    game_input_open = False
    last_status: str | None = None
    last_banner: str | None = None

    def set_status(key: str) -> None:
        nonlocal last_status
        if key == last_status:
            return
        last_status = key
        text, color = STATUS[key]
        ui_queue.put(lambda: overlay.set_status(text, color))

    def check_input() -> None:
        """遊戲聊天輸入框開／關的邊緣觸發：開 → 呼出翻譯輸入；關 → 收回。"""
        nonlocal game_input_open
        if on_input_open is None or not cfg.get("auto_show_input", True):
            return
        now_open = reader.input_open()
        if now_open == game_input_open:
            return
        game_input_open = now_open
        print(f"[reader] game chat input {'opened' if now_open else 'closed'}",
              file=sys.stderr)
        if now_open:
            on_input_open()
        elif on_input_close is not None:
            on_input_close()

    def wait_watching_input(seconds: float) -> None:
        """等待下一輪讀取，期間以 INPUT_POLL_INTERVAL 持續取樣輸入框狀態。

        input_open() 只讀一個已快取節點的可見性旗標（實測 <0.1ms），可以用遠高於
        poll_interval 的頻率取樣；讀聊天記錄則貴得多（實測約 10ms），維持原本的節奏。
        取樣不另開執行緒——WizChatReader 內部跑自己的 asyncio loop，跨執行緒併發呼叫
        會踩到彼此。"""
        deadline = time.monotonic() + seconds
        while not stop.is_set():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return
            stop.wait(min(INPUT_POLL_INTERVAL, remaining))
            check_input()

    def set_banner(text: str | None) -> None:
        nonlocal last_banner
        if text == last_banner:
            return
        last_banner = text
        if text is None:
            ui_queue.put(overlay.clear_error)
        else:
            ui_queue.put(lambda t=text: overlay.set_error(t))

    while not stop.is_set():
        set_status("listening" if reader.anchored else "locating")
        try:
            new_lines = reader.read_new()
        except GameNotRunning as exc:
            set_status("waiting_game")
            if not game_missing:
                game_missing = True
                print(f"[reader] game not ready: {exc}", file=sys.stderr)
            set_banner(banner_for(game_missing, pool.error_state))
            if game_input_open:
                game_input_open = False  # 遊戲斷線＝輸入框已不存在，同步收回
                if on_input_close is not None:
                    on_input_close()
            stop.wait(GAME_MISSING_INTERVAL)
            continue
        except Exception as exc:  # 收訊偶發錯誤：略過該輪，不讓執行緒死掉
            print(f"[reader] poll skipped: {exc}", file=sys.stderr)
            stop.wait(cfg["poll_interval"])
            continue

        if game_missing:
            game_missing = False
            print("[reader] game back, resuming", file=sys.stderr)

        for line in new_lines:
            ctx = context.snapshot()   # 該行之前的行；提交後即固定，重試不漂移
            context.push(line.text)
            msg_id = next(msg_ids)
            ui_queue.put(lambda o=line.text, c=line.color, m=msg_id:
                         overlay.add_message(o, PENDING_NOTICE, msg_id=m, pending=True,
                                             color=c))
            pool.submit(line.text, ctx, msg_id)

        set_banner(banner_for(game_missing, pool.error_state))
        if pool.in_flight:
            set_status("translating")
        else:
            set_status("listening" if reader.anchored else "locating")

        check_input()
        wait_watching_input(cfg["poll_interval"])

    reader.close()  # 停止：解除 wizwalker hook、關閉連線



def main() -> None:
    if getattr(sys, "frozen", False):
        # windowed exe 沒有 stdout/stderr（為 None）；全部導到 exe 旁的 app.log，
        # 使用者回報問題時附上此檔即可（附加模式、保留近 7 天，每次啟動寫一行分段標頭）。
        sys.stdout = sys.stderr = TimestampedStream(open_session_log("app.log"))
    else:
        # 開發模式輸出到主控台，同樣補時戳，才對得上 messages.log 的時間軸。
        # 主控台編碼常是 cp950（非 UTF-8），UI 文字裡的 ✕ 之類字元會讓 print 直接
        # 拋 UnicodeEncodeError 把程式帶掉，故先放寬成無法編碼就替換。
        for stream in (sys.stdout, sys.stderr):
            if hasattr(stream, "reconfigure"):
                stream.reconfigure(errors="replace")
        sys.stdout = TimestampedStream(sys.stdout)
        sys.stderr = TimestampedStream(sys.stderr)

    # 版本先印：使用者回報問題時，app.log 分段標頭後第一行就看得到版本
    print(f"[app] version={__version__}", file=sys.stderr)

    # 收訊原始內容另存一份（不清理、不過濾），訊息類問題直接比對這份
    message_log = MessageLog(TimestampedStream(open_session_log("messages.log")))

    cfg = load_config(CONFIG_PATH)

    # 介面語言要在建立任何視窗之前決定：文案與字型都由它決定。
    set_language(cfg["ui_language"] or detect_system_language())

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
          f"ui_language={cfg['ui_language']} (active={current_language()}), "
          f"provider={cfg['api']['provider']}, model={cfg['api']['model']}, "
          f"target_language={cfg['target_language']}, hotkey={cfg['hotkey']}, "
          f"poll_interval={cfg['poll_interval']}, "
          f"parallel={cfg['max_parallel_translations']}", file=sys.stderr)

    translator = Translator(**cfg["api"], target_language=cfg["target_language"])
    context = ChatContext()
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

    pool = TranslationPool(
        translator=translator,
        on_result=lambda mid, text, failed: ui_queue.put(
            lambda: overlay.update_message(mid, text, failed=failed)),
        workers=cfg["max_parallel_translations"],
        failed_notice=TRANSLATE_FAILED_NOTICE)

    def on_translated(translated: str, hwnd: int | None) -> None:
        type_into_window(hwnd, translated, delay=cfg["type_delay"])

    def save_input_geometry(x: int, y: int, width: int) -> None:
        cfg["input_position"] = {"x": x, "y": y}
        cfg["input_width"] = width
        save_config(CONFIG_PATH, cfg)

    input_box = InputBox(root, lambda text: translator.translate_outgoing(
        text, context.snapshot()), ui_queue, on_translated,
        position=cfg["input_position"], width=cfg["input_width"],
        on_geometry_change=save_input_geometry)
    hotkey_handle = keyboard.add_hotkey(cfg["hotkey"], lambda: ui_queue.put(input_box.show))

    def apply_settings() -> None:
        nonlocal hotkey_handle
        save_config(CONFIG_PATH, cfg)
        translator.reconfigure(**cfg["api"], target_language=cfg["target_language"])
        pool.resize(cfg["max_parallel_translations"])
        keyboard.remove_hotkey(hotkey_handle)
        hotkey_handle = keyboard.add_hotkey(cfg["hotkey"],
                                            lambda: ui_queue.put(input_box.show))
        overlay.set_limits(cfg["max_messages"], cfg["fade_seconds"])
        overlay.set_alpha(cfg["overlay_alpha"])
        print(f"[settings] applied; provider={cfg['api']['provider']}, "
              f"model={cfg['api']['model']}, hotkey={cfg['hotkey']}, "
              f"parallel={cfg['max_parallel_translations']}", file=sys.stderr)

    settings = SettingsWindow(root, cfg, on_save=apply_settings,
                              on_alpha_preview=overlay.set_alpha)

    stop = threading.Event()
    reader_thread = threading.Thread(
        target=reader_loop, args=(cfg, overlay, ui_queue, stop, context, pool),
        kwargs={"on_input_open": lambda: ui_queue.put(input_box.show),
                "on_input_close": lambda: ui_queue.put(input_box.close),
                "message_log": message_log},
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
        pool.shutdown()
        keyboard.unhook_all()
        # 等 reader 執行緒跑完 reader.close()（解除 wizwalker hook、還原遊戲記憶體）再退出；
        # 否則 daemon 執行緒會被直接砍掉，hook 殘留 → 下次掛入 PatternFailed、需重開遊戲。
        reader_thread.join(timeout=8)
        try:
            root.destroy()
        except Exception:
            pass
        print("[app] shutdown complete")
        # 翻譯 worker 執行緒非 daemon，逾時仍卡在 HTTP 請求中的話（最長 _TIMEOUT=60 秒）
        # 一般 return 會讓直譯器在 concurrent.futures.thread._python_exit 卡住等它們
        # join，使用者看到視窗已關、程式卻在工作管理員裡多留最多 60 秒——像當掉一樣。
        # 該還原的都還原了（reader 執行緒已 join、hook 已解除、log 已寫完且線緩衝），
        # 故直接砍行程；日後若想「修」回乾淨 return，請先確認上述 60 秒卡住已消失。
        os._exit(0)


if __name__ == "__main__":
    main()
