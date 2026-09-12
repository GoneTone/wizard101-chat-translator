"""i18n 核心：語言檔一致性、fallback 行為、系統語言配對。"""
import json
from pathlib import Path

import langcodes
import pytest

from src import i18n


@pytest.fixture(autouse=True)
def restore_language():
    """還原全域語言，並清掉快取 —— 測試會往 _cache 塞假 key，不清會污染後續測試。"""
    before = i18n.current_language()
    yield
    i18n.forget_catalogs()
    i18n.set_language(before)


@pytest.fixture
def fake_catalog(monkeypatch, tmp_path):
    """把語言檔目錄換成 tmp，回傳「寫一個語言檔」的函式。

    要驗證的正是「新增語言只需丟一個語言檔」，所以刻意不碰真正的 src/i18n/*.json。"""
    def write(code: str, strings: dict) -> None:
        (tmp_path / f"{code}.json").write_text(
            json.dumps(strings, ensure_ascii=False), encoding="utf-8")
        i18n.forget_catalogs()

    monkeypatch.setattr(i18n, "bundle_dir", lambda name: tmp_path)
    i18n.forget_catalogs()
    yield write
    i18n.forget_catalogs()


def _load_raw(code: str) -> dict:
    path = Path(i18n.__file__).parent / f"{code}.json"
    return json.loads(path.read_text(encoding="utf-8"))


def test_every_language_has_the_same_keys():
    source = set(_load_raw(i18n.SOURCE_LANGUAGE))
    for code in i18n.available_languages():
        assert set(_load_raw(code)) == source, f"{code} 的 key 與來源語言不一致"


def test_placeholders_match_across_languages():
    import string

    def placeholders(text: str) -> set[str]:
        return {name for _, name, _, _ in string.Formatter().parse(text) if name}

    source = _load_raw(i18n.SOURCE_LANGUAGE)
    for code in i18n.available_languages():
        strings = _load_raw(code)
        for key, template in source.items():
            assert placeholders(strings[key]) == placeholders(template), \
                f"{code} 的 {key} 變數與來源語言不一致"


def test_language_files_have_no_duplicate_keys():
    for code in i18n.available_languages():
        path = Path(i18n.__file__).parent / f"{code}.json"
        # 重複 key 在 json.load 下會被靜默保留後者，用 hook 明確擋下
        with path.open(encoding="utf-8") as handle:
            json.load(handle, object_pairs_hook=i18n._no_duplicates)


def test_t_returns_current_language_string():
    i18n.set_language("en-US")
    assert i18n.t("app.name") == "Wizard101 Chat Translator"
    i18n.set_language("zh-CN")
    assert i18n.t("app.name") == "Wizard101 对话翻译助手"


def test_t_formats_named_placeholders():
    i18n.set_language("zh-TW")
    i18n._load("zh-TW")["test.greet"] = "你好 {name}"
    assert i18n.t("test.greet", name="Amy") == "你好 Amy"


def test_fallback_order_is_current_then_english_then_source():
    i18n.set_language("zh-CN")
    assert i18n.fallback_order() == ["zh-CN", "en-US", "zh-TW"]
    i18n.set_language("en-US")
    assert i18n.fallback_order() == ["en-US", "zh-TW"]      # 當前語言就是英文，不重複查
    i18n.set_language("zh-TW")
    assert i18n.fallback_order() == ["zh-TW", "en-US"]


def test_missing_key_falls_back_to_english_before_source():
    # 缺翻譯優先退英文：這條繁中也有，取的仍必須是英文那份
    i18n.set_language("zh-CN")
    i18n._load("en-US")["test.partial"] = "English copy"
    i18n._load(i18n.SOURCE_LANGUAGE)["test.partial"] = "繁中文案"
    assert i18n.t("test.partial") == "English copy"


def test_missing_key_falls_back_to_source_language_last():
    # 英文還沒跟上時（新文案一定先進來源語言）退繁中當保底
    i18n.set_language("zh-CN")
    i18n._load(i18n.SOURCE_LANGUAGE)["test.only_source"] = "只有來源語言有"
    assert i18n.t("test.only_source") == "只有來源語言有"


def test_missing_everywhere_returns_the_key_itself():
    i18n.set_language("en-US")
    assert i18n.t("test.nowhere") == "test.nowhere"


def test_broken_placeholder_falls_back_to_the_next_language():
    # 譯者把 {count} 打成 {conut}：該語言的字串無法 format，退 fallback 順序的下一個語言
    i18n.set_language("zh-CN")
    i18n._load("zh-CN")["test.count"] = "共 {conut} 条"
    i18n._load("en-US")["test.count"] = "{count} in total"
    assert i18n.t("test.count", count=3) == "3 in total"


def test_unknown_language_code_falls_back_to_default():
    i18n.set_language("fr-FR")
    assert i18n.current_language() == i18n.DEFAULT_LANGUAGE


@pytest.mark.parametrize("tag, expected", [
    ("zh-TW", "zh-TW"),
    ("zh-HK", "zh-TW"),      # 港澳沒有自己的語言檔，CLDR 距離最近的是繁中
    ("zh-MO", "zh-TW"),
    ("zh-Hant", "zh-TW"),    # 只指字集、不指地區
    ("zh-CN", "zh-CN"),
    ("zh-SG", "zh-CN"),
    ("zh-Hans", "zh-CN"),
    ("en-US", "en-US"),
    ("en-GB", "en-US"),      # 同語言的其他地區變體
    ("ja-JP", "en-US"),      # 沒有夠近的語言檔：退預設語言
    ("xx-XX", "en-US"),      # 不存在的語言
    ("", "en-US"),           # 標籤根本解析不了
])
def test_best_match_picks_the_closest_catalog(tag, expected):
    assert i18n.best_match([tag]) == expected


def test_best_match_follows_the_preference_order():
    # 使用者偏好清單依序找，第一個有夠近語言檔的標籤勝出（日文沒有語言檔，跳過）
    assert i18n.best_match(["ja-JP", "zh-CN", "en-US"]) == "zh-CN"


def test_best_match_falls_back_to_default_when_nothing_is_close():
    assert i18n.best_match([]) == i18n.DEFAULT_LANGUAGE
    assert i18n.best_match(["ja-JP", "ko-KR"]) == i18n.DEFAULT_LANGUAGE


def test_system_preferences_are_valid_language_tags():
    # 實機檢查：Windows 回的必須是配對得動的 BCP-47 標籤（LCID 那條路會給 zh_CHT 這種
    # 非標準名稱，配對時會被當成單純的 zh 而對錯字集）
    tags = i18n.preferred_ui_languages()
    assert tags, "系統至少會有一個介面語言"
    assert all(langcodes.tag_is_valid(tag) for tag in tags), tags
    assert i18n.detect_system_language() in i18n.available_languages()


def test_languages_are_listed_as_endonyms():
    # 刻意不列舉語言：多一個語言檔就要改測試的話，「新增語言不必動程式碼」就破功了。
    # 抽驗永遠存在的來源語言，名稱必須來自它自己的 language.name。
    languages = i18n.available_languages()
    assert list(languages) == sorted(languages)     # 語言碼字母序
    assert languages[i18n.SOURCE_LANGUAGE] == "繁體中文（台灣）"


def test_every_language_file_declares_its_own_metadata():
    for code, name in i18n.available_languages().items():
        assert name and name != code, f"{code} 缺 language.name"
        assert i18n.font_family(code), f"{code} 缺 language.font"


def test_a_new_language_file_needs_no_code_change(fake_catalog):
    # 本測試就是「新增語言只要丟一個語言檔」的實證：程式碼裡沒有任何 ja-JP 的痕跡
    fake_catalog("en-US", {"language.name": "English", "language.font": "Segoe UI"})
    fake_catalog("ja-JP", {"language.name": "日本語", "language.font": "Yu Gothic UI"})

    assert i18n.available_languages() == {"en-US": "English", "ja-JP": "日本語"}
    assert i18n.language_name("ja-JP") == "日本語"
    assert i18n.font_family("ja-JP") == "Yu Gothic UI"
    assert i18n.best_match(["ja-JP"]) == "ja-JP"   # 系統語言配對也自動認得，語言檔不必宣告什麼


def test_same_language_different_scripts_are_told_apart(fake_catalog):
    # 繁簡共用語言子標籤 zh，靠 CLDR 的字集資料分辨，語言檔不必宣告要認領哪些 locale
    fake_catalog("zh-TW", {"language.name": "繁體中文（台灣）"})
    fake_catalog("zh-CN", {"language.name": "简体中文（中国）"})

    assert i18n.best_match(["zh-HK"]) == "zh-TW"
    assert i18n.best_match(["zh-SG"]) == "zh-CN"


def test_regional_variants_are_told_apart(fake_catalog):
    # 同字集不同地區（pt-BR／pt-PT）同樣由 CLDR 距離決定，莫三比克葡語靠向葡萄牙
    fake_catalog("pt-BR", {"language.name": "Português (Brasil)"})
    fake_catalog("pt-PT", {"language.name": "Português (Portugal)"})

    assert i18n.best_match(["pt-MZ"]) == "pt-PT"
    assert i18n.best_match(["pt-BR"]) == "pt-BR"


def test_missing_metadata_never_borrows_another_language(fake_catalog):
    # metadata 不走 t() 的 fallback：忘了填 language.name 就顯示語言碼，
    # 顯示成 "English" 反而看不出是漏填。
    fake_catalog("en-US", {"language.name": "English", "language.font": "Segoe UI"})
    fake_catalog("ja-JP", {"app.name": "ウィザード"})

    assert i18n.available_languages()["ja-JP"] == "ja-JP"
    assert i18n.font_family("ja-JP") is None


def test_translators_are_never_borrowed_from_another_language(fake_catalog):
    # 譯者掛名借到別的語言就是把功勞掛錯人，所以與其他 metadata 一樣不走 t() 的 fallback。
    fake_catalog("en-US", {"language.name": "English",
                           "language.translators": "[Someone](https://example.com)"})
    fake_catalog("ja-JP", {"language.name": "日本語"})

    assert i18n.translators("en-US") == "[Someone](https://example.com)"
    assert i18n.translators("ja-JP") == ""


def test_translators_is_optional_metadata():
    # 來源語言是開發者自己寫的、沒有譯者；欄位空著是正常狀態，不該被當成漏填。
    for code in i18n.available_languages():
        assert isinstance(i18n.translators(code), str)


def test_unknown_language_code_is_rejected_by_the_scanned_list(fake_catalog):
    fake_catalog("en-US", {"language.name": "English"})
    i18n.set_language("ja-JP")   # 目錄裡沒有 ja-JP.json
    assert i18n.current_language() == i18n.DEFAULT_LANGUAGE


def test_set_language_keeps_previous_language_when_load_fails(monkeypatch):
    # 切換失敗（缺檔／內容損毀）時，_current 不該被改到一個永遠載入不了的語言，
    # 否則之後每一次 t() 都會炸例外。
    i18n.set_language("zh-TW")
    original_load = i18n._load

    def failing_load(code):
        if code == "en-US":
            raise ValueError("corrupted catalog")
        return original_load(code)

    monkeypatch.setattr(i18n, "_load", failing_load)
    with pytest.raises(ValueError):
        i18n.set_language("en-US")
    assert i18n.current_language() == "zh-TW"
