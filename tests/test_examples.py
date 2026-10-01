"""範例集：生成請求、驗證與合併的純邏輯測試。"""
from src.translation.examples import (
    EMPTY,
    EXAMPLE_REVISION,
    GAME_LANGUAGE_EXAMPLES,
    MAX_ENTRIES,
    SOURCE_LINES,
    ExampleCoordinator,
    ExampleSet,
    ExampleStore,
    examples_fingerprint,
    generation_request,
    parse_generated,
)
from src.translation.postprocess import number_lines, numbered_entries, unnumber_lines

GOOD = "\n".join([
    "1. [Amy] 知らないよ、Kai と新しい鎧を手に入れて、"
    "巨像大道（Colossus Boulevard）で火猫（Fire Cat）を覚えた、笑",
    "2. Kai が火猫（Fire Cat）を教えてくれた！巨像大道（Colossus Boulevard）で {0} ゴールドを獲得した。",
    "3. 火猫（Fire Cat）に話しかける",
    "4. 巨像大道（Colossus Boulevard）へ行く",
    "5. オプション",
    "6. そしてあなたは",
])


def test_parse_generated_accepts_every_path():
    examples, failures = parse_generated(GOOD)
    assert examples.complete and failures == []
    assert examples.incoming == (SOURCE_LINES[0], GOOD.splitlines()[0][3:])
    assert examples.region[0] == "\n".join(SOURCE_LINES[2:])     # 框選範例不帶編號
    assert examples.region[1].splitlines() == [
        "火猫（Fire Cat）に話しかける", "巨像大道（Colossus Boulevard）へ行く",
        "オプション", "そしてあなたは"]


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


def test_parse_generated_names_the_failed_region_condition():
    _, failures = parse_generated(GOOD.replace("3. 火猫（Fire Cat）に話しかける", "3. 火猫に話しかける"))
    assert failures == ["line 3: needs Fire Cat in parentheses"]


def test_parse_generated_tells_a_wrong_line_count_from_an_untranslated_line():
    _, failures = parse_generated("\n".join(line[3:] for line in GOOD.splitlines()[:3]))
    assert failures == [f"line count 3 != {len(SOURCE_LINES)}"]
    _, failures = parse_generated(GOOD.replace("6. そしてあなたは", "6. and then you must"))
    assert failures == ["line 6 untranslated"]


def test_parse_generated_accepts_an_interface_label_spelled_like_its_source():
    # 法文的 OPTIONS 就是 OPTIONS：照抄是正確譯文，不可當成沒翻而整份作廢
    examples, failures = parse_generated(GOOD.replace("5. オプション", "5. OPTIONS"))
    assert examples.complete and failures == []
    assert examples.region[1].splitlines()[2] == "OPTIONS"


def test_parse_generated_treats_a_missing_interface_label_as_untranslated():
    missing_label = "\n".join(line for line in GOOD.splitlines() if not line.startswith("5."))
    assert parse_generated(missing_label) == (EMPTY, ["line 5 untranslated"])


def test_parse_generated_rejects_parentheses_on_the_interface_label():
    examples, failures = parse_generated(GOOD.replace("5. オプション", "5. オプション（OPTIONS）"))
    assert examples.region is None
    assert failures == ["line 5: interface label must not have parentheses"]


def test_parse_generated_treats_a_line_echoing_its_source_as_missing():
    missing_line_3 = "\n".join(line for line in GOOD.splitlines() if not line.startswith("3."))
    assert parse_generated(missing_line_3) == (EMPTY, ["line 3 untranslated"])


def test_parse_generated_counts_a_numbered_preamble_as_a_failure():
    shifted = "\n".join(f"{i + 1}.{line[2:]}" for i, line in enumerate(GOOD.splitlines(), 1))
    examples, failures = parse_generated("1. 以下是翻譯：\n" + shifted)
    assert failures and examples.incoming is None and not examples.complete


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


def test_game_language_region_example_keeps_the_interface_label_bare():
    source, output = GAME_LANGUAGE_EXAMPLES.region
    assert source.splitlines()[2] == "选项" and output.splitlines()[2] == "Options"


def test_game_language_region_examples_have_no_surrounding_newlines():
    assert all(text == text.strip("\n") for text in GAME_LANGUAGE_EXAMPLES.region)


def test_store_round_trips_and_keeps_several_fingerprints(tmp_path):
    store = ExampleStore(tmp_path / "ex.json")
    a, b = ExampleSet(("s", "o"), None, None), ExampleSet(None, ("s", "o"), None)
    store.put("fp-a", a)
    store.put("fp-b", b)
    assert ExampleStore(tmp_path / "ex.json").get("fp-a") == a
    assert store.get("fp-b") == b and store.get("fp-c") is None


def test_store_drops_the_oldest_beyond_the_limit(tmp_path):
    examples = ExampleSet(("s", "o"), None, None)
    store = ExampleStore(tmp_path / "ex.json")
    for i in range(MAX_ENTRIES + 1):
        store.put(f"fp-{i}", examples)
    assert store.get("fp-0") is None and store.get(f"fp-{MAX_ENTRIES}") is not None


def test_re_putting_a_fingerprint_moves_it_to_the_end(tmp_path):
    examples = ExampleSet(("s", "o"), None, None)
    store = ExampleStore(tmp_path / "ex.json")
    for i in range(MAX_ENTRIES):
        store.put(f"fp-{i}", examples)
    store.put("fp-0", examples)
    store.put("fp-new", examples)
    assert store.get("fp-0") is not None and store.get("fp-1") is None


def test_a_failed_write_leaves_no_temp_file(tmp_path, monkeypatch):
    from src.translation import examples as module

    def fail(*_args):
        raise OSError("disk full")

    monkeypatch.setattr(module.os, "replace", fail)
    ExampleStore(tmp_path / "ex.json").put("fp", ExampleSet(("s", "o"), None, None))
    assert list(tmp_path.iterdir()) == []


def test_store_rejects_a_pair_that_is_not_a_list_of_two_strings(tmp_path):
    path = tmp_path / "ex.json"
    path.write_text('{"entries": {"fp": {"incoming": "ab", "system": null, "region": null}}}',
                    encoding="utf-8")
    assert ExampleStore(path).get("fp") is None


def test_store_treats_a_corrupt_file_as_empty(tmp_path):
    path = tmp_path / "ex.json"
    path.write_text("{not json", encoding="utf-8")
    assert ExampleStore(path).get("fp") is None


def test_store_does_not_write_an_empty_set(tmp_path):
    path = tmp_path / "ex.json"
    ExampleStore(path).put("fp", EMPTY)
    assert not path.exists()


class FakeTr:
    def __init__(self, fp, result=None, error=None):
        self.examples_fingerprint, self.examples = fp, None
        self.result, self.error, self.calls = result, error, 0

    def set_examples(self, ex, fp):
        if fp != self.examples_fingerprint:
            return False
        self.examples = ex
        return True

    def generate_examples(self):
        self.calls += 1
        if self.error:
            raise self.error
        return self.examples_fingerprint, self.result

    def describe(self):
        return "provider=custom, model=m"


SET = ExampleSet(("s", "o"), ("s", "o"), ("s", "o"))


def coordinator(tmp_path, applied=None):
    applied = [] if applied is None else applied
    return ExampleCoordinator(ExampleStore(tmp_path / "ex.json"), post=lambda job: job(),
                              spawn=lambda job: job(), on_applied=applied.append)


def test_cache_hit_applies_without_generating(tmp_path):
    applied = []
    co = coordinator(tmp_path, applied)
    co._store.put("fp", SET)
    tr = FakeTr("fp", SET)
    co.ensure(tr)
    assert tr.examples == SET and tr.calls == 0 and applied == [tr]


def test_a_miss_generates_stores_and_applies(tmp_path):
    applied = []
    co = coordinator(tmp_path, applied)
    tr = FakeTr("fp", SET)
    co.ensure(tr)
    assert tr.examples == SET and tr.calls == 1 and applied == [tr]
    assert ExampleStore(tmp_path / "ex.json").get("fp") == SET


def test_two_translators_on_the_same_service_share_one_generation(tmp_path):
    jobs = []
    co = ExampleCoordinator(ExampleStore(tmp_path / "ex.json"), post=lambda j: j(), spawn=jobs.append)
    a, b = FakeTr("fp", SET), FakeTr("fp", SET)
    co.ensure(a)
    co.ensure(b)
    assert len(jobs) == 1
    jobs[0]()
    assert a.examples == SET and b.examples == SET


def test_a_failed_fingerprint_is_not_retried_in_the_same_run(tmp_path):
    co = coordinator(tmp_path)
    tr = FakeTr("fp", error=RuntimeError("offline"))
    co.ensure(tr)
    co.ensure(tr)
    assert tr.calls == 1 and tr.examples is None


def test_an_empty_result_counts_as_a_failure(tmp_path):
    co = coordinator(tmp_path)
    tr = FakeTr("fp", EMPTY)
    co.ensure(tr)
    co.ensure(tr)
    assert tr.calls == 1 and ExampleStore(tmp_path / "ex.json").get("fp") is None


def test_a_result_for_an_outdated_fingerprint_is_stored_but_not_applied(tmp_path):
    jobs = []
    co = ExampleCoordinator(ExampleStore(tmp_path / "ex.json"), post=lambda j: j(), spawn=jobs.append)
    tr = FakeTr("fp-ja", SET)
    co.ensure(tr)
    tr.examples_fingerprint = "fp-zh"  # 生成途中使用者切換了目標語言
    tr.generate_examples = lambda: ("fp-ja", SET)
    jobs[0]()
    assert tr.examples is None
    assert ExampleStore(tmp_path / "ex.json").get("fp-ja") == SET


def test_a_result_is_applied_by_its_own_fingerprint_not_the_requested_one(tmp_path):
    jobs = []
    store = ExampleStore(tmp_path / "ex.json")
    co = ExampleCoordinator(store, post=lambda j: j(), spawn=jobs.append)
    a, b = FakeTr("fp-ja", SET), FakeTr("fp-zh", SET)
    co.ensure(a)
    co.ensure(b)
    a.generate_examples = lambda: ("fp-zh", SET)
    jobs[0]()
    assert b.examples == SET and a.examples is None
    assert store.get("fp-zh") == SET and store.get("fp-ja") is None


def test_on_applied_is_skipped_when_set_examples_rejects(tmp_path):
    applied = []
    co = coordinator(tmp_path, applied)
    co._store.put("fp", SET)
    tr = FakeTr("fp", SET)
    tr.set_examples = lambda ex, fp: False
    co.ensure(tr)
    assert applied == []


def test_registering_the_same_translator_twice_registers_and_generates_once(tmp_path):
    applied = []
    co = coordinator(tmp_path, applied)
    tr = FakeTr("fp", SET)
    co.ensure(tr)
    co.ensure(tr)
    assert applied == [tr, tr] and len(co._translators) == 1
    assert tr.calls == 1


def test_a_failure_after_the_settings_changed_does_not_block_a_later_retry(tmp_path):
    jobs = []
    co = ExampleCoordinator(ExampleStore(tmp_path / "ex.json"), post=lambda j: j(), spawn=jobs.append)
    tr = FakeTr("fp-a", SET, error=RuntimeError("client closed"))
    co.ensure(tr)
    tr.examples_fingerprint = "fp-b"
    jobs[0]()
    tr.examples_fingerprint = "fp-a"
    tr.error = None
    co.ensure(tr)
    assert len(jobs) == 2
    jobs[1]()
    assert tr.examples == SET


def test_number_lines_skips_blank_lines():
    assert number_lines("a\n\n b \n") == (["a", "b"], "1. a\n2. b")


def test_unnumber_lines_accepts_various_number_styles_and_plain_output():
    assert unnumber_lines("1) 甲\n２．乙\n3、丙", ["a", "b", "c"]) == "甲\n乙\n丙"
    assert unnumber_lines("甲\n乙", ["a", "b"]) == "甲\n乙"        # 沒編號但行數相同
    assert unnumber_lines("一整段", ["a", "b"]) == "一整段"        # 對不上就原樣回傳


def test_numbered_entries_ignores_unnumbered_lines():
    assert numbered_entries("以下是翻譯：\n1. 甲\n\n3. 丙") == {1: "甲", 3: "丙"}
