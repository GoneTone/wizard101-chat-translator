"""i18n 核心：語言檔一致性、fallback 行為、系統語言映射。"""
import json
from pathlib import Path

import pytest

from src import i18n


@pytest.fixture(autouse=True)
def restore_language():
    """還原全域語言，並清掉快取——測試會往 _cache 塞假 key，不清會污染後續測試。"""
    before = i18n.current_language()
    yield
    i18n.forget_catalogs()
    i18n.set_language(before)


@pytest.fixture
def fake_catalog(monkeypatch, tmp_path):
    """把語言檔目錄換成 tmp，回傳「寫一個語言檔」的函式。

    這組測試要驗證的正是「新增語言只需要丟一個語言檔」，所以刻意不碰真正的
    src/i18n/*.json——用假目錄才能演練還不存在的語言。"""
    def write(code: str, strings: dict) -> None:
        (tmp_path / f"{code}.json").write_text(
            json.dumps(strings, ensure_ascii=False), encoding="utf-8")
        i18n.forget_catalogs()

    monkeypatch.setattr(i18n, "_i18n_dir", lambda: tmp_path)
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
    i18n.set_language("en")
    assert i18n.t("app.name") == "Wizard101 Chat Translator"
    i18n.set_language("zh-CN")
    assert i18n.t("app.name") == "Wizard101 对话翻译助手"


def test_t_formats_named_placeholders():
    i18n.set_language("zh-TW")
    i18n._load("zh-TW")["test.greet"] = "你好 {name}"
    assert i18n.t("test.greet", name="Amy") == "你好 Amy"


def test_fallback_order_is_current_then_english_then_source():
    i18n.set_language("zh-CN")
    assert i18n.fallback_order() == ["zh-CN", "en", "zh-TW"]
    i18n.set_language("en")
    assert i18n.fallback_order() == ["en", "zh-TW"]      # 當前語言就是英文，不重複查
    i18n.set_language("zh-TW")
    assert i18n.fallback_order() == ["zh-TW", "en"]


def test_missing_key_falls_back_to_english_before_source():
    # 缺翻譯優先退英文：這條繁中也有，取的仍必須是英文那份
    i18n.set_language("zh-CN")
    i18n._load("en")["test.partial"] = "English copy"
    i18n._load(i18n.SOURCE_LANGUAGE)["test.partial"] = "繁中文案"
    assert i18n.t("test.partial") == "English copy"


def test_missing_key_falls_back_to_source_language_last():
    # 英文還沒跟上時（新文案一定先進來源語言）退繁中當保底
    i18n.set_language("zh-CN")
    i18n._load(i18n.SOURCE_LANGUAGE)["test.only_source"] = "只有來源語言有"
    assert i18n.t("test.only_source") == "只有來源語言有"


def test_missing_everywhere_returns_the_key_itself():
    i18n.set_language("en")
    assert i18n.t("test.nowhere") == "test.nowhere"


def test_broken_placeholder_falls_back_to_the_next_language():
    # 譯者把 {count} 打成 {conut}：該語言的字串無法 format，退 fallback 順序的下一個語言
    i18n.set_language("zh-CN")
    i18n._load("zh-CN")["test.count"] = "共 {conut} 条"
    i18n._load("en")["test.count"] = "{count} in total"
    assert i18n.t("test.count", count=3) == "3 in total"


def test_unknown_language_code_falls_back_to_default():
    i18n.set_language("fr-FR")
    assert i18n.current_language() == i18n.DEFAULT_LANGUAGE


@pytest.mark.parametrize("name, expected", [
    ("zh_TW", "zh-TW"),
    ("zh_HK", "zh-TW"),
    ("zh_MO", "zh-TW"),
    ("zh_CN", "zh-CN"),
    ("zh_SG", "zh-CN"),
    ("en_US", "en"),
    ("xx_XX", "en"),   # 沒有人認領也對不上語言碼：退預設語言
    ("", "en"),
])
def test_map_locale_name(name, expected):
    assert i18n.map_locale_name(name) == expected


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
    # 本測試就是「新增語言只要丟一個語言檔」的實證：程式碼裡沒有任何 ja 的痕跡
    fake_catalog("en", {"language.name": "English", "language.font": "Segoe UI"})
    fake_catalog("ja", {"language.name": "日本語", "language.font": "Yu Gothic UI"})

    assert i18n.available_languages() == {"en": "English", "ja": "日本語"}
    assert i18n.language_name("ja") == "日本語"
    assert i18n.font_family("ja") == "Yu Gothic UI"
    assert i18n.map_locale_name("ja_JP") == "ja"   # 沒寫 locales 也能靠語言前綴命中


def test_declared_locales_win_over_the_language_prefix(fake_catalog):
    # 語言碼與 locale 前綴對不上（pt-BR vs pt_PT）時，靠語言檔自己宣告的 locales
    fake_catalog("en", {"language.name": "English"})
    fake_catalog("pt-BR", {"language.name": "Português (Brasil)",
                           "language.locales": "pt_BR pt_PT"})

    assert i18n.map_locale_name("pt_PT") == "pt-BR"
    assert i18n.map_locale_name("pt_BR") == "pt-BR"


def test_missing_metadata_never_borrows_another_language(fake_catalog):
    # metadata 不走 t() 的 fallback：忘了填 language.name 就顯示語言碼，
    # 顯示成 "English" 反而看不出是漏填。
    fake_catalog("en", {"language.name": "English", "language.font": "Segoe UI"})
    fake_catalog("ja", {"app.name": "ウィザード"})

    assert i18n.available_languages()["ja"] == "ja"
    assert i18n.font_family("ja") is None


def test_unknown_language_code_is_rejected_by_the_scanned_list(fake_catalog):
    fake_catalog("en", {"language.name": "English"})
    i18n.set_language("ja")   # 目錄裡沒有 ja.json
    assert i18n.current_language() == i18n.DEFAULT_LANGUAGE


def test_set_language_keeps_previous_language_when_load_fails(monkeypatch):
    # 切換失敗（缺檔／內容損毀）時，_current 不該被改到一個永遠載入不了的語言，
    # 否則之後每一次 t() 都會炸例外。
    i18n.set_language("zh-TW")
    original_load = i18n._load

    def failing_load(code):
        if code == "en":
            raise ValueError("corrupted catalog")
        return original_load(code)

    monkeypatch.setattr(i18n, "_load", failing_load)
    with pytest.raises(ValueError):
        i18n.set_language("en")
    assert i18n.current_language() == "zh-TW"
