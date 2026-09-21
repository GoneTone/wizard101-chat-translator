import copy
import json
import sys
from pathlib import Path

import pytest

from src.config import (
    DEFAULT_CONFIG,
    app_dir,
    is_configured,
    load_config,
    local_state_dir,
    save_config,
)
from src.services import UNSUPPORTED_SERVICES
from tests.config_helpers import configured_cfg


def test_load_missing_file_returns_defaults(tmp_path: Path):
    cfg = load_config(tmp_path / "nope.json")
    assert cfg == DEFAULT_CONFIG
    assert cfg is not DEFAULT_CONFIG  # 必須是副本，呼叫端改動不能污染預設值


def test_load_merges_partial_file_with_defaults(tmp_path: Path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"poll_interval": 3.0}), encoding="utf-8")
    cfg = load_config(p)
    assert cfg["poll_interval"] == 3.0
    assert cfg["services"] == []
    assert cfg["default_service"] is None
    assert cfg["hotkey"] == "ctrl+space"


def test_save_then_load_roundtrip(tmp_path: Path):
    p = tmp_path / "config.json"
    cfg = load_config(p)
    cfg["overlay_position"] = {"x": 100, "y": 200}
    cfg["poll_interval"] = 2.5  # 範圍內的值：出界值的夾限行為由 clamp 測試專門驗證
    save_config(p, cfg)
    reloaded = load_config(p)
    assert reloaded["overlay_position"] == {"x": 100, "y": 200}
    assert reloaded["poll_interval"] == 2.5


def test_app_dir_dev_mode_is_project_root():
    # 開發模式（非 frozen）：專案根目錄（pyproject.toml 所在）
    assert (app_dir() / "pyproject.toml").exists()


def test_app_dir_frozen_uses_executable_dir(monkeypatch, tmp_path):
    monkeypatch.setattr(sys, "frozen", True, raising=False)
    monkeypatch.setattr(sys, "executable", str(tmp_path / "Wizard101ChatTranslator.exe"))
    assert app_dir() == tmp_path


def test_legacy_api_block_is_migrated_and_rewritten(tmp_path):
    # 遷移後立刻落地，使用者不必先按一次儲存才看得到新結構
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"api": {"provider": "claude",
                                     "claude": {"model": "claude-opus-5",
                                                "api_key": "sk-ant-1"}}}),
                 encoding="utf-8")
    cfg = load_config(p)
    assert [s["provider"] for s in cfg["services"]] == ["claude"]
    assert cfg["default_service"] == cfg["services"][0]["id"]
    on_disk = json.loads(p.read_text(encoding="utf-8"))
    assert "api" not in on_disk
    assert on_disk["services"][0]["api_key"] == "sk-ant-1"


def test_an_unsupported_service_survives_the_rewrite_and_a_roundtrip(tmp_path):
    """這一版認不得的服務原樣留在檔案裡：跑過一次不該少掉任何欄位，金鑰尤其不能掉。"""
    p = tmp_path / "config.json"
    entry = {"id": "cccccccc", "name": "Future", "provider": "gemini",
             "model": "g-2", "api_key": "sk-REAL-USER-KEY", "quirk": True}
    p.write_text(json.dumps({"services": [copy.deepcopy(entry)]}), encoding="utf-8")

    cfg = load_config(p)
    assert cfg["services"] == []
    assert cfg[UNSUPPORTED_SERVICES] == [entry]
    assert json.loads(p.read_text(encoding="utf-8"))[UNSUPPORTED_SERVICES] == [entry]

    save_config(p, cfg)
    assert load_config(p)[UNSUPPORTED_SERVICES] == [entry]


def test_load_does_not_rewrite_a_current_file(tmp_path, monkeypatch):
    from src import config as config_module

    p = tmp_path / "config.json"
    save_config(p, configured_cfg("openai", model="gpt-x"))
    monkeypatch.setattr(config_module, "save_config",
                        lambda *a, **kw: pytest.fail("已是新格式就不該回寫"))
    assert load_config(p)["services"][0]["model"] == "gpt-x"


def test_load_survives_a_read_only_config_file(tmp_path, monkeypatch):
    from src import config as config_module

    p = tmp_path / "config.json"
    p.write_text(json.dumps({"api": {"provider": "custom", "base_url": "http://x",
                                     "model": "m"}}), encoding="utf-8")

    def _refuse(*_args, **_kwargs):
        raise OSError("read-only")

    monkeypatch.setattr(config_module, "save_config", _refuse)
    cfg = load_config(p)  # 寫不進去也要照常啟動
    assert cfg["services"][0]["base_url"] == "http://x"


def test_load_clamps_out_of_range_advanced_values(tmp_path):
    # 手改 config.json 填出界值（如 poll_interval=0 會變熱迴圈）→ 載入時拉回安全範圍
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"poll_interval": 0.001, "fade_seconds": -5,
                             "max_messages": 99999, "type_delay": 9.0}), encoding="utf-8")
    cfg = load_config(p)
    assert cfg["poll_interval"] == 0.1
    assert cfg["fade_seconds"] == 0
    assert cfg["max_messages"] == 1000
    assert cfg["type_delay"] == 0.5


def test_parallel_translations_defaults_and_clamps(tmp_path):
    from src.config import DEFAULT_CONFIG, clamp_advanced, load_config, save_config
    assert DEFAULT_CONFIG["max_parallel_translations"] == 4
    assert clamp_advanced({"max_parallel_translations": 99, **_others()})[
        "max_parallel_translations"] == 8
    assert clamp_advanced({"max_parallel_translations": 0, **_others()})[
        "max_parallel_translations"] == 1
    path = tmp_path / "config.json"
    save_config(path, {"max_parallel_translations": 50})
    assert load_config(path)["max_parallel_translations"] == 8


def _others():
    """clamp_advanced 會遍歷所有 ADVANCED_LIMITS 的鍵，補齊其餘欄位避免 KeyError。"""
    from src.config import DEFAULT_CONFIG
    return {k: DEFAULT_CONFIG[k] for k in
            ("poll_interval", "fade_seconds", "max_messages", "type_delay", "overlay_alpha")}


def test_is_configured_needs_a_complete_default_service():
    assert not is_configured(copy.deepcopy(DEFAULT_CONFIG))  # 一筆服務都沒有
    assert is_configured(configured_cfg())
    assert not is_configured(configured_cfg("openai", api_key=""))  # 官方端點要金鑰
    assert not is_configured(configured_cfg("custom", base_url=""))  # 自訂端點要網址


def test_default_config_has_ui_language():
    from src.config import DEFAULT_CONFIG

    assert DEFAULT_CONFIG["ui_language"] is None  # None＝尚未選過，啟動時偵測系統語言


def test_app_name_follows_language(tmp_path):
    from src import i18n
    from src.config import app_name

    before = i18n.current_language()
    try:
        i18n.set_language("en-US")
        assert app_name() == "Wizard101 Chat Translator"
        i18n.set_language("zh-TW")
        assert app_name() == "Wizard101 對話翻譯助手"
    finally:
        i18n.set_language(before)


def test_old_config_without_ui_language_loads(tmp_path):
    import json

    from src.config import load_config

    path = tmp_path / "config.json"
    path.write_text(json.dumps({"hotkey": "ctrl+alt+t"}), encoding="utf-8")
    cfg = load_config(path)
    assert cfg["ui_language"] is None       # 舊 config 補上預設值
    assert cfg["hotkey"] == "ctrl+alt+t"    # 既有設定不動


def test_local_state_dir_uses_localappdata(monkeypatch):
    monkeypatch.setenv("LOCALAPPDATA", r"C:\Users\test\AppData\Local")
    assert local_state_dir() == Path(r"C:\Users\test\AppData\Local") / "wizard101-chat-translator"


def test_local_state_dir_falls_back_to_home_without_localappdata(monkeypatch):
    monkeypatch.delenv("LOCALAPPDATA", raising=False)
    assert local_state_dir() == Path.home() / "wizard101-chat-translator"


def test_hook_state_shares_the_same_state_dir():
    from src.reader import hook_state
    assert local_state_dir() == hook_state.STATE_DIR


def test_translate_system_messages_defaults_to_off():
    assert DEFAULT_CONFIG["translate_system_messages"] is False


def test_load_config_fills_in_translate_system_messages(tmp_path):
    path = tmp_path / "config.json"
    path.write_text(json.dumps({"target_language": "日本語"}), encoding="utf-8")
    cfg = load_config(path)
    assert cfg["translate_system_messages"] is False


def test_load_config_with_broken_json_falls_back_to_defaults(tmp_path, monkeypatch):
    # 手改 config.json 少個逗號：windowed exe 沒有 console，炸在這裡等於無聲退出
    from src import config as config_module
    logged = []
    monkeypatch.setattr(config_module, "log", logged.append)
    p = tmp_path / "config.json"
    p.write_text('{"hotkey": "f8",}', encoding="utf-8")
    cfg = load_config(p)
    assert cfg == DEFAULT_CONFIG
    assert any("config.json" in line and "unreadable" in line for line in logged)


def test_load_config_with_non_numeric_advanced_value_uses_the_default(tmp_path, monkeypatch):
    from src import config as config_module
    logged = []
    monkeypatch.setattr(config_module, "log", logged.append)
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"poll_interval": "fast", "max_messages": None}), encoding="utf-8")
    cfg = load_config(p)
    assert cfg["poll_interval"] == DEFAULT_CONFIG["poll_interval"]
    assert cfg["max_messages"] == DEFAULT_CONFIG["max_messages"]
    assert any("poll_interval" in line for line in logged)


def test_default_config_has_a_region_hotkey_distinct_from_the_input_hotkey():
    from src.config import DEFAULT_CONFIG
    assert DEFAULT_CONFIG["region_hotkey"] == "ctrl+shift+space"
    assert DEFAULT_CONFIG["region_hotkey"] != DEFAULT_CONFIG["hotkey"]
