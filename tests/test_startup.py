"""啟動時的診斷：設定摘要（啟動與套用設定共用）。"""
from src.startup import config_summary
from tests.config_helpers import configured_cfg


def test_config_summary_lists_every_slot_and_never_leaks_the_key():
    cfg = configured_cfg("custom", base_url="http://x", model="gemma",
                         api_key="sk-secret")
    summary = config_summary(cfg)
    assert "sk-secret" not in summary
    assert "has_key=True" in summary
    assert "incoming=default" in summary and "region=default" in summary
    # 隔離筆數：不帶內容，但「服務不見了」的回報要看得出有沒有東西被隔離
    assert "quarantined=0" in summary
    # 每個使用者可調的設定都要在摘要裡，回報問題時才不必追問
    for key in ("target_language", "hotkey", "region_hotkey",
                "paste_hotkey", "auto_show_input", "fade_seconds",
                "max_messages", "overlay_alpha", "translate_system_messages"):
        assert f"{key}=" in summary
