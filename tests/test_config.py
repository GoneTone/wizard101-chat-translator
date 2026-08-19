import json
from pathlib import Path

from src.config import DEFAULT_CONFIG, load_config, save_config


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
    cfg["chat_region"] = {"left": 10, "top": 20, "width": 300, "height": 150}
    save_config(p, cfg)
    assert load_config(p)["chat_region"] == {"left": 10, "top": 20, "width": 300, "height": 150}
