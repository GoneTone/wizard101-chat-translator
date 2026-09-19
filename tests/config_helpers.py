"""測試用的設定輔助：一行生出一份 API 設定完整的 cfg。"""
import copy

from src.config import DEFAULT_CONFIG

# 各服務商「剛好填滿必填欄位」的最小值；自訂端點不需金鑰，欄位最少，故為預設。
_MINIMAL = {
    "custom": {"base_url": "http://x", "model": "m"},
    "openai": {"api_key": "k", "model": "m"},
    "claude": {"api_key": "k", "model": "m"},
}


def configured_cfg(provider: str = "custom", **fields) -> dict:
    """一份 API 設定完整的 cfg 深副本；`fields` 覆寫該服務商的欄位。"""
    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["api"]["provider"] = provider
    cfg["api"][provider].update({**_MINIMAL[provider], **fields})
    return cfg
