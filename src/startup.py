"""啟動時的環境準備與診斷：介面語言、視窗 icon、輸出導向、設定摘要 log。"""
import ctypes
import sys
import tkinter as tk

import win32api
import win32con
import win32gui

from src.i18n import current_language, detect_system_language, language_name
from src.log import log
from src.logfiles import TimestampedStream, open_session_log
from src.resources import icon_path
from src.services import SLOTS, find, quarantined
from src.ui.winstyle import root_hwnd


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


def config_summary(cfg: dict) -> str:
    """一行設定摘要（啟動與套用設定時記錄，兩處同一份才不會漏欄位）；金鑰絕不列入。"""
    fields = [f"services={len(cfg['services'])}",
              # 隔離筆數：使用者回報「服務不見了」時，這一格就是答案
              f"quarantined={len(quarantined(cfg))}",
              f"default={_service_summary(find(cfg, cfg['default_service']))}"]
    fields += [f"{slot}=" + (_service_summary(find(cfg, cfg['service_slots'][slot]))
                             if cfg["service_slots"][slot] else "default")
               for slot in SLOTS]
    fields += [f"target_language={cfg['target_language']!r}",
               f"ui_language={cfg['ui_language']}",
               f"hotkey={cfg['hotkey']}",
               f"region_hotkey={cfg['region_hotkey']}",
               f"paste_hotkey={cfg['paste_hotkey']}",
               f"auto_show_input={cfg['auto_show_input']}",
               f"parallel={cfg['max_parallel_translations']}",
               f"fade_seconds={cfg['fade_seconds']}",
               f"max_messages={cfg['max_messages']}",
               f"overlay_alpha={cfg['overlay_alpha']}",
               f"translate_system_messages={cfg['translate_system_messages']}"]
    return ", ".join(fields)


def _service_summary(service: dict | None) -> str:
    """一筆服務在 log 裡的樣子；金鑰只記有沒有，絕不記內容。"""
    if service is None:
        return "none"
    return (f"{service['name']}({service['provider']}/{service['model']},"
            f"has_key={bool(service.get('api_key'))})")


def log_startup_summary(cfg: dict) -> None:
    """啟動摘要：回報問題時第一眼掌握環境。"""
    log(f"[app] startup; frozen={getattr(sys, 'frozen', False)}, "
        f"elevated={is_elevated()}, active_language={current_language()}, "
        f"{config_summary(cfg)}")
