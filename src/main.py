"""進入點：reader 執行緒（wizwalker 收訊）+ 全域熱鍵 + tkinter 主迴圈（UI 事件經 ui_queue 序列化）。"""
import ctypes
import itertools
import os
import queue
import sys
import threading
import time
import tkinter as tk

import keyboard
import win32api
import win32con
import win32gui
import win32event
import winerror

from src import __version__
from src.composer.input_box import InputBox
from src.composer.paste import type_into_window
from src.config import (CONFIG_PATH, active_api, app_name, is_configured,
                        load_config, save_config)
from src.context import ChatContext
from src.i18n import (current_language, detect_system_language, language_name,
                      set_language, t)
from src.logfiles import TimestampedStream, open_session_log
from src.reader.mem_reader import (
    GameAccessDenied, GameNotRunning, GameVersionMismatch, WizChatReader,
)
from src.reader.message_log import MessageLog
from src.reader.overlay import OverlayWindow
from src.resources import icon_path
from src.translation_pool import TranslationPool
from src.translator import Translator
from src.ui.settings import SettingsWindow
from src.updater import check_for_update

GAME_MISSING_INTERVAL = 5.0  # 找不到遊戲時的重試間隔（秒）
# 單一實例的 mutex 名稱。跑第二份會讓兩邊搶著對遊戲掛 wizwalker hook，
# 也會同時寫同一份 config.json 與 log，因此直接擋掉。
SINGLE_INSTANCE_MUTEX = "wizard101-chat-translator.single-instance"

# 遊戲聊天輸入框的取樣間隔（秒）：只讀一個可見性旗標，可比 poll_interval 密得多，
# 讓翻譯輸入框幾乎在聊天欄打開的當下就彈出
INPUT_POLL_INTERVAL = 0.05


def is_elevated() -> bool:
    """本程序是否以系統管理員權限執行。掛入權限問題的診斷欄位，查不到當作否。"""
    try:
        import ctypes
        return bool(ctypes.windll.shell32.IsUserAnAdmin())
    except Exception:
        return False


def banner_for(game_issue: str | None, error_state: str | None) -> str | None:
    """依目前狀況決定該顯示哪一條錯誤橫幅的文案 key（None＝不顯示）。
    game_issue 是遊戲端問題的文案 key（None＝遊戲正常），優先於翻譯錯誤：
    連不上遊戲時翻譯狀態已無意義。"""
    if game_issue:
        return game_issue
    if error_state == "config":
        return "notice.config_error"
    if error_state == "offline":
        return "notice.offline"
    return None


def bootstrap_language(cfg: dict, config_existed: bool, detect=detect_system_language) -> str:
    """決定啟動時要套用的介面語言碼，並在真正首次執行時就地補上 target_language 預設值。

    是否為首次執行由呼叫端傳入的 `config_existed`（啟動時 config.json 是否已存在）判斷，
    不能看 `cfg["ui_language"]` 是否為 None：pre-i18n 版本寫出的舊 config 一樣會被
    `load_config()`／`_merge` 回填成 `ui_language: None`（見 `tests/test_config.py`），
    若沿用舊判斷式，既有使用者升級後會被誤判成首次執行，導致他們自己選過的
    target_language 被系統偵測值悄悄覆蓋，還會在下一次任何 `save_config`（例如只是拖動
    視窗）時永久寫死，之後每次啟動都重演，手動改 config.json 也救不回來。

    `config_existed` 為 False（真正首次執行）時：介面語言依系統偵測，翻譯目標語言也
    跟著它走，否則非 zh-TW 系統會在精靈第三步看到不相關的「繁體中文（台灣）」預設值。
    `config_existed` 為 True 時：`ui_language` 若是真實語言碼就直接採用、不呼叫
    `detect`（避免每次啟動都做多餘的系統查詢）；若仍是 None（沿用自舊版設定檔），
    介面語言照樣呼叫 `detect` 決定，但不動 target_language，尊重使用者原本的選擇。"""
    if cfg["ui_language"] is None:
        detected = detect()
        if not config_existed:
            cfg["target_language"] = language_name(detected)
        return detected
    return cfg["ui_language"]


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


def announce_update(ui_queue: queue.Queue, overlay, checker=check_for_update) -> None:
    """檢查更新，有新版就把橫幅回呼排進 ui_queue（供背景執行緒呼叫）。

    任何失敗都只留 log：更新檢查是附加功能，不能影響啟動與收訊。`checker` 可注入
    是為了測試，正式路徑用預設的 check_for_update。"""
    try:
        release = checker()
    except Exception as exc:
        print(f"[update] check failed: {exc}", file=sys.stderr)
        return
    if release is None:
        return
    ui_queue.put(lambda: overlay.set_update(release))


def reader_loop(cfg: dict, overlay: OverlayWindow, ui_queue: queue.Queue,
                stop: threading.Event, context: ChatContext, pool: TranslationPool,
                on_input_open=None, on_input_close=None,
                message_log: MessageLog | None = None) -> None:
    # 讀遊戲聊天記錄 → 依序推進上下文、在 overlay 佔位 → 交給 pool 平行翻譯。
    # 本迴圈不做翻譯，因此單則翻譯卡住不會延誤後續訊息的讀取與顯示。
    reader = WizChatReader(game_path=cfg.get("game_path"), message_log=message_log)
    msg_ids = itertools.count(1)
    game_issue: str | None = None  # 遊戲端問題的橫幅文案 key（None＝遊戲正常）
    game_input_open = False
    last_status: str | None = None
    last_banner: str | None = None

    def set_status(state: str) -> None:
        nonlocal last_status
        if state == last_status:
            return
        last_status = state
        ui_queue.put(lambda s=state: overlay.set_status(s))

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

    def set_banner(key: str | None) -> None:
        nonlocal last_banner
        if key == last_banner:
            return
        last_banner = key
        if key is None:
            ui_queue.put(overlay.clear_error)
        else:
            ui_queue.put(lambda k=key: overlay.set_error(k))

    while not stop.is_set():
        set_status("listening" if reader.anchored else "locating")
        try:
            new_lines = reader.read_new()
        except GameNotRunning as exc:
            if isinstance(exc, GameAccessDenied):
                status, issue = "access_denied", "notice.access_denied"
            elif isinstance(exc, GameVersionMismatch):
                status, issue = "version_mismatch", "notice.version_mismatch"
            else:
                status, issue = "waiting_game", "notice.game_missing"
            set_status(status)
            if issue != game_issue:  # 只在原因改變時記錄，否則每輪重試都灌一行
                print(f"[reader] game not ready: {exc}", file=sys.stderr)
            game_issue = issue
            set_banner(banner_for(game_issue, pool.error_state))
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

        if game_issue:
            game_issue = None
            print("[reader] game back, resuming", file=sys.stderr)

        for line in new_lines:
            ctx = context.snapshot()   # 該行之前的行；提交後即固定，重試不漂移
            context.push(line.text)
            msg_id = next(msg_ids)
            ui_queue.put(lambda o=line.text, c=line.color, m=msg_id:
                         overlay.add_message(o, t("notice.pending"), msg_id=m,
                                             pending=True, color=c))
            pool.submit(line.text, ctx, msg_id)

        set_banner(banner_for(game_issue, pool.error_state))
        if pool.in_flight:
            set_status("translating")
        else:
            set_status("listening" if reader.anchored else "locating")

        check_input()
        wait_watching_input(cfg["poll_interval"])

    reader.close()  # 停止：解除 wizwalker hook、關閉連線



def acquire_single_instance(name: str = SINGLE_INSTANCE_MUTEX) -> int | None:
    """搶下單一實例的 mutex；已經有一份在跑時回 None。

    呼叫端必須留住回傳的 handle 直到程序結束：mutex 隨 handle 關閉而釋放，
    handle 一被回收就等於放行下一份實例。name 可覆寫是為了讓測試各用各的名稱，
    不會被使用者正在執行的本尊卡住。"""
    handle = win32event.CreateMutex(None, False, name)
    if win32api.GetLastError() == winerror.ERROR_ALREADY_EXISTS:
        return None
    return handle


def focus_running_instance(title: str) -> bool:
    """把既有實例的視窗帶到前景；找不到視窗或被系統擋下時回 False。

    Windows 的前景鎖會擋掉背景程序的 SetForegroundWindow，這時系統改成閃工作列
    按鈕——使用者仍看得到回應，所以失敗只記錄、不當成錯誤。"""
    found = []

    def collect(hwnd, _):
        try:
            if win32gui.IsWindowVisible(hwnd) and win32gui.GetWindowText(hwnd) == title:
                found.append(hwnd)
        except Exception:
            pass   # 列舉途中單一視窗查詢失敗不該中斷整輪掃描
        return True

    try:
        win32gui.EnumWindows(collect, None)
    except Exception as exc:
        print(f"[app] window scan failed: {exc}", file=sys.stderr)
        return False
    if not found:
        print(f"[app] no window titled {title!r} to focus", file=sys.stderr)
        return False
    hwnd = found[0]
    try:
        win32gui.ShowWindow(hwnd, win32con.SW_RESTORE)
        win32gui.SetForegroundWindow(hwnd)
    except Exception as exc:
        print(f"[app] focus existing instance failed: hwnd={hwnd:#x} error={exc}",
              file=sys.stderr)
        return False
    return True


def apply_window_icon(root: tk.Tk) -> int | None:
    """把應用程式 icon 裝到視窗類別上，回傳裝上去的 HICON（失敗回 None）。

    不用 tkinter 的 `iconbitmap`：它在 Windows 上挑不對 ICO 的 frame，實測掛出來的
    32x32 是被放大裁切過的糊圖（徽章切在邊緣、101 缺一半）。改用 LoadImage 指定
    尺寸載入——它會挑最接近的原生 frame——再寫進視窗類別，之後建立的每個 Toplevel
    都自動沿用，不必逐一設定。

    類別得透過一個 TkTopLevel 視窗才設得到：withdraw 的 root 在 Windows 上是
    TkChild，跟實際顯示的視窗不同類別，所以這裡開一個隱藏的 Toplevel 當跳板
    （隱藏狀態仍屬 TkTopLevel，不會閃畫面）。

    小圖示（GCL_HICONSM）設不進去（實測寫入回報成功卻讀不回來），但工作列按鈕在
    Windows 11 是看 exe 的圖示、不看視窗 icon，所以不影響——真正要顧的是 exe 資源
    裡的尺寸要齊全（見 src/assets/icon.ico）。

    icon 是可有可無的裝飾，掛不上去只留 log，不能因此擋掉啟動。"""
    path = icon_path()
    probe = None
    try:
        probe = tk.Toplevel(root)
        probe.withdraw()
        probe.update_idletasks()
        hwnd = win32gui.GetAncestor(probe.winfo_id(), 2)   # GA_ROOT
        size = win32api.GetSystemMetrics(win32con.SM_CXICON)
        hicon = win32gui.LoadImage(0, str(path), win32con.IMAGE_ICON, size, size,
                                   win32con.LR_LOADFROMFILE)
        user32 = ctypes.windll.user32
        user32.SetClassLongPtrW.argtypes = [ctypes.c_void_p, ctypes.c_int,
                                            ctypes.c_void_p]
        user32.SetClassLongPtrW.restype = ctypes.c_void_p
        user32.SetClassLongPtrW(hwnd, win32con.GCL_HICON, hicon)
        print(f"[ui] window icon applied: path={path} size={size} hicon={hicon:#x}",
              file=sys.stderr)
        return hicon
    except Exception as exc:
        print(f"[ui] window icon failed: path={path} error={exc}", file=sys.stderr)
        return None
    finally:
        if probe is not None:
            probe.destroy()


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

    config_existed = CONFIG_PATH.exists()
    cfg = load_config(CONFIG_PATH)

    # 介面語言要在建立任何視窗之前決定：文案與字型都由它決定。
    set_language(bootstrap_language(cfg, config_existed))

    # 擋掉第二份實例：兩份會搶著對遊戲掛 wizwalker hook，也會同時寫同一份設定與 log。
    # 位置卡在語言定案之後（既有實例的視窗標題就是 app_name()，要有語言才算得出來）、
    # 開 messages.log 之前（否則被擋下的那份會在收訊記錄裡多留一段空白 session）。
    # instance_lock 必須留著不放：handle 一被回收，mutex 就跟著釋放、放行下一份。
    instance_lock = acquire_single_instance()
    if instance_lock is None:
        focused = focus_running_instance(app_name())
        print(f"[app] another instance is already running (focused={focused}), exiting",
              file=sys.stderr)
        return

    # 收訊原始內容另存一份（不清理、不過濾），訊息類問題直接比對這份
    message_log = MessageLog(TimestampedStream(open_session_log("messages.log")))

    root = tk.Tk()
    root.withdraw()
    apply_window_icon(root)

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
    api = active_api(cfg)
    print(f"[app] startup; frozen={getattr(sys, 'frozen', False)}, "
          f"elevated={is_elevated()}, "
          f"ui_language={cfg['ui_language']} (active={current_language()}), "
          f"provider={api['provider']}, model={api['model']}, "
          f"target_language={cfg['target_language']}, hotkey={cfg['hotkey']}, "
          f"poll_interval={cfg['poll_interval']}, "
          f"parallel={cfg['max_parallel_translations']}", file=sys.stderr)

    translator = Translator(**api, target_language=cfg["target_language"])
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
        failed_notice_fn=lambda: t("notice.translate_failed"))

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
    ui_language = cfg["ui_language"]   # 用來判斷設定視窗是否改過介面語言

    def relabel_ui() -> None:
        """介面語言換掉後讓常駐視窗跟上（設定視窗預覽、還原與儲存都走這裡）。
        翻譯輸入框每次呼出才建立元件，會自然帶到新語言，不需要另外處理。"""
        overlay.refresh_labels()
        print(f"[ui] overlay relabelled for language {current_language()}",
              file=sys.stderr)

    def apply_settings() -> None:
        nonlocal hotkey_handle, ui_language
        save_config(CONFIG_PATH, cfg)
        translator.reconfigure(**active_api(cfg),
                               target_language=cfg["target_language"])
        pool.resize(cfg["max_parallel_translations"])
        keyboard.remove_hotkey(hotkey_handle)
        hotkey_handle = keyboard.add_hotkey(cfg["hotkey"],
                                            lambda: ui_queue.put(input_box.show))
        overlay.set_limits(cfg["max_messages"], cfg["fade_seconds"])
        overlay.set_alpha(cfg["overlay_alpha"])
        # 語言已由設定視窗套用（set_language）；這裡負責讓常駐的 overlay 跟上。
        # 預覽時通常已 relabel 過，這裡是沒經過預覽的路徑（程式化改語言）的保底。
        if cfg["ui_language"] != ui_language:
            ui_language = cfg["ui_language"]
            relabel_ui()
        applied = active_api(cfg)
        print(f"[settings] applied; provider={applied['provider']}, "
              f"model={applied['model']}, hotkey={cfg['hotkey']}, "
              f"ui_language={cfg['ui_language']}, "
              f"parallel={cfg['max_parallel_translations']}", file=sys.stderr)

    settings = SettingsWindow(root, cfg, on_save=apply_settings,
                              on_alpha_preview=overlay.set_alpha,
                              on_language_preview=relabel_ui)

    stop = threading.Event()
    reader_thread = threading.Thread(
        target=reader_loop, args=(cfg, overlay, ui_queue, stop, context, pool),
        kwargs={"on_input_open": lambda: ui_queue.put(input_box.show),
                "on_input_close": lambda: ui_queue.put(input_box.close),
                "message_log": message_log},
        daemon=True)
    reader_thread.start()

    # 更新檢查另開一條 daemon 執行緒：網路慢或不通都不該拖住啟動，關閉程式時也
    # 不等它（結果只是一條橫幅，丟掉無妨）。
    threading.Thread(target=announce_update, args=(ui_queue, overlay),
                     daemon=True).start()

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
