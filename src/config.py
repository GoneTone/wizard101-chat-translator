"""config.json 讀寫；缺漏欄位以 DEFAULT_CONFIG 補齊。"""
import copy
import json
import os
import sys
from pathlib import Path

from src.i18n import t


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
    與 `app_dir()` 分開——那裡放的是使用者會去看、去改的東西（config.json、log）。"""
    return Path(os.environ.get("LOCALAPPDATA") or str(Path.home())) / "wizard101-chat-translator"


CONFIG_PATH = app_dir() / "config.json"

# 每家服務商各存一份設定（切換時互不覆蓋），且只存自己用得到的欄位——
# 官方端點的網址寫死在 translator，Claude 不吃 thinking 開關而是思考深度 effort。
# 這張表同時是服務商清單與「哪家有哪些欄位」的單一真實來源：載入補值、UI 該畫哪些
# 欄位（見 ui/fields.py 的 PROVIDERS）都看它。
API_PROFILE_FIELDS: dict[str, dict] = {
    "openai": {"model": "", "api_key": "", "thinking": False},
    "claude": {"model": "", "api_key": "", "effort": "auto"},
    "custom": {"base_url": "", "model": "", "api_key": "", "thinking": False},
}

API_PROVIDERS = tuple(API_PROFILE_FIELDS)

# Claude 的思考深度：auto＝不帶參數、由模型自行決定（adaptive）；low＝壓到最低。
# Claude 沒有「完全不思考」這個選項，故意不與另兩家的 thinking 開關共用欄位名。
EFFORT_AUTO = "auto"
EFFORT_LOW = "low"
API_EFFORTS = (EFFORT_AUTO, EFFORT_LOW)


def _default_api() -> dict:
    return {"provider": API_PROVIDERS[0],
            **{name: copy.deepcopy(fields)
               for name, fields in API_PROFILE_FIELDS.items()}}


DEFAULT_CONFIG: dict = {
    "api": _default_api(),
    "ui_language": None,     # 介面語言；None＝尚未選過，啟動時依系統語言自動判定
    "target_language": "繁體中文（台灣）",  # 收訊翻成的目標語言（人讀名稱，直接帶入提示詞）；發話固定翻英文
    "poll_interval": 0.4,    # 收訊輪詢間隔（秒）；快掃很便宜，可設小一點更即時
    "fade_seconds": 0,       # <=0：訊息永不依時間淡出（可滾動看歷史）
    "max_messages": 200,     # 視窗保留的訊息則數上限，超過移除最舊
    # 同時進行的收訊翻譯則數。實測 4 併發後幾乎無額外收益，只讓單則延遲更差；
    # 設 1 等同逐則排隊（本功能之前的行為）。
    "max_parallel_translations": 4,
    "hotkey": "ctrl+space",
    "auto_show_input": True,  # 遊戲開啟聊天輸入框時自動呼出翻譯輸入（關閉時自動收回）
    "game_path": None,       # 遊戲根目錄；null=自動偵測執行中的程序路徑（Steam 版需要）
    "type_delay": 0.02,      # 自動鍵入時每個字元間隔（秒），遊戲漏字就調大
    "overlay_alpha": 0.80,   # 視窗不透明度（半透明底板與泡泡），小＝更透明
    # x／y 為 null＝尚未拖曳過：首次啟動時擺螢幕正中央（見 OverlayWindow）
    "overlay": {"x": None, "y": None, "width": 640, "height": 420},
    "input_position": {"x": None, "y": None},  # 翻譯輸入框位置（拖曳後記住）
    "input_width": 460,      # 翻譯輸入框寬度（縮放後記住）；高度依內容自適應，不記
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


def active_api(cfg: dict) -> dict:
    """目前選用那家的 API 設定（扁平副本，額外帶 provider）。
    翻譯端與表單驗證都經過這裡，不必知道其他家的設定也存在同一個區塊裡。"""
    api = cfg["api"]
    return {"provider": api["provider"], **copy.deepcopy(api[api["provider"]])}


_LEGACY_API_FIELDS = {key for fields in API_PROFILE_FIELDS.values() for key in fields}


def _is_legacy_api(api: dict) -> bool:
    """舊版的 api 區塊是扁平的：設定欄位與 provider 並排（新版全收在各服務商底下）。"""
    return any(key in api for key in _LEGACY_API_FIELDS)


def _migrate_api(api: dict) -> dict:
    """把舊版扁平的 api 區塊整組搬進所屬服務商的子區塊（不屬於那家的欄位丟掉）。
    再更舊、連 provider 欄位都沒有的設定一律視為自訂端點。"""
    provider = api.get("provider", "custom")
    fields = API_PROFILE_FIELDS.get(provider, {})
    profile = {key: value for key, value in api.items() if key in fields}
    print(f"[config] migrated flat api block into provider={provider} profile "
          f"(fields={sorted(profile)})", file=sys.stderr)
    return {"provider": provider, provider: profile}


def _prune_profiles(api: dict) -> list[str]:
    """就地刪掉每家 profile 裡不屬於它的欄位，回傳刪掉的 `服務商.欄位` 清單。
    欄位表變動過（如 Claude 從 thinking 改成 effort）時，舊檔案會留下不再生效的
    欄位——留著只會讓人以為它有作用。"""
    dropped = []
    for provider, fields in API_PROFILE_FIELDS.items():
        profile = api.get(provider)
        if not isinstance(profile, dict):
            continue
        for key in [k for k in profile if k not in fields]:
            del profile[key]
            dropped.append(f"{provider}.{key}")
    return dropped


def load_config(path: Path) -> dict:
    if not path.exists():
        return copy.deepcopy(DEFAULT_CONFIG)
    data = json.loads(path.read_text(encoding="utf-8"))
    legacy = isinstance(data.get("api"), dict) and _is_legacy_api(data["api"])
    if legacy:
        data["api"] = _migrate_api(data["api"])
    cfg = clamp_advanced(_merge(DEFAULT_CONFIG, data))
    # 手改 config.json 打錯服務商名稱時 active_api 會 KeyError，先在這裡擋掉
    if cfg["api"]["provider"] not in API_PROVIDERS:
        print(f"[config] unknown provider {cfg['api']['provider']!r}; falling back to "
              f"{DEFAULT_CONFIG['api']['provider']}", file=sys.stderr)
        cfg["api"]["provider"] = DEFAULT_CONFIG["api"]["provider"]
    dropped = _prune_profiles(cfg["api"])
    if dropped:
        print(f"[config] dropped stale api fields ({', '.join(dropped)})",
              file=sys.stderr)
    if legacy or dropped:
        # 整理後立刻落地，不必等使用者按下儲存：手開 config.json 看到的就是生效的結構。
        # 寫不進去（目錄唯讀等）不該擋住啟動，記一行就繼續用記憶體裡整理好的設定。
        try:
            save_config(path, cfg)
            print(f"[config] rewrote {path.name} in the per-provider format",
                  file=sys.stderr)
        except OSError as exc:
            print(f"[config] could not rewrite {path.name}: {exc}", file=sys.stderr)
    return cfg


def save_config(path: Path, cfg: dict) -> None:
    path.write_text(json.dumps(cfg, ensure_ascii=False, indent=2), encoding="utf-8")


def needs_base_url(provider: str) -> bool:
    """這家服務商要不要自己填端點網址（官方端點的網址寫死在 translator）。"""
    return "base_url" in API_PROFILE_FIELDS[provider]


def is_configured(cfg: dict) -> bool:
    """API 設定是否完整（不完整 → 啟動時進首次設定精靈）。"""
    api = active_api(cfg)
    if not api["model"]:
        return False
    if needs_base_url(api["provider"]):
        return bool(api["base_url"])
    return bool(api["api_key"])
