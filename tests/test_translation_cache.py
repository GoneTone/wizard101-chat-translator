"""系統訊息譯文快取：數字正規化、LRU、持久化與指紋失效。"""
import json

import pytest

import src.translation_cache as cache_module
from src.translation_cache import (
    TranslationCache, fingerprint_of, normalize, placeholders_match, restore,
    translate_and_cache,
)


def test_normalize_replaces_numbers_with_placeholders():
    assert normalize("你获得了 39 金币！") == ("你获得了 {0} 金币！", ["39"])


def test_normalize_numbers_the_placeholders_in_order():
    template, nums = normalize("你获得了 3 经验值和 39 金币！")
    assert template == "你获得了 {0} 经验值和 {1} 金币！"
    assert nums == ["3", "39"]


def test_normalize_keeps_decimals_and_thousands_separators_whole():
    assert normalize("你获得了 1,234 金币！") == ("你获得了 {0} 金币！", ["1,234"])
    assert normalize("耗时 3.5 秒") == ("耗时 {0} 秒", ["3.5"])


def test_normalize_leaves_text_without_numbers_untouched():
    assert normalize("熔岩百合") == ("熔岩百合", [])


def test_restore_puts_the_numbers_back():
    assert restore("你獲得了 {0} 金幣！", ["39"]) == "你獲得了 39 金幣！"


def test_restore_follows_the_translated_word_order():
    # 譯文可能調換佔位符順序，回填必須依編號而不是依出現位置
    assert restore("{1} gold and {0} XP", ["3", "39"]) == "39 gold and 3 XP"


def test_normalize_restore_round_trips():
    original = "你获得了 3 经验值和 1,234 金币！"
    template, nums = normalize(original)
    assert restore(template, nums) == original


def test_placeholders_match_accepts_a_faithful_translation():
    assert placeholders_match("你获得了 {0} 金币！", "你獲得了 {0} 金幣！") is True


def test_placeholders_match_rejects_a_dropped_placeholder():
    assert placeholders_match("你获得了 {0} 金币！", "你獲得了金幣！") is False


def test_placeholders_match_rejects_an_invented_placeholder():
    assert placeholders_match("熔岩百合", "熔岩百合 {0}") is False


def test_placeholders_match_rejects_a_duplicated_placeholder():
    assert placeholders_match("{0} 金币", "{0} 金幣 {0}") is False


@pytest.fixture
def cache_path(tmp_path, monkeypatch):
    path = tmp_path / "syscache.json"
    monkeypatch.setattr(cache_module, "CACHE_PATH", path)
    return path


FP = "custom|gemma|繁體中文（台灣）"


def test_get_returns_none_when_empty(cache_path):
    assert TranslationCache(FP).get("你获得了 39 金币！") is None


def test_put_then_get_round_trips_with_the_number_restored(cache_path):
    c = TranslationCache(FP)
    assert c.put("你获得了 39 金币！", "你獲得了 {0} 金幣！") is True
    assert c.get("你获得了 39 金币！") == "你獲得了 39 金幣！"


def test_a_different_number_hits_the_same_entry(cache_path):
    c = TranslationCache(FP)
    c.put("你获得了 39 金币！", "你獲得了 {0} 金幣！")
    assert c.get("你获得了 65 金币！") == "你獲得了 65 金幣！"


def test_put_rejects_a_translation_with_broken_placeholders(cache_path):
    c = TranslationCache(FP)
    assert c.put("你获得了 39 金币！", "你獲得了金幣！") is False
    assert c.get("你获得了 39 金币！") is None


def test_put_expects_the_translation_of_the_normalized_template(cache_path):
    # 呼叫端送進來的譯文是「樣板的譯文」，不是「原文的譯文」
    c = TranslationCache(FP)
    c.put("熔岩百合", "熔岩百合(Lava Lily)")
    assert c.get("熔岩百合") == "熔岩百合(Lava Lily)"


def test_put_takes_the_raw_text_not_the_template(cache_path):
    # put() 內部自己正規化。傳入已含 {0} 的樣板會讓裡面的 0 被當成數字而變成 {{0}}，
    # 之後永遠對不上——呼叫端務必傳原文（見 main._translate_and_cache）。
    c = TranslationCache(FP)
    c.put("你获得了 39 金币！", "你獲得了 {0} 金幣！")
    assert c.get("你获得了 39 金币！") == "你獲得了 39 金幣！"
    assert c.get("你获得了 {0} 金币！") is None


def test_evicts_the_least_recently_used_entry(cache_path, monkeypatch):
    monkeypatch.setattr(cache_module, "MAX_ENTRIES", 2)
    c = TranslationCache(FP)
    c.put("a", "A")
    c.put("b", "B")
    c.get("a")          # a 變成最近使用
    c.put("c", "C")     # 擠掉 b
    assert c.get("a") == "A"
    assert c.get("b") is None
    assert c.get("c") == "C"


def test_flush_writes_the_file_with_the_fingerprint(cache_path):
    c = TranslationCache(FP)
    c.put("熔岩百合", "熔岩百合(Lava Lily)")
    c.flush()
    data = json.loads(cache_path.read_text(encoding="utf-8"))
    assert data["fingerprint"] == FP
    assert data["entries"]["熔岩百合"] == "熔岩百合(Lava Lily)"


def test_load_restores_entries_from_disk(cache_path):
    first = TranslationCache(FP)
    first.put("熔岩百合", "熔岩百合(Lava Lily)")
    first.flush()
    second = TranslationCache(FP)
    second.load()
    assert second.get("熔岩百合") == "熔岩百合(Lava Lily)"


def test_load_discards_everything_when_the_fingerprint_differs(cache_path):
    first = TranslationCache(FP)
    first.put("熔岩百合", "熔岩百合(Lava Lily)")
    first.flush()
    second = TranslationCache("claude|claude-opus-5|日本語")
    second.load()
    assert second.get("熔岩百合") is None


def test_load_survives_a_corrupt_file(cache_path):
    cache_path.write_text("{ not json", encoding="utf-8")
    c = TranslationCache(FP)
    c.load()            # 不得拋例外
    assert c.get("熔岩百合") is None


def test_load_survives_a_structurally_malformed_file(cache_path):
    # entries 語法上是合法 JSON，但形狀不對（list／string 而非 dict）——
    # 截斷寫入或版本不一致的舊檔可能留下這種殘骸，一樣不得拋例外。
    cache_path.write_text(
        json.dumps({"fingerprint": FP, "entries": ["not", "a", "dict"]}),
        encoding="utf-8")
    c = TranslationCache(FP)
    c.load()            # 不得拋例外
    assert c.get("熔岩百合") is None

    cache_path.write_text(
        json.dumps({"fingerprint": FP, "entries": "not a dict either"}),
        encoding="utf-8")
    c2 = TranslationCache(FP)
    c2.load()           # 不得拋例外
    assert c2.get("熔岩百合") is None


def test_load_survives_a_missing_file(cache_path):
    c = TranslationCache(FP)
    c.load()
    assert c.get("熔岩百合") is None


def test_autoflushes_after_enough_new_entries(cache_path, monkeypatch):
    monkeypatch.setattr(cache_module, "FLUSH_EVERY", 2)
    c = TranslationCache(FP)
    c.put("a", "A")
    assert not cache_path.exists()
    c.put("b", "B")
    assert cache_path.exists(), "累積到門檻應自動落盤"


def test_rebind_flushes_the_old_fingerprint_and_starts_empty(cache_path):
    c = TranslationCache(FP)
    c.put("熔岩百合", "熔岩百合(Lava Lily)")
    c.rebind("claude|claude-opus-5|日本語")
    assert c.get("熔岩百合") is None    # 舊快取隨指紋變更清空
    data = json.loads(cache_path.read_text(encoding="utf-8"))
    assert data["fingerprint"] == FP    # 落盤的是變更「前」的指紋
    assert data["entries"]["熔岩百合"] == "熔岩百合(Lava Lily)"


def test_rebind_is_a_no_op_when_the_fingerprint_is_unchanged(cache_path):
    c = TranslationCache(FP)
    c.put("熔岩百合", "熔岩百合(Lava Lily)")
    c.rebind(FP)
    assert not cache_path.exists()      # 沒變更就不觸發落盤
    assert c.get("熔岩百合") == "熔岩百合(Lava Lily)"


class FakeTranslator:
    """記錄收到的每一次翻譯請求；translations 是「送進去的文字→回傳的譯文」對照表。"""

    def __init__(self, translations):
        self.translations = dict(translations)
        self.requested: list[str] = []

    def translate_system_message(self, text):
        self.requested.append(text)
        return self.translations[text]


def test_translate_and_cache_stores_the_normalized_template(cache_path):
    translator = FakeTranslator({"你获得了 {0} 金币！": "你獲得了 {0} 金幣！"})
    cache = TranslationCache(FP)
    result = translate_and_cache(translator, cache, "你获得了 39 金币！")
    assert result == "你獲得了 39 金幣！"
    assert translator.requested == ["你获得了 {0} 金币！"]   # 送去翻譯的是樣板，不是原文
    assert cache.get("你获得了 39 金币！") == "你獲得了 39 金幣！"
    assert cache.get("你获得了 65 金币！") == "你獲得了 65 金幣！"   # 換個數字也命中同一筆


def test_translate_and_cache_falls_back_when_the_model_mangles_a_placeholder(cache_path):
    # 模型把樣板譯文裡的佔位符弄丟：不快取，改用原文直翻一次，正確性優先於命中率
    translator = FakeTranslator({
        "你获得了 {0} 金币！": "你獲得了金幣！",        # 樣板譯文：佔位符不見了
        "你获得了 39 金币！": "你獲得了 39 金幣！",     # 直翻原文（fallback）的結果
    })
    cache = TranslationCache(FP)
    result = translate_and_cache(translator, cache, "你获得了 39 金币！")
    assert result == "你獲得了 39 金幣！"
    assert translator.requested == ["你获得了 {0} 金币！", "你获得了 39 金币！"]
    assert cache.get("你获得了 39 金币！") is None   # 沒存進快取


def test_fingerprint_never_contains_the_api_key():
    fp = fingerprint_of("custom", "gemma-4-26b-a4b", "繁體中文（台灣）")
    assert "gemma-4-26b-a4b" in fp
    assert "sk-" not in fp


def test_cache_file_never_contains_the_api_key(cache_path):
    c = TranslationCache(fingerprint_of("custom", "gemma", "繁體中文（台灣）"))
    c.put("熔岩百合", "熔岩百合(Lava Lily)")
    c.flush()
    assert "sk-" not in cache_path.read_text(encoding="utf-8")
