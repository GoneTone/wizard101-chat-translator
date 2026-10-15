"""進入點：supervisor 執行緒（每個遊戲客戶端一條 wizwalker 收訊執行緒）+ 全域熱鍵 +
tkinter 主迴圈（UI 事件經 ui_queue 序列化）。"""
import ctypes
import os
import queue
import sys
import threading
import tkinter as tk
import traceback
from dataclasses import dataclass

import keyboard

from src import __version__
from src.composer.paste import PasteInterceptor, type_into_window
from src.config import (
    CONFIG_PATH,
    app_name,
    bootstrap_language,
    config_summary,
    is_configured,
    load_config,
    save_config,
)
from src.config.services import (
    SLOT_INCOMING,
    SLOT_OUTGOING,
    SLOT_REGION,
    SLOTS,
    resolve,
)
from src.i18n import current_language, set_language, t
from src.input.dispatch import (
    ChatInputTracker,
    GameInputEvents,
    on_hotkey,
    on_paste_hotkey,
    on_region_hotkey,
    register_region_hotkey,
    should_intercept_paste,
)
from src.input.hotkeys import install, register_hotkey
from src.logs import log
from src.logs.files import TimestampedStream, open_session_log, redirect_output
from src.reader.loop import MessageIds, ReaderShared, reader_loop
from src.reader.message_log import MessageLog
from src.reader.status import StatusBoard
from src.reader.supervisor import JOIN_TIMEOUT, game_windows, supervise
from src.region.pipeline import RegionPipeline
from src.runtime import splash
from src.runtime.single_instance import acquire_single_instance, focus_running_instance, start_instance_watch
from src.runtime.updater import check_for_update
from src.translation.cache import (
    TranslationCache,
    fingerprint_of,
    translate_and_cache,
)
from src.translation.context import ClientContexts
from src.translation.examples import ExampleCoordinator, ExampleSet, ExampleStore
from src.translation.gate import ConcurrencyGate
from src.translation.pool import TranslationPool
from src.translation.translator import Translator
from src.ui.icons import apply_window_icon
from src.ui.input_box import InputBox
from src.ui.overlay.window import OverlayWindow
from src.ui.region.flow import RegionFlow
from src.ui.settings.window import SettingsWindow

UPDATE_CHECK_INTERVAL_MS = 60 * 60 * 1000   # 自動檢查更新的間隔


def is_elevated() -> bool:
    """本程序是否以系統管理員權限執行。掛入權限問題的診斷欄位，查不到當作否。"""
    try:
        return bool(ctypes.windll.shell32.IsUserAnAdmin())
    except Exception:
        return False


def log_startup_summary(cfg: dict) -> None:
    """啟動摘要：回報問題時第一眼掌握環境。"""
    log(f"[app] startup; frozen={getattr(sys, 'frozen', False)}, "
        f"elevated={is_elevated()}, active_language={current_language()}, "
        f"{config_summary(cfg)}")


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


def shutdown(stop: threading.Event, pools: list[TranslationPool],
             reader_thread: threading.Thread, root: tk.Tk,
             cache: TranslationCache) -> None:
    """乾淨關閉：停下所有 reader（各自解除 wizwalker hook）、停翻譯池、卸熱鍵、落盤快取，最後硬退出。
    步驟順序見各段註解；本函式不返回。"""
    log("[app] shutting down, waiting for readers to unhook")
    stop.set()
    for pool in pools:
        pool.shutdown()
    try:
        keyboard.unhook_all()
    except Exception as exc:
        # 例外逃出會連 reader join、cache flush、os._exit 都跳過
        log(f"[app] keyboard.unhook_all failed: {exc}")
    # 等 supervisor 把每條 reader join 完（各自 unhook）；+2 秒讓 supervisor 自己的
    # JOIN_TIMEOUT 預算永遠先跑完，「still alive」的診斷 log 才寫得出來。
    reader_thread.join(timeout=JOIN_TIMEOUT + 2)
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


def incoming_fingerprint(cfg: dict, examples: ExampleSet | None = None) -> str:
    """收訊那格目前生效的譯文快取指紋（建構、套用設定與範例就緒共用同一份算法）。
    端點只有自訂服務有，其餘服務商的網址寫死在 translator；`examples` 為已套用的範例（無則 None）。"""
    incoming = resolve(cfg, SLOT_INCOMING)
    return fingerprint_of(incoming["provider"], incoming["model"],
                          cfg["target_language"], incoming.get("base_url", ""),
                          examples.digest() if examples else "-")


def build_translation(cfg: dict, deliver, post) -> tuple[
        dict[str, Translator], TranslationCache, list[TranslationPool], ExampleCoordinator]:
    """翻譯端：三個用途各一個翻譯器、系統訊息譯文快取、兩條翻譯池（玩家對話吃上下文；
    系統訊息不吃上下文、走快取），以及範例協調器。兩條池共用同一個總量閘與
    `deliver(msg_id, text, failed)`；`post` 把回呼排進 UI 執行緒（須執行緒安全）。"""
    translators = {slot: Translator(**resolve(cfg, slot),
                                    target_language=cfg["target_language"])
                   for slot in SLOTS}
    gate = ConcurrencyGate(cfg["max_parallel_translations"])
    store = ExampleStore()
    stored = store.get(translators[SLOT_INCOMING].examples_fingerprint)
    cache = TranslationCache(incoming_fingerprint(cfg, stored))   # 與磁碟上次存的指紋一致才不會被清掉
    cache.load()

    def on_examples_applied(tr: Translator) -> None:
        if tr is translators[SLOT_INCOMING]:
            cache.rebind(incoming_fingerprint(cfg, tr.examples))

    coordinator = ExampleCoordinator(store, post, on_applied=on_examples_applied)

    def make_pool(translate_fn=None) -> TranslationPool:
        return TranslationPool(translator=translators[SLOT_INCOMING], on_result=deliver,
                               workers=cfg["max_parallel_translations"],
                               failed_notice_fn=lambda: t("notice.translate_failed"),
                               translate_fn=translate_fn, gate=gate)

    pool = make_pool()
    system_pool = make_pool(
        lambda text, _ctx: translate_and_cache(translators[SLOT_INCOMING], cache, text))
    coordinator.ensure(translators[SLOT_INCOMING])
    coordinator.ensure(translators[SLOT_REGION])
    return translators, cache, [pool, system_pool], coordinator


def reconfigure_translation(cfg: dict, translators: dict[str, Translator],
                            cache: TranslationCache, pools: list[TranslationPool],
                            coordinator: ExampleCoordinator) -> None:
    """設定存檔後讓翻譯端跟上新設定（build_translation 的對應面）。
    沒改到的那幾格會原地不動（見 Translator.reconfigure）—— 一次存檔只該影響被改動的用途；
    被重建的收訊／框選格會重新備妥範例。"""
    for slot, tr in translators.items():
        if tr.reconfigure(**resolve(cfg, slot), target_language=cfg["target_language"]):
            log(f"[translate] {slot} translator rebuilt: {tr.describe()}")
            if slot in (SLOT_INCOMING, SLOT_REGION):
                coordinator.ensure(tr)
    for p in pools:
        p.resize(cfg["max_parallel_translations"])
    # 收訊那格的服務（含端點）、目標語言或範例一改，舊譯文即失效；只改區域翻譯那格不該波及它
    cache.rebind(incoming_fingerprint(cfg, translators[SLOT_INCOMING].examples))


def build_app(cfg: dict, root: tk.Tk, message_stream) -> App:
    """建構並接線所有元件（翻譯器與快取、overlay、翻譯池、輸入框與熱鍵、設定視窗），
    啟動 supervisor 執行緒與更新檢查。`message_stream`＝messages.log 的串流，每個客戶端
    各建一個 `MessageLog` 共寫。"""
    log_startup_summary(cfg)
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
        # region_flow 在下方才建立；閉包晚繫結沒關係，這顆按鈕只在點下當下才會呼叫它
        on_region=lambda: ui_queue.put(lambda: region_flow.start_from_button()),
        region_hotkey=cfg["region_hotkey"],
    )
    # 讓第一份實例主動攔截第二次啟動：先砍兄弟 bootloader、再喚起自己的視窗，
    # 使用者不必再等新實例解壓完才看到既有視窗被喚起。frozen 模式才生效；
    # 沒攔到（自我檢查失敗等）時，acquire_single_instance() 的 mutex 檢查仍是保底。
    start_instance_watch(lambda: ui_queue.put(lambda: focus_running_instance(app_name())))

    def deliver(msg_id: int, text: str, failed: bool) -> None:
        """譯完（worker 執行緒）：把結果轉交 UI 執行緒回填 overlay 的佔位列。"""
        ui_queue.put(lambda: overlay.update_message(msg_id, text, failed=failed))

    translators, cache, pools, example_coordinator = build_translation(
        cfg, deliver, ui_queue.put)
    pool, system_pool = pools
    contexts: dict[int, ClientContexts] = {}

    def context_for(hwnd: int | None) -> list[str]:
        """發話時帶的上下文：譯文要打回哪個客戶端，就用那個客戶端的聊天。"""
        client = contexts.get(hwnd) if hwnd is not None else None
        return client.current().snapshot() if client is not None else []

    input_box = InputBox(root, lambda text, cancel: translators[SLOT_OUTGOING].translate_outgoing(
        text, context_for(input_box.target_hwnd), cancel=cancel), ui_queue,
        lambda translated, hwnd: type_into_window(hwnd, translated, delay=cfg["type_delay"]),
        describe_service=translators[SLOT_OUTGOING].describe)
    unregister_hotkey, cfg["hotkey"] = register_hotkey(
        cfg["hotkey"], lambda: on_hotkey(input_box, ui_queue))
    region_pipeline = RegionPipeline(translators[SLOT_REGION])
    region_flow = RegionFlow(root, region_pipeline, ui_queue, cfg["overlay_alpha"])
    unregister_region = register_region_hotkey(
        cfg, lambda: on_region_hotkey(region_flow, ui_queue))
    tracker = ChatInputTracker()
    # 攔截器每次都直接讀 cfg，設定視窗改開關不必重掛；關閉時由 shutdown 的 unhook_all 一併卸除
    install(PasteInterceptor(lambda: should_intercept_paste(cfg),
                             lambda: on_paste_hotkey(cfg, tracker)))
    ui_language = cfg["ui_language"]   # 用來判斷設定視窗是否改過介面語言

    def relabel_ui() -> None:
        """介面語言換掉後讓常駐視窗跟上（預覽、還原與儲存都走這裡）。
        翻譯輸入框每次呼出才建立元件，會自然帶到新語言。"""
        overlay.refresh_labels()
        log(f"[ui] overlay relabelled for language {current_language()}")

    def apply_settings() -> None:
        nonlocal unregister_hotkey, unregister_region, ui_language
        save_config(CONFIG_PATH, cfg)
        reconfigure_translation(cfg, translators, cache, pools, example_coordinator)
        unregister_hotkey()
        unregister_hotkey, cfg["hotkey"] = register_hotkey(
            cfg["hotkey"], lambda: on_hotkey(input_box, ui_queue))
        if unregister_region is not None:
            unregister_region()
        unregister_region = register_region_hotkey(
            cfg, lambda: on_region_hotkey(region_flow, ui_queue))
        overlay.set_region_hotkey(cfg["region_hotkey"])
        region_flow.set_alpha(cfg["overlay_alpha"])
        overlay.set_limits(cfg["max_messages"], cfg["fade_seconds"])
        overlay.set_alpha(cfg["overlay_alpha"])
        # set_language 已由設定視窗呼叫；預覽通常已 relabel 過，這裡是沒經過預覽路徑的保底
        if cfg["ui_language"] != ui_language:
            ui_language = cfg["ui_language"]
            relabel_ui()
        log(f"[settings] applied; {config_summary(cfg)}")

    settings = SettingsWindow(root, cfg, on_save=apply_settings,
                              on_alpha_preview=overlay.set_alpha,
                              on_language_preview=relabel_ui,
                              on_update_found=overlay.set_update,
                              cache=cache)

    input_events = GameInputEvents(cfg, input_box, tracker)

    stop = threading.Event()
    board = StatusBoard(overlay, ui_queue, pool, system_pool)
    shared = ReaderShared(cfg, overlay, ui_queue, stop, pool, board, MessageIds(),
                          system_pool=system_pool, cache=cache)

    def spawn(hwnd: int, slot: int) -> threading.Thread:
        """supervisor 發現一個遊戲視窗：為它起一條收訊執行緒（上下文每客戶端一份）。"""
        contexts[hwnd] = ClientContexts()
        thread = threading.Thread(
            target=reader_loop,
            args=(shared, hwnd, slot),
            kwargs={"on_input_open": lambda h, anchor: ui_queue.put(
                        lambda: input_events.opened(h, anchor)),
                    "on_input_close": lambda h: ui_queue.put(lambda: input_events.closed(h)),
                    "message_log": MessageLog(message_stream, slot=slot),
                    "contexts": contexts[hwnd]},
            daemon=True, name=f"reader-{slot}")
        thread.start()
        return thread

    reader_thread = threading.Thread(
        target=supervise,
        args=(stop, spawn, game_windows, board),
        daemon=True, name="reader-supervisor")
    reader_thread.start()

    schedule_update_checks(root, ui_queue, overlay)
    return App(ui_queue, overlay, pools, cache, stop, reader_thread)


def main() -> None:
    redirect_output()
    # 輸出已可寫入，先把 splash 在導向之前累積的診斷補寫出來，才不會憑空消失
    splash.drain_logs()
    # 版本先印：app.log 分段標頭後第一行就是版本
    log(f"[app] version={__version__}")
    status = "active" if splash.is_available() else "not present"
    if splash.had_failure():
        # is_available() 只看 pyi_splash 有沒有 import 成功，看不出 IPC socket 斷線；
        # 沒有這行，socket 斷掉時這裡仍會印 active，使用者看到的卻是凍結的畫面
        status += " but a call already failed (see [splash] lines above)"
    log(f"[splash] startup screen {status} (frozen={getattr(sys, 'frozen', False)})")

    config_existed = CONFIG_PATH.exists()
    cfg = load_config(CONFIG_PATH)

    # 介面語言要在建立任何視窗之前定案：文案與字型都由它決定
    set_language(bootstrap_language(cfg, config_existed))

    # 單一實例檢查卡在語言定案之後（既有實例的視窗標題 app_name() 要有語言才算得出來）、
    # 開 messages.log 之前（否則被擋下的那份會多留一段空白 session）。
    # instance_lock 必須留著：handle 一被回收，mutex 就釋放、放行下一份。
    instance_lock = acquire_single_instance()
    if instance_lock is None:
        splash.close()   # 先關掉，否則會蓋在被喚起的既有視窗上
        focused = focus_running_instance(app_name())
        log(f"[app] another instance is already running (focused={focused}), exiting")
        return

    # 收訊原始內容另存一份，訊息類問題直接比對這份
    message_stream = TimestampedStream(open_session_log("messages.log"))

    root = tk.Tk()
    root.withdraw()
    apply_window_icon(root)

    if not is_configured(cfg):
        from src.ui.settings.wizard import run_wizard
        splash.close()   # 精靈要跟使用者互動，啟動畫面不能擋在前面
        log("[app] config incomplete, launching first-run wizard")
        if not run_wizard(root, cfg):
            log("[app] wizard cancelled, exiting")
            root.destroy()
            return
        log("[app] wizard completed, config saved")
        save_config(CONFIG_PATH, cfg)

    splash.update(splash.PHASE_STARTING)
    app = build_app(cfg, root, message_stream)
    splash.close()

    def pump() -> None:
        drain_ui_queue(app.ui_queue)
        app.overlay.prune()
        root.after(50, pump)

    log(f"[app] running; hotkey={cfg['hotkey']} opens the input box, "
        f"region_hotkey={cfg['region_hotkey']} starts region translation; "
        f"quit via the overlay ✕")
    pump()
    try:
        root.mainloop()
    except KeyboardInterrupt:
        pass  # Ctrl+C 安靜結束，不印 traceback
    finally:
        shutdown(app.stop, app.pools, app.reader_thread, root, app.cache)


if __name__ == "__main__":
    main()
