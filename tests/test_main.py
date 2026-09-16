"""main 的純邏輯：設定摘要（啟動與套用設定共用）。"""
import copy
import subprocess
import sys
from pathlib import Path

from src.config import DEFAULT_CONFIG, active_api
from src.main import config_summary


def test_config_summary_covers_the_default_config_without_the_api_key():
    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["api"]["custom"].update(base_url="http://x", model="gemma", api_key="sk-secret")
    cfg["api"]["provider"] = "custom"
    summary = config_summary(cfg, active_api(cfg))
    assert "model=gemma" in summary
    assert "sk-secret" not in summary
    # 每個使用者可調的設定都要在摘要裡，回報問題時才不必追問
    for key in ("target_language", "hotkey", "region_hotkey",
                "paste_hotkey", "auto_show_input", "poll_interval", "fade_seconds",
                "max_messages", "overlay_alpha", "translate_system_messages"):
        assert f"{key}=" in summary


ROOT = Path(__file__).resolve().parents[1]


def test_startup_path_does_not_import_anthropic():
    """啟動路徑不得載入 anthropic（約 0.7 秒，只有 Claude 官方 provider 用得到）。

    必須另起乾淨的直譯器問：同一個 process 內別的測試早就把 anthropic 載進來了。
    這條性質會被任何一次無心的 import 悄悄破壞，且不會有別的測試變紅。
    """
    code = "import src.main, sys; print('anthropic' in sys.modules)"
    out = subprocess.run([sys.executable, "-c", code], capture_output=True,
                         text=True, cwd=ROOT, check=True)
    assert out.stdout.strip() == "False", f"stdout={out.stdout!r} stderr={out.stderr!r}"
