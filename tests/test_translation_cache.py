"""系統訊息譯文快取：數字正規化、LRU、持久化與指紋失效。"""
from src.translation_cache import normalize, placeholders_match, restore


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
