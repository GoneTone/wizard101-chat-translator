"""config.json 讀寫；缺漏欄位以 DEFAULT_CONFIG 補齊。"""
import copy
import json
import sys
from pathlib import Path


APP_NAME = "Wizard101 對話翻譯助手"  # 應用程式顯示名稱：各視窗標題／工作列統一使用


def app_dir() -> Path:
    """應用程式目錄：config.json 與 log 的存放處。
    打包執行（frozen）時為 exe 所在目錄；開發時為專案根目錄。"""
    if getattr(sys, "frozen", False):
        return Path(sys.executable).parent
    return Path(__file__).resolve().parent.parent


CONFIG_PATH = app_dir() / "config.json"

DEFAULT_CONFIG: dict = {
    "api": {"provider": "openai", "base_url": "http://127.0.0.1:8000",
            "model": "", "api_key": "", "thinking": False},
    "target_language": "繁體中文（台灣）",  # 收訊翻成的目標語言（人讀名稱，直接帶入提示詞）；發話固定翻英文
    "poll_interval": 0.4,    # 收訊輪詢間隔（秒）；快掃很便宜，可設小一點更即時
    "fade_seconds": 0,       # <=0：訊息永不依時間淡出（可滾動看歷史）
    "max_messages": 200,     # 視窗保留的訊息則數上限，超過移除最舊
    "hotkey": "ctrl+space",
    "game_path": None,       # 遊戲根目錄；null=自動偵測執行中的程序路徑（Steam 版需要）
    "type_delay": 0.02,      # 自動鍵入時每個字元間隔（秒），遊戲漏字就調大
    "overlay_alpha": 0.80,   # 視窗不透明度（半透明底板與泡泡），小＝更透明
    "overlay": {"x": None, "y": None, "width": 460, "height": 300},
    "input_position": {"x": None, "y": None},  # 翻譯輸入框位置（拖曳後記住）
    "bubble_position": {"x": None, "y": None},  # 縮小泡泡位置（拖曳後記住；null＝overlay 右上角）
}


# 進階數值的安全範圍：設定視窗的 Spinbox／提示文字與 load_config 共用同一份。
ADVANCED_LIMITS: dict[str, tuple[float, float]] = {
    "poll_interval": (0.1, 5.0),
    "fade_seconds": (0, 3600),
    "max_messages": (10, 1000),
    "type_delay": (0.0, 0.5),
    "overlay_alpha": (0.3, 1.0),
}


def clamp_advanced(values: dict) -> dict:
    """就地把進階數值夾在安全範圍（直接修改傳入的 dict）並回傳同一個 dict。
    設定視窗儲存與 config.json 載入都經過這裡——手動編輯出界值
    （如 poll_interval=0 會讓 reader 變熱迴圈）也會被拉回。"""
    for key, (lo, hi) in ADVANCED_LIMITS.items():
        values[key] = min(hi, max(lo, values[key]))
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
    if not path.exists():
        return copy.deepcopy(DEFAULT_CONFIG)
    data = json.loads(path.read_text(encoding="utf-8"))
    # 舊版 config 沒有 provider 欄位：一律視為自訂端點，原設定不動
    if isinstance(data.get("api"), dict) and "provider" not in data["api"]:
        data["api"]["provider"] = "custom"
    return clamp_advanced(_merge(DEFAULT_CONFIG, data))


def save_config(path: Path, cfg: dict) -> None:
    path.write_text(json.dumps(cfg, ensure_ascii=False, indent=2), encoding="utf-8")


def is_configured(cfg: dict) -> bool:
    """API 設定是否完整（不完整 → 啟動時進首次設定精靈）。"""
    api = cfg["api"]
    if not api["model"]:
        return False
    if api["provider"] in ("openai", "claude"):
        return bool(api["api_key"])
    return bool(api["base_url"])
