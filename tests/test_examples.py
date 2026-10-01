"""範例集：生成請求、驗證與合併的純邏輯測試。"""
from src.translation.examples import (
    EMPTY,
    EXAMPLE_REVISION,
    GAME_LANGUAGE_EXAMPLES,
    SOURCE_LINES,
    ExampleSet,
    examples_fingerprint,
    generation_request,
    parse_generated,
)

GOOD = "\n".join([
    "1. [Amy] 知らないよ、Kai と新しい鎧を手に入れて、"
    "巨像大道（Colossus Boulevard）で火猫（Fire Cat）を覚えた、笑",
    "2. Kai が火猫（Fire Cat）を教えてくれた！巨像大道（Colossus Boulevard）で {0} ゴールドを獲得した。",
    "3. 火猫（Fire Cat）に話しかける",
    "4. 巨像大道（Colossus Boulevard）へ行く",
    "5. そしてあなたは",
])


def test_parse_generated_accepts_every_path():
    examples, failures = parse_generated(GOOD)
    assert examples.complete and failures == []
    assert examples.incoming == (SOURCE_LINES[0], GOOD.splitlines()[0][3:])
    assert examples.region[0].startswith("1. Talk to the Fire Cat")
    assert examples.region[1].splitlines()[2] == "3. そしてあなたは"


def test_parse_generated_drops_only_the_failing_path():
    bad_system = GOOD.replace("{0} ゴールド", "ゴールド")
    examples, failures = parse_generated(bad_system)
    assert examples.system is None and examples.incoming is not None and examples.region is not None
    assert any("line 2" in f for f in failures)


def test_parse_generated_requires_the_original_in_parentheses():
    examples, _ = parse_generated(GOOD.replace("火猫（Fire Cat）を覚えた", "火猫を覚えた"))
    assert examples.incoming is None


def test_parse_generated_rejects_a_wrong_line_count():
    examples, failures = parse_generated("\n".join(GOOD.splitlines()[:3]))
    assert examples == EMPTY and failures


def test_parse_generated_ignores_a_preamble_and_code_fences():
    examples, _ = parse_generated("以下是翻譯：\n```\n" + GOOD + "\n```")
    assert examples.complete


def test_merge_keeps_existing_paths_and_fills_missing_ones():
    a = ExampleSet(("s", "o"), None, None)
    b = ExampleSet(("x", "y"), ("s2", "o2"), None)
    assert a.merge(b) == ExampleSet(("s", "o"), ("s2", "o2"), None)


def test_summary_and_digest():
    assert ExampleSet(("s", "o"), None, None).summary() == "incoming=ok system=failed region=failed"
    assert ExampleSet(("s", "o"), None, None).digest() != EMPTY.digest()


def test_fingerprint_contains_service_target_and_revision_but_no_key():
    api = {"provider": "custom", "base_url": "https://x", "model": "m", "api_key": "secret"}
    fp = examples_fingerprint(api, "日本語")
    assert fp == f"custom|https://x|m|日本語|e{EXAMPLE_REVISION}" and "secret" not in fp


def test_fingerprint_ignores_surrounding_whitespace_in_the_target():
    api = {"provider": "openai", "model": "m"}
    assert examples_fingerprint(api, " 日本語 ") == examples_fingerprint(api, "日本語")


def test_generation_request_names_the_target_and_numbers_the_sources():
    system, turns = generation_request("Deutsch")
    assert "Deutsch" in system and "1. [Amy]" in system
    numbered = "\n".join(f"{i}. {s}" for i, s in enumerate(SOURCE_LINES, 1))
    assert turns == [{"role": "user", "content": numbered}]


def test_game_language_examples_translate_into_english():
    assert "Fire Cat (火" in GAME_LANGUAGE_EXAMPLES.incoming[1]
    assert GAME_LANGUAGE_EXAMPLES.complete
