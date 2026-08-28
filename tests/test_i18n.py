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
    i18n._cache.clear()
    i18n.set_language(before)


def _load_raw(code: str) -> dict:
    path = Path(i18n.__file__).parent / f"{code}.json"
    return json.loads(path.read_text(encoding="utf-8"))


def test_every_language_has_the_same_keys():
    source = set(_load_raw(i18n.SOURCE_LANGUAGE))
    for code in i18n.LANGUAGES:
        assert set(_load_raw(code)) == source, f"{code} 的 key 與來源語言不一致"


def test_placeholders_match_across_languages():
    import string

    def placeholders(text: str) -> set[str]:
        return {name for _, name, _, _ in string.Formatter().parse(text) if name}

    source = _load_raw(i18n.SOURCE_LANGUAGE)
    for code in i18n.LANGUAGES:
        strings = _load_raw(code)
        for key, template in source.items():
            assert placeholders(strings[key]) == placeholders(template), \
                f"{code} 的 {key} 變數與來源語言不一致"


def test_language_files_have_no_duplicate_keys():
    for code in i18n.LANGUAGES:
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


def test_missing_key_falls_back_to_source_language():
    i18n.set_language("en")
    i18n._load(i18n.SOURCE_LANGUAGE)["test.only_source"] = "只有來源語言有"
    assert i18n.t("test.only_source") == "只有來源語言有"


def test_missing_everywhere_returns_the_key_itself():
    i18n.set_language("en")
    assert i18n.t("test.nowhere") == "test.nowhere"


def test_broken_placeholder_falls_back_to_source_language():
    # 譯者把 {count} 打成 {conut}：該語言的字串無法 format，退回來源語言
    i18n.set_language("en")
    i18n._load(i18n.SOURCE_LANGUAGE)["test.count"] = "共 {count} 則"
    i18n._load("en")["test.count"] = "total {conut}"
    assert i18n.t("test.count", count=3) == "共 3 則"


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
    ("ja_JP", "en"),
    ("", "en"),
])
def test_map_locale_name(name, expected):
    assert i18n.map_locale_name(name) == expected


def test_languages_are_listed_as_endonyms():
    assert i18n.LANGUAGES == {
        "zh-TW": "繁體中文（台灣）",
        "zh-CN": "简体中文（中国）",
        "en": "English",
    }
