"""config.json 讀寫;缺漏欄位以 DEFAULT_CONFIG 補齊。"""
import copy
import json
from pathlib import Path

DEFAULT_CONFIG: dict = {
    "api": {"base_url": "http://127.0.0.1:8000", "model": "", "api_key": ""},
    "poll_interval": 0.4,    # 收訊輪詢間隔(秒);快掃很便宜,可設小一點更即時
    "fade_seconds": 0,       # <=0:訊息永不依時間淡出(可滾動看歷史)
    "max_messages": 200,     # 視窗保留的訊息則數上限,超過移除最舊
    "hotkey": "ctrl+space",
    "game_path": None,       # 遊戲根目錄;null=自動偵測執行中的程序路徑(Steam 版需要)
    "type_delay": 0.02,      # 自動鍵入時每個字元間隔(秒),遊戲漏字就調大
    "overlay": {"x": None, "y": None, "width": 460, "height": 300},
    "input_position": {"x": None, "y": None},  # 翻譯輸入框位置(拖曳後記住)
}


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
    return _merge(DEFAULT_CONFIG, data)


def save_config(path: Path, cfg: dict) -> None:
    path.write_text(json.dumps(cfg, ensure_ascii=False, indent=2), encoding="utf-8")
