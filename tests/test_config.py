import json
import sys
from pathlib import Path

from src.config import DEFAULT_CONFIG, app_dir, is_configured, load_config, save_config


def test_load_missing_file_returns_defaults(tmp_path: Path):
    cfg = load_config(tmp_path / "nope.json")
    assert cfg == DEFAULT_CONFIG
    assert cfg is not DEFAULT_CONFIG  # 必須是副本,呼叫端改動不能污染預設值


def test_load_merges_partial_file_with_defaults(tmp_path: Path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"poll_interval": 3.0, "api": {"model": "m1"}}), encoding="utf-8")
    cfg = load_config(p)
    assert cfg["poll_interval"] == 3.0
    assert cfg["api"]["model"] == "m1"
    assert cfg["api"]["base_url"] == DEFAULT_CONFIG["api"]["base_url"]  # 缺的欄位補預設
    assert cfg["hotkey"] == "ctrl+space"


def test_save_then_load_roundtrip(tmp_path: Path):
    p = tmp_path / "config.json"
    cfg = load_config(p)
    cfg["overlay_position"] = {"x": 100, "y": 200}
    cfg["poll_interval"] = 6.5
    save_config(p, cfg)
    reloaded = load_config(p)
    assert reloaded["overlay_position"] == {"x": 100, "y": 200}
    assert reloaded["poll_interval"] == 6.5


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
    assert cfg["api"]["provider"] == "custom"  # 舊使用者的自架端點設定原封不動繼續用


def test_load_config_without_api_block_keeps_default_provider(tmp_path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"hotkey": "f8"}), encoding="utf-8")
    cfg = load_config(p)
    assert cfg["api"]["provider"] == "openai"


def test_is_configured():
    cfg = load_config(Path("nope.json"))
    assert not is_configured(cfg)  # model 空
    cfg["api"].update(provider="openai", model="gpt-x", api_key="sk-1")
    assert is_configured(cfg)
    cfg["api"]["api_key"] = ""
    assert not is_configured(cfg)  # openai/claude 需要金鑰
    cfg["api"].update(provider="custom", model="m", base_url="http://127.0.0.1:8000")
    assert is_configured(cfg)  # custom 不需金鑰，需 base_url
    cfg["api"]["base_url"] = ""
    assert not is_configured(cfg)
