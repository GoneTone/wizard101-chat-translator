"""翻譯服務資料層測試。"""
from src import i18n
from src.config import is_configured
from src.services import (
    API_PROFILE_FIELDS,
    API_PROVIDERS,
    PROVIDERS,
    SLOT_INCOMING,
    SLOT_OUTGOING,
    SLOT_REGION,
    SLOTS,
    describe,
    find,
    needs_base_url,
    new_service,
    normalize,
    resolve,
    unique_name,
    validate_endpoint_fields,
    validate_service,
)
from tests.config_helpers import configured_cfg


def test_configured_cfg_passes_the_completeness_check():
    assert is_configured(configured_cfg())
    assert is_configured(configured_cfg("openai"))
    assert is_configured(configured_cfg("claude"))


def test_configured_cfg_returns_an_independent_copy():
    first = configured_cfg()
    first["api"]["custom"]["model"] = "mutated"
    assert configured_cfg()["api"]["custom"]["model"] == "m"


def test_every_provider_has_ui_metadata():
    assert set(PROVIDERS) == set(API_PROVIDERS)
    assert needs_base_url("custom")
    assert not needs_base_url("openai")


def test_short_names_are_brands_except_the_custom_endpoint():
    before = i18n.current_language()
    try:
        i18n.set_language("en-US")
        assert PROVIDERS["openai"].short_name == "ChatGPT"
        assert PROVIDERS["claude"].short_name == "Claude"
        # 自訂端點沒有品牌名，短名要走語言檔（此處只確認它被翻過，不釘字面）
        assert PROVIDERS["custom"].short_name != "provider.custom_short"
    finally:
        i18n.set_language(before)


def test_validate_service_reports_every_missing_required_field():
    errors = validate_service({"provider": "openai", "model": "", "api_key": ""})
    assert set(errors) == {"error.need_model", "error.need_api_key"}


def test_validate_endpoint_fields_ignores_the_model():
    assert validate_endpoint_fields(
        {"provider": "custom", "base_url": "http://x", "model": ""}) == []


def test_profile_fields_cover_what_the_translator_takes():
    assert set(API_PROFILE_FIELDS["claude"]) == {"model", "api_key", "effort"}
    assert "base_url" not in API_PROFILE_FIELDS["openai"]


def _section(*services, default=None, **slots):
    """一份只含服務三鍵的 cfg 片段（resolve 只讀這三個鍵）。"""
    listed = list(services)
    return {"services": listed,
            "default_service": default or (listed[0]["id"] if listed else None),
            "service_slots": {slot: slots.get(slot) for slot in SLOTS}}


def test_new_service_fills_the_provider_defaults_and_a_name():
    service = new_service("claude", [])
    assert service["provider"] == "claude"
    assert service["name"] == "Claude"
    assert service["model"] == "" and service["effort"] == "auto"
    assert len(service["id"]) == 8


def test_new_services_never_share_an_id():
    services = []
    for _ in range(50):
        services.append(new_service("openai", services))
    assert len({s["id"] for s in services}) == 50


def test_unique_name_numbers_collisions_from_two():
    services = [{"id": "a", "name": "ChatGPT"}]
    assert unique_name("ChatGPT", services) == "ChatGPT (2)"
    services.append({"id": "b", "name": "ChatGPT (2)"})
    assert unique_name("ChatGPT", services) == "ChatGPT (3)"
    assert unique_name("Claude", services) == "Claude"


def test_unique_name_does_not_collide_with_the_entry_being_edited():
    services = [{"id": "a", "name": "ChatGPT"}]
    assert unique_name("ChatGPT", services, ignore_id="a") == "ChatGPT"


def test_resolve_follows_the_default_when_a_slot_is_unset():
    a = new_service("openai", [])
    a.update(model="gpt-x", api_key="k1")
    cfg = _section(a)
    assert resolve(cfg, SLOT_INCOMING)["model"] == "gpt-x"


def test_resolve_prefers_the_slots_own_service():
    a = new_service("openai", [])
    a.update(model="gpt-x")
    b = new_service("claude", [a])
    b.update(model="claude-x")
    cfg = _section(a, b, region=b["id"])
    assert resolve(cfg, SLOT_REGION)["model"] == "claude-x"
    assert resolve(cfg, SLOT_INCOMING)["model"] == "gpt-x"


def test_resolve_drops_the_bookkeeping_fields():
    a = new_service("openai", [])
    resolved = resolve(_section(a), SLOT_INCOMING)
    assert "id" not in resolved and "name" not in resolved
    assert set(resolved) == {"provider", "model", "api_key", "thinking"}


def test_resolve_returns_a_copy():
    a = new_service("openai", [])
    a["model"] = "gpt-x"
    cfg = _section(a)
    resolve(cfg, SLOT_INCOMING)["model"] = "mutated"
    assert cfg["services"][0]["model"] == "gpt-x"


def test_find_returns_none_for_an_unknown_id():
    assert find(_section(new_service("openai", [])), "nope") is None
    assert find(_section(), None) is None


def test_describe_shows_the_brand_and_the_model():
    service = new_service("claude", [])
    service["model"] = "claude-opus-5"
    assert describe(service) == "Claude · claude-opus-5"


def _empty_section():
    return {"services": [], "default_service": None,
            "service_slots": {slot: None for slot in SLOTS}}


def test_migrates_the_flat_legacy_api_block():
    cfg = _empty_section()
    cfg["api"] = {"provider": "custom", "base_url": "http://127.0.0.1:8000",
                  "model": "gemma", "api_key": "sk-1"}
    assert normalize(cfg) is True
    assert "api" not in cfg
    assert len(cfg["services"]) == 1
    service = cfg["services"][0]
    assert service["provider"] == "custom"
    assert service["base_url"] == "http://127.0.0.1:8000"
    assert service["model"] == "gemma"
    assert cfg["default_service"] == service["id"]


def test_migrates_the_per_provider_api_block_keeping_every_filled_provider():
    cfg = _empty_section()
    cfg["api"] = {"provider": "claude",
                  "openai": {"model": "gpt-x", "api_key": "sk-1", "thinking": False},
                  "claude": {"model": "claude-x", "api_key": "sk-ant", "effort": "auto"},
                  "custom": {"base_url": "", "model": "", "api_key": "",
                             "thinking": False}}
    assert normalize(cfg) is True
    assert [s["provider"] for s in cfg["services"]] == ["openai", "claude"]
    assert [s["name"] for s in cfg["services"]] == ["ChatGPT", "Claude"]
    # 原本選中的那家成為預設，沒填過的自訂端點不留空殼
    assert find(cfg, cfg["default_service"])["provider"] == "claude"


def test_migration_of_an_untouched_config_leaves_an_empty_list():
    cfg = _empty_section()
    cfg["api"] = {"provider": "openai",
                  "openai": {"model": "", "api_key": "", "thinking": False},
                  "claude": {"model": "", "api_key": "", "effort": "auto"},
                  "custom": {"base_url": "", "model": "", "api_key": "",
                             "thinking": False}}
    normalize(cfg)
    assert cfg["services"] == []
    assert cfg["default_service"] is None


def test_drops_a_service_with_an_unknown_provider():
    cfg = _empty_section()
    cfg["services"] = [{"id": "aaaaaaaa", "name": "X", "provider": "gemini",
                        "model": "g"}]
    assert normalize(cfg) is True
    assert cfg["services"] == []


def test_fills_missing_fields_and_drops_stale_ones():
    cfg = _empty_section()
    cfg["services"] = [{"id": "aaaaaaaa", "name": "C", "provider": "claude",
                        "model": "claude-x", "thinking": True}]
    assert normalize(cfg) is True
    assert cfg["services"][0] == {"id": "aaaaaaaa", "name": "C", "provider": "claude",
                                  "model": "claude-x", "api_key": "", "effort": "auto"}


def test_regenerates_duplicate_ids():
    cfg = _empty_section()
    cfg["services"] = [
        {"id": "dup", "name": "A", "provider": "openai", "model": "m", "api_key": "k",
         "thinking": False},
        {"id": "dup", "name": "B", "provider": "openai", "model": "m", "api_key": "k",
         "thinking": False}]
    assert normalize(cfg) is True
    assert cfg["services"][0]["id"] != cfg["services"][1]["id"]


def test_default_service_falls_back_to_the_first_entry():
    cfg = _empty_section()
    cfg["services"] = [new_service("openai", [])]
    cfg["default_service"] = "nope"
    assert normalize(cfg) is True
    assert cfg["default_service"] == cfg["services"][0]["id"]


def test_a_slot_pointing_at_nothing_falls_back_to_the_default():
    cfg = _empty_section()
    cfg["services"] = [new_service("openai", [])]
    cfg["default_service"] = cfg["services"][0]["id"]
    cfg["service_slots"] = {SLOT_INCOMING: "nope", SLOT_OUTGOING: None,
                            SLOT_REGION: None, "bogus": "x"}
    assert normalize(cfg) is True
    assert cfg["service_slots"] == {slot: None for slot in SLOTS}


def test_a_tidy_config_reports_no_change():
    cfg = _empty_section()
    cfg["services"] = [new_service("openai", [])]
    cfg["services"][0].update(model="m", api_key="k")
    cfg["default_service"] = cfg["services"][0]["id"]
    normalize(cfg)
    assert normalize(cfg) is False
