"""config.json 讀寫；缺漏欄位以 DEFAULT_CONFIG 補齊。"""
import copy
import json
import os
import sys
from pathlib import Path

from src.i18n import SOURCE_LANGUAGE, language_name, t
from src.log import log
from src.services import SLOTS, UNSUPPORTED_SERVICES, find, normalize, validate_service


def app_name() -> str:
    """應用程式顯示名稱：各視窗標題／工作列統一使用（隨介面語言變動）。"""
    return t("app.name")


def app_dir() -> Path:
    """應用程式目錄：config.json 與 log 的存放處。
    打包執行（frozen）時為 exe 所在目錄；開發時為專案根目錄。"""
    if getattr(sys, "frozen", False):
        return Path(sys.executable).parent
    return Path(__file__).resolve().parent.parent


def local_state_dir() -> Path:
    """本機狀態目錄：跨啟動保留、但不屬於使用者資料的檔案（hook 修復狀態、譯文快取）。
    與 `app_dir()` 分開 —— 那裡放的是使用者會去看、去改的東西（config.json、log）。"""
    return Path(os.environ.get("LOCALAPPDATA") or str(Path.home())) / "wizard101-chat-translator"


CONFIG_PATH = app_dir() / "config.json"


DEFAULT_CONFIG: dict = {
    "services": [],           # 使用者建立的翻譯服務；空＝尚未設定，啟動時進精靈
    "default_service": None,  # 預設服務的 id；未指定用途的都跟著它走
    "service_slots": {slot: None for slot in SLOTS},  # None＝跟隨預設
    # 這一版認不得 provider 的服務暫放於此（原樣保留，含金鑰）；日後認得它的版本會自動搬回 services
    UNSUPPORTED_SERVICES: [],
    "ui_language": None,     # 介面語言；None＝尚未選過，啟動時依系統語言自動判定
    # 收訊的目標語言（人讀名稱，直接帶入提示詞）；發話固定翻英文。首次啟動會被
    # bootstrap_language 換成系統語言，這裡只是舊設定檔缺欄位時的補值
    "target_language": language_name(SOURCE_LANGUAGE),
    "poll_interval": 0.4,    # 收訊輪詢間隔（秒）；快掃很便宜，可設小一點更即時
    "fade_seconds": 0,       # <=0：訊息永不依時間淡出（可滾動看歷史）
    "max_messages": 200,     # 視窗保留的訊息則數上限，超過移除最舊
    "max_parallel_translations": 4,  # 同時翻譯則數；實測 4 以上幾乎無額外收益，1＝逐則排隊
    "translate_system_messages": False,  # 是否翻譯遊戲系統訊息（掉寶／升等等）；量大會擠掉玩家對話
    "hotkey": "ctrl+space",
    "region_hotkey": "ctrl+shift+space",  # 框選畫面區域翻譯的熱鍵；不可與 hotkey 相同
    "auto_show_input": True,  # 遊戲開啟聊天輸入框時自動呼出翻譯輸入（關閉時自動收回）
    "paste_hotkey": True,     # 遊戲在前景時攔截 Ctrl+V，把剪貼簿文字自動鍵入（遊戲不支援貼上）
    "type_delay": 0.02,      # 自動鍵入時每個字元間隔（秒），遊戲漏字就調大
    "overlay_alpha": 0.80,   # 視窗不透明度（半透明底板與泡泡），小＝更透明
    # x／y 為 null＝尚未拖曳過，首次啟動擺螢幕正中央（見 OverlayWindow）
    "overlay": {"x": None, "y": None, "width": 640, "height": 420},
    "bubble_position": {"x": None, "y": None},  # 縮小泡泡位置（拖曳後記住；null＝overlay 右上角）
}


# 進階數值的安全範圍：設定視窗的 Spinbox／提示文字與 load_config 共用同一份。
ADVANCED_LIMITS: dict[str, tuple[float, float]] = {
    "poll_interval": (0.1, 5.0),
    "fade_seconds": (0, 3600),
    "max_messages": (10, 1000),
    "max_parallel_translations": (1, 8),
    "type_delay": (0.0, 0.5),
    "overlay_alpha": (0.3, 1.0),
}


def clamp_advanced(values: dict) -> dict:
    """就地把進階數值夾在安全範圍並回傳同一個 dict。設定視窗儲存與 config.json 載入
    都經過這裡 —— 手動編輯的出界值（如 poll_interval=0 會讓 reader 變熱迴圈）也會被拉回，
    手改成非數值（字串、null）則退回預設值並留 log。"""
    for key, (lo, hi) in ADVANCED_LIMITS.items():
        value = values[key]
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            log(f"[config] {key} is not a number ({value!r}); using default "
                f"{DEFAULT_CONFIG[key]}")
            value = DEFAULT_CONFIG[key]
        values[key] = min(hi, max(lo, value))
    return values


def _merge(base: dict, override: dict) -> dict:
    out = copy.deepcopy(base)
    for key, value in override.items():
        if isinstance(value, dict) and isinstance(out.get(key), dict):
            out[key] = _merge(out[key], value)
        else:
            out[key] = value
    return out


def load_config(path: Path) -> dict:
    """讀 config.json 並補齊缺漏欄位；檔案不存在或壞掉（手改少逗號）時回預設值。
    壞檔不覆寫：使用者的金鑰還在裡面，留給他自己修，只在 log 說明原因。"""
    if not path.exists():
        return copy.deepcopy(DEFAULT_CONFIG)
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(data, dict):
            raise ValueError(f"top level is {type(data).__name__}, expected an object")
    except (OSError, ValueError) as exc:
        log(f"[config] {path.name} is unreadable, using defaults (fix the file to "
            f"restore your settings): {exc}")
        return copy.deepcopy(DEFAULT_CONFIG)
    cfg = clamp_advanced(_merge(DEFAULT_CONFIG, data))
    if normalize(cfg):
        # 整理後立刻落地，手開 config.json 看到的就是生效的結構；寫不進去不擋啟動
        try:
            save_config(path, cfg)
            log(f"[config] rewrote {path.name} in the service list format")
        except OSError as exc:
            log(f"[config] could not rewrite {path.name}: {exc}")
    return cfg


def save_config(path: Path, cfg: dict) -> None:
    """把整份設定寫回 config.json（UTF-8、縮排，方便手改）。"""
    path.write_text(json.dumps(cfg, ensure_ascii=False, indent=2), encoding="utf-8")


def is_configured(cfg: dict) -> bool:
    """預設服務存在且設定完整（不完整 → 啟動時進首次設定精靈）。"""
    service = find(cfg, cfg["default_service"])
    return service is not None and not validate_service(service)
