"""main 的純邏輯：設定摘要（啟動與套用設定共用）。"""
import copy

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
    for key in ("target_language", "hotkey", "region_hotkey", "paste_hotkey",
                "auto_show_input", "poll_interval", "fade_seconds", "max_messages",
                "overlay_alpha", "translate_system_messages"):
        assert f"{key}=" in summary
