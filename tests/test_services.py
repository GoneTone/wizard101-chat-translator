"""翻譯服務資料層測試。"""
from src.config import is_configured
from tests.config_helpers import configured_cfg


def test_configured_cfg_passes_the_completeness_check():
    assert is_configured(configured_cfg())
    assert is_configured(configured_cfg("openai"))
    assert is_configured(configured_cfg("claude"))


def test_configured_cfg_returns_an_independent_copy():
    first = configured_cfg()
    first["api"]["custom"]["model"] = "mutated"
    assert configured_cfg()["api"]["custom"]["model"] == "m"
