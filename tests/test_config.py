import json
import sys
from pathlib import Path

import pytest

from src.config import (
    API_PROFILE_FIELDS,
    API_PROVIDERS,
    DEFAULT_CONFIG,
    active_api,
    app_dir,
    is_configured,
    load_config,
    local_state_dir,
    save_config,
)


def test_load_missing_file_returns_defaults(tmp_path: Path):
    cfg = load_config(tmp_path / "nope.json")
    assert cfg == DEFAULT_CONFIG
    assert cfg is not DEFAULT_CONFIG  # 必須是副本，呼叫端改動不能污染預設值


def test_load_merges_partial_file_with_defaults(tmp_path: Path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"poll_interval": 3.0, "api": {"openai": {"model": "m1"}}}),
                 encoding="utf-8")
    cfg = load_config(p)
    assert cfg["poll_interval"] == 3.0
    assert cfg["api"]["openai"]["model"] == "m1"
    assert cfg["api"]["openai"]["api_key"] == ""      # 同一家缺的欄位補預設
    assert cfg["api"]["claude"] == DEFAULT_CONFIG["api"]["claude"]  # 沒提到的服務商補整份
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


def test_load_old_config_without_provider_migrates_to_custom(tmp_path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"api": {"base_url": "http://127.0.0.1:8000", "model": "m1"}}),
                 encoding="utf-8")
    cfg = load_config(p)
    # 舊使用者的自架端點設定原封不動繼續用，只是搬進 custom 那一份
    assert cfg["api"]["provider"] == "custom"
    assert cfg["api"]["custom"]["base_url"] == "http://127.0.0.1:8000"
    assert cfg["api"]["custom"]["model"] == "m1"


def test_load_flat_config_moves_fields_into_its_own_provider(tmp_path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"api": {"provider": "claude", "model": "claude-opus-5",
                                     "api_key": "sk-ant-1"}}), encoding="utf-8")
    cfg = load_config(p)
    assert cfg["api"]["provider"] == "claude"
    assert cfg["api"]["claude"]["model"] == "claude-opus-5"
    assert cfg["api"]["claude"]["api_key"] == "sk-ant-1"
    assert cfg["api"]["openai"]["model"] == ""  # 沒用過的服務商維持空白


def test_load_rewrites_a_legacy_file_in_place(tmp_path):
    # 遷移後立刻落地，使用者不必先按一次儲存才看得到新結構
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"api": {"provider": "custom", "base_url": "http://x",
                                     "model": "m"}}), encoding="utf-8")
    load_config(p)
    on_disk = json.loads(p.read_text(encoding="utf-8"))["api"]
    assert on_disk["custom"]["base_url"] == "http://x"
    assert set(on_disk) == {"provider", *API_PROVIDERS}


def test_load_does_not_rewrite_a_current_file(tmp_path, monkeypatch):
    from src import config as config_module

    p = tmp_path / "config.json"
    p.write_text(json.dumps({"api": {"provider": "openai",
                                     "openai": {"model": "gpt-x"}}}), encoding="utf-8")
    monkeypatch.setattr(config_module, "save_config",
                        lambda *a, **kw: pytest.fail("已是新格式就不該回寫"))
    assert load_config(p)["api"]["openai"]["model"] == "gpt-x"


def test_load_survives_a_read_only_config_file(tmp_path, monkeypatch):
    from src import config as config_module

    p = tmp_path / "config.json"
    p.write_text(json.dumps({"api": {"provider": "custom", "base_url": "http://x"}}),
                 encoding="utf-8")

    def _refuse(*_args, **_kwargs):
        raise OSError("read-only")

    monkeypatch.setattr(config_module, "save_config", _refuse)
    cfg = load_config(p)  # 寫不進去也要照常啟動
    assert cfg["api"]["custom"]["base_url"] == "http://x"


def test_load_new_config_keeps_every_provider_profile(tmp_path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"api": {"provider": "custom",
                                     "openai": {"model": "gpt-x", "api_key": "sk-1"},
                                     "custom": {"base_url": "http://x", "model": "m"}}}),
                 encoding="utf-8")
    cfg = load_config(p)
    assert active_api(cfg)["model"] == "m"
    assert cfg["api"]["openai"]["api_key"] == "sk-1"  # 切走的那家設定留著


def test_load_unknown_provider_falls_back_to_default(tmp_path):
    # 手改 config.json 打錯服務商名稱：active_api 會 KeyError，載入時就拉回預設
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"api": {"provider": "gemini"}}), encoding="utf-8")
    cfg = load_config(p)
    assert cfg["api"]["provider"] == DEFAULT_CONFIG["api"]["provider"]


def test_active_api_is_a_flat_copy_of_the_selected_profile(tmp_path):
    cfg = load_config(tmp_path / "nope.json")
    cfg["api"]["openai"]["model"] = "gpt-x"
    cfg["api"]["claude"]["model"] = "claude-x"
    assert active_api(cfg) == {"provider": "openai", "model": "gpt-x",
                               "api_key": "", "thinking": False}
    cfg["api"]["provider"] = "claude"
    assert active_api(cfg)["model"] == "claude-x"
    active_api(cfg)["model"] = "mutated"  # 副本：改它不該回頭污染 cfg
    assert cfg["api"]["claude"]["model"] == "claude-x"


def test_every_provider_has_a_profile_by_default():
    assert set(DEFAULT_CONFIG["api"]) == {"provider", *API_PROVIDERS}
    assert DEFAULT_CONFIG["api"]["custom"]["base_url"] == ""  # 端點網址一律由使用者填


def test_each_profile_only_carries_its_own_fields():
    api = DEFAULT_CONFIG["api"]
    for provider, fields in API_PROFILE_FIELDS.items():
        assert set(api[provider]) == set(fields)
    # 官方端點的網址寫死在 translator；Claude 用思考深度而不是 thinking 開關
    assert "base_url" not in api["openai"]
    assert "thinking" not in api["claude"]
    assert api["claude"]["effort"] == "auto"


def test_load_drops_fields_that_no_longer_belong_to_a_provider(tmp_path):
    # 欄位表變動過（Claude 從 thinking 改成 effort）：舊檔案留下的欄位要清掉並落地
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"api": {"provider": "claude",
                                     "claude": {"model": "claude-opus-5",
                                                "api_key": "sk-ant-1",
                                                "thinking": True},
                                     "openai": {"model": "gpt-x", "base_url": "http://x"}}}),
                 encoding="utf-8")
    cfg = load_config(p)
    assert cfg["api"]["claude"] == {"model": "claude-opus-5", "api_key": "sk-ant-1",
                                    "effort": "auto"}
    assert "base_url" not in cfg["api"]["openai"]
    assert cfg["api"]["openai"]["model"] == "gpt-x"      # 認得的欄位原樣留著
    on_disk = json.loads(p.read_text(encoding="utf-8"))["api"]
    assert "thinking" not in on_disk["claude"]           # 清掉的欄位不該留在檔案裡


def test_migrating_a_flat_block_drops_fields_that_provider_lacks(tmp_path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"api": {"provider": "claude", "model": "claude-opus-5",
                                     "api_key": "sk-ant-1", "base_url": "http://x",
                                     "thinking": True}}), encoding="utf-8")
    cfg = load_config(p)
    assert cfg["api"]["claude"] == {"model": "claude-opus-5", "api_key": "sk-ant-1",
                                    "effort": "auto"}


def test_load_config_without_api_block_keeps_default_provider(tmp_path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"hotkey": "f8"}), encoding="utf-8")
    cfg = load_config(p)
    assert cfg["api"]["provider"] == "openai"


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


def test_is_configured():
    cfg = load_config(Path("nope.json"))
    assert not is_configured(cfg)  # model 空
    cfg["api"]["openai"].update(model="gpt-x", api_key="sk-1")
    assert is_configured(cfg)
    cfg["api"]["openai"]["api_key"] = ""
    assert not is_configured(cfg)  # openai/claude 需要金鑰
    cfg["api"]["provider"] = "custom"
    cfg["api"]["custom"].update(model="m", base_url="http://127.0.0.1:8000")
    assert is_configured(cfg)  # custom 不需金鑰，需 base_url
    cfg["api"]["custom"]["base_url"] = ""
    assert not is_configured(cfg)  # 預設就是空的：沒填端點網址不算設定完成


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


def test_region_force_ocr_defaults_to_off():
    assert DEFAULT_CONFIG["region_force_ocr"] is False


def test_load_config_fills_in_region_force_ocr(tmp_path):
    path = tmp_path / "config.json"
    path.write_text(json.dumps({"target_language": "日本語"}), encoding="utf-8")
    cfg = load_config(path)
    assert cfg["region_force_ocr"] is False


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
