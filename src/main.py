"""進入點：reader 執行緒（wizwalker 收訊）+ 全域熱鍵 + tkinter 主迴圈（UI 事件經 ui_queue 序列化）。"""
import ctypes
import os
import queue
import sys
import threading
import tkinter as tk
import traceback
from dataclasses import dataclass

import keyboard
import win32api
import win32con
import win32event
import win32gui
import winerror

from src import __version__
from src.composer.paste import (
    PasteInterceptor,
    force_foreground,
    foreground_exe,
    install_paste_hook,
    paste_clipboard,
    type_into_window,
)
from src.config import (
    CONFIG_PATH,
    DEFAULT_CONFIG,
    active_api,
    app_name,
    is_configured,
    load_config,
    save_config,
)
from src.i18n import current_language, detect_system_language, language_name, set_language, t
from src.log import log
from src.logfiles import TimestampedStream, open_session_log
from src.reader.loop import reader_loop
from src.reader.message_log import MessageLog
from src.reader.process import is_game_process_path
from src.resources import icon_path
from src.translation.cache import (
    TranslationCache,
    fingerprint_of,
    translate_and_cache,
)
from src.translation.context import ChatContext
from src.translation.gate import ConcurrencyGate
from src.translation.pool import TranslationPool
from src.translation.translator import Translator
from src.ui.input_box import InputBox
from src.ui.overlay import OverlayWindow
from src.ui.settings import SettingsWindow
from src.ui.winstyle import root_hwnd
from src.updater import check_for_update

# 第二份實例會搶著對遊戲掛 wizwalker hook，也會同時寫同一份 config.json 與 log。
SINGLE_INSTANCE_MUTEX = "wizard101-chat-translator.single-instance"
UPDATE_CHECK_INTERVAL_MS = 60 * 60 * 1000   # 自動檢查更新的間隔

def is_elevated() -> bool:
    """本程序是否以系統管理員權限執行。掛入權限問題的診斷欄位，查不到當作否。"""
    try:
        return bool(ctypes.windll.shell32.IsUserAnAdmin())
    except Exception:
        return False


def bootstrap_language(cfg: dict, config_existed: bool, detect=detect_system_language) -> str:
    """決定啟動時的介面語言碼；真正首次執行時順便把 target_language 補成同語言。

    首次執行只能看 `config_existed`，不能看 `ui_language` 是否為 None：pre-i18n 的舊
    config 經 `load_config` 補值後同樣是 None，若據此判斷，升級的使用者會被當成首次
    執行，自己選過的 target_language 被系統偵測值覆蓋，並在下一次 save_config 永久寫死。
    首次執行時 target_language 跟著系統語言走，否則非 zh-TW 系統會在精靈看到不相關的
    繁中預設值；舊設定檔 ui_language 仍為 None 時只偵測介面語言、不動 target_language；
    ui_language 已是真實語言碼時不呼叫 `detect`。"""
    if cfg["ui_language"] is None:
        detected = detect()
        if not config_existed:
            cfg["target_language"] = language_name(detected)
        return detected
    return cfg["ui_language"]


def register_hotkey(hotkey: str, callback) -> tuple[object, str]:
    """向 keyboard 註冊全域熱鍵，回傳（handle，實際生效的熱鍵）。
    手改 config.json 填了不認得的鍵名時退回預設熱鍵：windowed exe 在這裡炸掉等於無聲退出，
    而設定視窗改熱鍵時舊的已先解除，失敗會讓輸入框再也呼不出來。"""
    try:
        return keyboard.add_hotkey(hotkey, callback), hotkey
    except ValueError as exc:
        fallback = DEFAULT_CONFIG["hotkey"]
        log(f"[app] hotkey {hotkey!r} is not a valid key combination ({exc}); "
            f"using {fallback!r}")
        return keyboard.add_hotkey(fallback, callback), fallback


def drain_ui_queue(ui_queue: queue.Queue) -> None:
    """依序取出並執行 ui_queue 裡的回呼；單一回呼拋錯不影響其餘回呼或呼叫端。"""
    while True:
        try:
            callback = ui_queue.get_nowait()
        except queue.Empty:
            break
        try:
            callback()
        except Exception as exc:
            log(f"[ui] callback failed: {type(exc).__name__}: {exc}\n"
                f"{traceback.format_exc()}")


def announce_update(ui_queue: queue.Queue, overlay, checker=check_for_update) -> None:
    """檢查更新，有新版就把橫幅回呼排進 ui_queue（供背景執行緒呼叫）。
    失敗只留 log：更新檢查是附加功能，不能影響啟動與收訊。`checker` 供測試注入。
    走 overlay.offer_update：使用者關掉過的版本不再反覆跳出。"""
    try:
        release = checker()
    except Exception as exc:
        log(f"[update] check failed: {exc}")
        return
    if release is None:
        return
    ui_queue.put(lambda: overlay.offer_update(release))


def schedule_update_checks(root, ui_queue: queue.Queue, overlay,
                           checker=check_for_update,
                           interval_ms: int = UPDATE_CHECK_INTERVAL_MS) -> None:
    """立刻在背景查一次更新，之後每隔 interval_ms 再查（由 root.after 排程，
    視窗銷毀即停）。網路慢不該拖住啟動，關閉時也不等它（結果只是一條橫幅）。"""
    threading.Thread(target=announce_update, args=(ui_queue, overlay),
                     kwargs={"checker": checker}, daemon=True).start()
    log(f"[update] next automatic check in {interval_ms // 60000} min")
    root.after(interval_ms, lambda: schedule_update_checks(
        root, ui_queue, overlay, checker=checker, interval_ms=interval_ms))


def acquire_single_instance(name: str = SINGLE_INSTANCE_MUTEX) -> int | None:
    """搶下單一實例的 mutex；已經有一份在跑時回 None。
    呼叫端必須留住回傳的 handle 直到程序結束：mutex 隨 handle 關閉而釋放。
    name 可覆寫讓測試各用各的名稱，不會被正在執行的本尊卡住。"""
    handle = win32event.CreateMutex(None, False, name)
    if win32api.GetLastError() == winerror.ERROR_ALREADY_EXISTS:
        return None
    return handle


def focus_running_instance(title: str) -> bool:
    """把既有實例的視窗帶到前景；找不到視窗或還原失敗時回 False。
    切前景走 composer 的 force_foreground（AttachThreadInput），單純 SetForegroundWindow
    會被前景鎖擋下；就算仍被擋，系統也會改成閃工作列按鈕，使用者看得到回應。"""
    found = []

    def collect(hwnd, _):
        try:
            if win32gui.IsWindowVisible(hwnd) and win32gui.GetWindowText(hwnd) == title:
                found.append(hwnd)
        except Exception:
            pass   # 單一視窗查詢失敗不該中斷整輪列舉
        return True

    try:
        win32gui.EnumWindows(collect, None)
    except Exception as exc:
        log(f"[app] window scan failed: {exc}")
        return False
    if not found:
        log(f"[app] no window titled {title!r} to focus")
        return False
    hwnd = found[0]
    try:
        win32gui.ShowWindow(hwnd, win32con.SW_RESTORE)
    except Exception as exc:
        log(f"[app] focus existing instance failed: hwnd={hwnd:#x} error={exc}")
        return False
    force_foreground(hwnd)
    return True


def on_hotkey(input_box: InputBox, ui_queue: queue.Queue) -> None:
    """全域熱鍵的回呼（在 keyboard 套件的執行緒上跑）：只有遊戲在前景才呼出翻譯輸入框，
    其他視窗前景時當作沒按，免得在瀏覽器、聊天軟體裡誤觸。輸入框已開著時（它自己就是
    前景）照舊重新對焦。"""
    if input_box.is_open:
        ui_queue.put(input_box.show)
        return
    exe = foreground_exe()
    if is_game_process_path(exe):
        ui_queue.put(input_box.show)
        return
    log(f"[app] hotkey ignored: foreground is not the game window (exe={exe!r})")


def should_intercept_paste(cfg: dict) -> bool:
    """Ctrl+V 要不要由本程式接手：設定開啟且遊戲在前景。其他視窗一律不碰，
    瀏覽器、聊天軟體裡的貼上照常。跑在鍵盤 hook 裡，每次按鍵都會問，要快。"""
    return bool(cfg["paste_hotkey"]) and is_game_process_path(foreground_exe())


def on_paste_hotkey(cfg: dict, game_chat_open: threading.Event) -> threading.Thread:
    """攔到遊戲內的 Ctrl+V：記下當下的前景視窗，交給背景執行緒鍵入剪貼簿（回傳該執行緒）。
    不能在 hook 回呼裡直接打字（Windows 會判定 hook 逾時而整個拔掉）。遊戲聊天輸入框
    開著時走單行模式（換行改空格），其他地方換行照打。"""
    hwnd = win32gui.GetForegroundWindow()
    worker = threading.Thread(target=paste_clipboard,
                              args=(hwnd, cfg["type_delay"], game_chat_open.is_set()),
                              daemon=True)
    worker.start()
    return worker


def apply_window_icon(root: tk.Tk) -> int | None:
    """把應用程式 icon 裝到視窗類別上，回傳 HICON；失敗回 None（icon 只是裝飾，不擋啟動）。

    不用 tkinter 的 `iconbitmap`：它在 Windows 上挑錯 ICO frame，實測 32x32 是放大裁切
    過的糊圖。改用 LoadImage 指定尺寸（會挑最接近的原生 frame）寫進視窗類別，之後每個
    Toplevel 都自動沿用。類別得透過 TkTopLevel 視窗才設得到 —— withdraw 的 root 是
    TkChild —— 故開一個隱藏 Toplevel 當跳板。小圖示（GCL_HICONSM）寫入回報成功卻讀不
    回來，但 Windows 11 工作列看的是 exe 資源的圖示，不受影響（尺寸要齊全，見
    src/assets/icon.ico）。"""
    path = icon_path()
    probe = None
    try:
        probe = tk.Toplevel(root)
        probe.withdraw()
        probe.update_idletasks()
        hwnd = root_hwnd(probe)
        size = win32api.GetSystemMetrics(win32con.SM_CXICON)
        hicon = win32gui.LoadImage(0, str(path), win32con.IMAGE_ICON, size, size,
                                   win32con.LR_LOADFROMFILE)
        user32 = ctypes.windll.user32
        user32.SetClassLongPtrW.argtypes = [ctypes.c_void_p, ctypes.c_int,
                                            ctypes.c_void_p]
        user32.SetClassLongPtrW.restype = ctypes.c_void_p
        user32.SetClassLongPtrW(hwnd, win32con.GCL_HICON, hicon)
        log(f"[ui] window icon applied: path={path} size={size} hicon={hicon:#x}")
        return hicon
    except Exception as exc:
        log(f"[ui] window icon failed: path={path} error={exc}")
        return None
    finally:
        if probe is not None:
            probe.destroy()


def redirect_output() -> None:
    """把 stdout／stderr 接到帶時戳的輸出：打包版落入 exe 旁的 app.log，開發模式留在主控台。"""
    if getattr(sys, "frozen", False):
        # windowed exe 的 stdout／stderr 為 None，全部導到 exe 旁的 app.log
        sys.stdout = sys.stderr = TimestampedStream(open_session_log("app.log"))
        return
    # 主控台編碼常是 cp950，UI 文字裡的 ✕ 之類字元會讓 print 拋 UnicodeEncodeError
    # 把程式帶掉，故先放寬成無法編碼就替換；時戳同樣要補，才對得上 messages.log。
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(errors="replace")
    sys.stdout = TimestampedStream(sys.stdout)
    sys.stderr = TimestampedStream(sys.stderr)


def config_summary(cfg: dict, api: dict) -> str:
    """一行設定摘要（啟動與套用設定時記錄，兩處同一份才不會漏欄位）；金鑰絕不列入。"""
    return (f"provider={api['provider']}, model={api['model']}, "
            f"target_language={cfg['target_language']!r}, ui_language={cfg['ui_language']}, "
            f"hotkey={cfg['hotkey']}, region_hotkey={cfg['region_hotkey']}, "
            f"paste_hotkey={cfg['paste_hotkey']}, "
            f"auto_show_input={cfg['auto_show_input']}, "
            f"poll_interval={cfg['poll_interval']}, "
            f"parallel={cfg['max_parallel_translations']}, "
            f"fade_seconds={cfg['fade_seconds']}, max_messages={cfg['max_messages']}, "
            f"overlay_alpha={cfg['overlay_alpha']}, "
            f"translate_system_messages={cfg['translate_system_messages']}")


def log_startup_summary(cfg: dict, api: dict) -> None:
    """啟動摘要：回報問題時第一眼掌握環境。"""
    log(f"[app] startup; frozen={getattr(sys, 'frozen', False)}, "
        f"elevated={is_elevated()}, active_language={current_language()}, "
        f"{config_summary(cfg, api)}")


def shutdown(stop: threading.Event, pools: list[TranslationPool],
             reader_thread: threading.Thread, root: tk.Tk,
             cache: TranslationCache) -> None:
    """乾淨關閉：停 reader（解除 wizwalker hook）、停翻譯池、卸熱鍵、落盤快取，最後硬退出。
    步驟順序見各段註解；本函式不返回。"""
    log("[app] shutting down, waiting for reader to unhook")
    stop.set()
    for pool in pools:
        pool.shutdown()
    try:
        keyboard.unhook_all()
    except Exception as exc:
        # 例外逃出會連 reader join、cache flush、os._exit 都跳過
        log(f"[app] keyboard.unhook_all failed: {exc}")
    # 等 reader 跑完 reader.close()（解除 hook、還原遊戲記憶體）；daemon 執行緒被直接
    # 砍掉會讓 hook 殘留，下次掛入 PatternFailed、需重開遊戲。
    reader_thread.join(timeout=8)
    try:
        root.destroy()
    except Exception:
        pass
    # 落盤放在 pool shutdown 與 reader join 之後，飛行中的 worker 才有機會先寫完
    # cache.put()；os._exit 不跑 atexit，這是寫回磁碟的最後機會。
    cache.flush()
    log("[app] shutdown complete")
    # 翻譯 worker 非 daemon，仍卡在 HTTP 請求（最長 _TIMEOUT=60 秒）時，一般 return 會讓
    # 直譯器在 concurrent.futures.thread._python_exit 等它們 join —— 視窗已關、程式卻在
    # 工作管理員多留 60 秒。該還原的都已還原（hook 已解除、log 為線緩衝），直接砍行程。
    os._exit(0)


@dataclass
class App:
    """build_app() 接好線的執行期物件：主迴圈的 pump 與關閉流程只需要這幾個。"""
    ui_queue: queue.Queue
    overlay: OverlayWindow
    pools: list[TranslationPool]
    cache: TranslationCache
    stop: threading.Event
    reader_thread: threading.Thread


def build_translation(cfg: dict, api: dict,
                      deliver) -> tuple[Translator, TranslationCache, list[TranslationPool]]:
    """翻譯端：翻譯器、系統訊息譯文快取，以及兩條翻譯池（玩家對話吃上下文；系統訊息
    不吃上下文、走快取）。兩條池共用同一個總量閘與 `deliver(msg_id, text, failed)`。"""
    translator = Translator(**api, target_language=cfg["target_language"])
    gate = ConcurrencyGate(cfg["max_parallel_translations"])
    cache = TranslationCache(fingerprint_of(api["provider"], api["model"],
                                            cfg["target_language"]))
    cache.load()

    def make_pool(translate_fn=None) -> TranslationPool:
        return TranslationPool(translator=translator, on_result=deliver,
                               workers=cfg["max_parallel_translations"],
                               failed_notice_fn=lambda: t("notice.translate_failed"),
                               translate_fn=translate_fn, gate=gate)

    pool = make_pool()
    system_pool = make_pool(lambda text, _ctx: translate_and_cache(translator, cache, text))
    return translator, cache, [pool, system_pool]


def build_app(cfg: dict, root: tk.Tk, message_log: MessageLog) -> App:
    """建構並接線所有元件（翻譯器與快取、overlay、翻譯池、輸入框與熱鍵、設定視窗），
    啟動 reader 執行緒與更新檢查。"""
    api = active_api(cfg)
    log_startup_summary(cfg, api)
    context = ChatContext()
    ui_queue: queue.Queue = queue.Queue()

    def save_field(key: str, value) -> None:
        """使用者拖過視窗／泡泡就立刻落地，不等按儲存。"""
        cfg[key] = value
        save_config(CONFIG_PATH, cfg)

    ov = cfg["overlay"]
    overlay = OverlayWindow(
        root,
        x=ov["x"], y=ov["y"], width=ov["width"], height=ov["height"],
        max_messages=cfg["max_messages"],
        fade_seconds=cfg["fade_seconds"],
        on_geometry_change=lambda x, y, w, h: save_field(
            "overlay", {"x": x, "y": y, "width": w, "height": h}),
        on_settings=lambda: ui_queue.put(lambda: settings.open()),
        on_close=root.quit,  # 結束 mainloop → 走 finally 的乾淨關閉
        bubble_position=cfg["bubble_position"],
        on_bubble_move=lambda x, y: save_field("bubble_position", {"x": x, "y": y}),
        alpha=cfg["overlay_alpha"],
    )

    def deliver(msg_id: int, text: str, failed: bool) -> None:
        """譯完（worker 執行緒）：把結果轉交 UI 執行緒回填 overlay 的佔位列。"""
        ui_queue.put(lambda: overlay.update_message(msg_id, text, failed=failed))

    translator, cache, pools = build_translation(cfg, api, deliver)
    pool, system_pool = pools

    input_box = InputBox(root, lambda text: translator.translate_outgoing(
        text, context.snapshot()), ui_queue,
        lambda translated, hwnd: type_into_window(hwnd, translated, delay=cfg["type_delay"]))
    hotkey_handle, cfg["hotkey"] = register_hotkey(cfg["hotkey"],
                                                   lambda: on_hotkey(input_box, ui_queue))
    # 遊戲聊天輸入框目前是否開著：reader 的邊緣觸發（經 ui_queue）設定，貼上執行緒讀取
    game_chat_open = threading.Event()
    # 攔截器每次都直接讀 cfg，設定視窗改開關不必重掛；關閉時由 shutdown 的 unhook_all 一併卸除
    install_paste_hook(PasteInterceptor(lambda: should_intercept_paste(cfg),
                                        lambda: on_paste_hotkey(cfg, game_chat_open)))
    ui_language = cfg["ui_language"]   # 用來判斷設定視窗是否改過介面語言

    def relabel_ui() -> None:
        """介面語言換掉後讓常駐視窗跟上（預覽、還原與儲存都走這裡）。
        翻譯輸入框每次呼出才建立元件，會自然帶到新語言。"""
        overlay.refresh_labels()
        log(f"[ui] overlay relabelled for language {current_language()}")

    def apply_settings() -> None:
        nonlocal hotkey_handle, ui_language
        save_config(CONFIG_PATH, cfg)
        applied_api = active_api(cfg)
        translator.reconfigure(**applied_api, target_language=cfg["target_language"])
        for p in pools:
            p.resize(cfg["max_parallel_translations"])
        # 服務商／模型／目標語言任一改變，舊譯文即失效
        cache.rebind(fingerprint_of(applied_api["provider"], applied_api["model"],
                                    cfg["target_language"]))
        keyboard.remove_hotkey(hotkey_handle)
        hotkey_handle, cfg["hotkey"] = register_hotkey(
            cfg["hotkey"], lambda: on_hotkey(input_box, ui_queue))
        overlay.set_limits(cfg["max_messages"], cfg["fade_seconds"])
        overlay.set_alpha(cfg["overlay_alpha"])
        # set_language 已由設定視窗呼叫；預覽通常已 relabel 過，這裡是沒經過預覽路徑的保底
        if cfg["ui_language"] != ui_language:
            ui_language = cfg["ui_language"]
            relabel_ui()
        log(f"[settings] applied; {config_summary(cfg, applied_api)}")

    settings = SettingsWindow(root, cfg, on_save=apply_settings,
                              on_alpha_preview=overlay.set_alpha,
                              on_language_preview=relabel_ui,
                              on_update_found=overlay.set_update,
                              cache=cache)

    def on_game_input_open(anchor) -> None:
        """遊戲聊天輸入框開了：記下錨點（熱鍵呼出也要貼齊），依設定決定是否自動呼出。"""
        game_chat_open.set()
        if anchor is not None:
            input_box.set_anchor(anchor)
        if cfg["auto_show_input"]:
            input_box.show()

    def on_game_input_close() -> None:
        """遊戲聊天輸入框關了：被動收起翻譯輸入框，打到一半的文字留到下次呼出。"""
        game_chat_open.clear()
        input_box.clear_anchor()
        if cfg["auto_show_input"]:
            input_box.hide()

    stop = threading.Event()
    reader_thread = threading.Thread(
        target=reader_loop, args=(cfg, overlay, ui_queue, stop, context, pool),
        kwargs={"on_input_open": lambda anchor: ui_queue.put(
                    lambda: on_game_input_open(anchor)),
                "on_input_close": lambda: ui_queue.put(on_game_input_close),
                "message_log": message_log,
                "system_pool": system_pool,
                "cache": cache},
        daemon=True)
    reader_thread.start()

    schedule_update_checks(root, ui_queue, overlay)
    return App(ui_queue, overlay, pools, cache, stop, reader_thread)


def main() -> None:
    redirect_output()
    # 版本先印：app.log 分段標頭後第一行就是版本
    log(f"[app] version={__version__}")

    config_existed = CONFIG_PATH.exists()
    cfg = load_config(CONFIG_PATH)

    # 介面語言要在建立任何視窗之前定案：文案與字型都由它決定
    set_language(bootstrap_language(cfg, config_existed))

    # 單一實例檢查卡在語言定案之後（既有實例的視窗標題 app_name() 要有語言才算得出來）、
    # 開 messages.log 之前（否則被擋下的那份會多留一段空白 session）。
    # instance_lock 必須留著：handle 一被回收，mutex 就釋放、放行下一份。
    instance_lock = acquire_single_instance()
    if instance_lock is None:
        focused = focus_running_instance(app_name())
        log(f"[app] another instance is already running (focused={focused}), exiting")
        return

    # 收訊原始內容另存一份，訊息類問題直接比對這份
    message_log = MessageLog(TimestampedStream(open_session_log("messages.log")))

    root = tk.Tk()
    root.withdraw()
    apply_window_icon(root)

    if not is_configured(cfg):
        from src.ui.wizard import run_wizard
        log("[app] config incomplete, launching first-run wizard")
        if not run_wizard(root, cfg):
            log("[app] wizard cancelled, exiting")
            root.destroy()
            return
        log("[app] wizard completed, config saved")
        save_config(CONFIG_PATH, cfg)

    app = build_app(cfg, root, message_log)

    def pump() -> None:
        drain_ui_queue(app.ui_queue)
        app.overlay.prune()
        root.after(50, pump)

    log(f"[app] running; hotkey={cfg['hotkey']} opens the input box; quit via the overlay ✕")
    pump()
    try:
        root.mainloop()
    except KeyboardInterrupt:
        pass  # Ctrl+C 安靜結束，不印 traceback
    finally:
        shutdown(app.stop, app.pools, app.reader_thread, root, app.cache)


if __name__ == "__main__":
    main()
