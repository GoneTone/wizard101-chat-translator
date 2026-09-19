"""翻譯服務資料層測試。"""
import copy

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
    UNSUPPORTED_SERVICES,
    describe,
    find,
    is_auto_name,
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
    first["services"][0]["model"] = "mutated"
    assert configured_cfg()["services"][0]["model"] == "m"


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


def test_is_auto_name_accepts_the_bare_base_and_numbered_suffixes():
    assert is_auto_name("ChatGPT", "ChatGPT")
    assert is_auto_name("ChatGPT (2)", "ChatGPT")
    assert is_auto_name("ChatGPT (10)", "ChatGPT")


def test_is_auto_name_rejects_shapes_unique_name_never_produces():
    assert not is_auto_name("ChatGPT 我的", "ChatGPT")
    assert not is_auto_name("Claude", "ChatGPT")
    assert not is_auto_name("ChatGPT ()", "ChatGPT")
    assert not is_auto_name("ChatGPT (x)", "ChatGPT")
    assert not is_auto_name("ChatGPT (2) extra", "ChatGPT")
    # unique_name 的序號從 2 起跳，(0)／(1)／前導零都不在它的值域內，
    # 這些形狀比較像使用者自己打的名字，判斷式寧可漏判也不能誤蓋
    assert not is_auto_name("ChatGPT (0)", "ChatGPT")
    assert not is_auto_name("ChatGPT (1)", "ChatGPT")
    assert not is_auto_name("ChatGPT (01)", "ChatGPT")


def test_is_auto_name_matches_what_unique_name_actually_generates():
    services = [{"id": "a", "name": "ChatGPT"}]
    assert is_auto_name(unique_name("ChatGPT", services), "ChatGPT")


def test_is_auto_name_accepts_exactly_the_numbers_unique_name_can_produce():
    """反向釘住值域：不只「產生器吐出來的都被認得」，還要「認得的不多不少」——
    這樣改動任一邊（起跳值、後綴格式）都會讓這則測試變紅。"""
    base = "ChatGPT"
    services = [{"id": "seed", "name": base}]
    generated_numbers = set()
    for n in range(2, 6):
        name = unique_name(base, services)
        assert name == f"{base} ({n})"
        generated_numbers.add(n)
        services.append({"id": name, "name": name})
    accepted = {n for n in range(0, 6) if is_auto_name(f"{base} ({n})", base)}
    assert accepted == generated_numbers


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
    assert service["api_key"] == "sk-1"
    assert cfg["default_service"] == service["id"]


def test_migrates_a_legacy_block_without_a_provider_as_a_custom_endpoint():
    # 最舊的設定連 provider 欄位都沒有（自架端點時代）：一律當自訂端點
    cfg = _empty_section()
    cfg["api"] = {"base_url": "http://127.0.0.1:8000", "model": "m1"}
    assert normalize(cfg) is True
    assert [s["provider"] for s in cfg["services"]] == ["custom"]
    assert cfg["services"][0]["base_url"] == "http://127.0.0.1:8000"
    assert cfg["default_service"] == cfg["services"][0]["id"]


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
    assert [s["api_key"] for s in cfg["services"]] == ["sk-1", "sk-ant"]
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


def test_a_leftover_api_block_does_not_wipe_an_existing_service_list():
    # 舊版 exe 可能寫回一份同時帶 services 與空 api 區塊的 config；不能讓 api 蓋掉金鑰。
    cfg = _empty_section()
    cfg["services"] = [new_service("claude", [])]
    cfg["services"][0].update(model="claude-x", api_key="sk-ant-REAL-USER-KEY")
    cfg["default_service"] = cfg["services"][0]["id"]
    cfg["api"] = {"provider": "openai",
                  "openai": {"model": "", "api_key": "", "thinking": False},
                  "claude": {"model": "", "api_key": "", "effort": "auto"},
                  "custom": {"base_url": "", "model": "", "api_key": "",
                             "thinking": False}}
    assert normalize(cfg) is True
    assert "api" not in cfg
    assert len(cfg["services"]) == 1
    assert cfg["services"][0]["api_key"] == "sk-ant-REAL-USER-KEY"
    assert cfg["default_service"] == cfg["services"][0]["id"]


def test_quarantines_a_service_with_an_unknown_provider():
    """日後版本新增的服務商被這一版讀到時不能連金鑰一起消失，整筆原樣搬進隔離區。"""
    cfg = _empty_section()
    entry = {"id": "aaaaaaaa", "name": "X", "provider": "gemini", "model": "g",
             "api_key": "sk-REAL-USER-KEY", "quirk": 1}
    cfg["services"] = [entry]
    assert normalize(cfg) is True
    assert cfg["services"] == []
    assert cfg[UNSUPPORTED_SERVICES] == [copy.deepcopy(entry)]
    assert cfg["default_service"] is None


def test_a_quarantined_service_is_never_pointed_at():
    """認得的那筆不受影響，預設與插槽都不會留在被隔離的 id 上。"""
    cfg = _empty_section()
    known = new_service("openai", [])
    known.update(model="m", api_key="k")
    unknown = {"id": "bbbbbbbb", "name": "Future", "provider": "gemini",
               "api_key": "sk-2"}
    cfg["services"] = [known, unknown]
    cfg["default_service"] = "bbbbbbbb"
    cfg["service_slots"] = {SLOT_INCOMING: "bbbbbbbb", SLOT_OUTGOING: None,
                            SLOT_REGION: None}
    assert normalize(cfg) is True
    assert cfg["services"] == [known]
    assert cfg[UNSUPPORTED_SERVICES] == [copy.deepcopy(unknown)]
    assert cfg["default_service"] == known["id"]
    assert cfg["service_slots"] == {slot: None for slot in SLOTS}


def test_a_hand_edited_quarantine_key_keeps_what_was_already_there():
    """隔離區被手改成 list 以外的東西時，原值要變成隔離區裡的一筆 —— 最可能的形狀是
    貼歪的裸金鑰字串，蓋掉它等於做了這次改動要杜絕的事。"""
    cfg = _empty_section()
    cfg[UNSUPPORTED_SERVICES] = "sk-PASTED-IN-THE-WRONG-PLACE"
    cfg["services"] = ["sk-ANOTHER-STRAY-KEY"]
    assert normalize(cfg) is True
    assert cfg["services"] == []
    assert cfg[UNSUPPORTED_SERVICES] == ["sk-PASTED-IN-THE-WRONG-PLACE",
                                         "sk-ANOTHER-STRAY-KEY"]


def test_a_quarantined_service_returns_once_its_provider_is_known():
    """認得那家服務商的版本要自動把它搬回清單，並照常補值與去重。"""
    cfg = _empty_section()
    cfg["services"] = [new_service("openai", [])]
    cfg[UNSUPPORTED_SERVICES] = [{"provider": "openai", "name": "ChatGPT",
                                  "api_key": "sk-3"}]
    assert normalize(cfg) is True
    assert cfg[UNSUPPORTED_SERVICES] == []
    restored = cfg["services"][1]
    assert restored["api_key"] == "sk-3"
    assert restored["model"] == ""
    assert restored["id"] and restored["id"] != cfg["services"][0]["id"]
    assert restored["name"] == "ChatGPT (2)"


def test_a_field_whose_default_is_none_keeps_the_real_value():
    """預設值若是 None，型別比對會把每個真值（含金鑰）判成型別不符整欄清掉。"""
    from src.services import _profile_field

    assert _profile_field({"api_key": "sk-1"}, "openai", "api_key", None) == "sk-1"


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
    cfg["default_service"] = "nope"
    assert normalize(cfg) is True
    assert normalize(cfg) is False


def test_a_null_field_in_a_migrated_api_block_stays_usable():
    """舊設定的 null 欄位遷移後要能通過 validate_service 的 .strip()：設定壞掉最多
    退回精靈，不能在任何視窗開出來之前就把程式帶掉。"""
    cfg = _empty_section()
    cfg["api"] = {"provider": "openai", "openai": {"model": "gpt-x", "api_key": None}}
    assert normalize(cfg) is True
    assert cfg["services"][0]["api_key"] == ""
    assert is_configured(cfg) is False


def test_a_null_field_in_the_flat_legacy_block_keeps_the_key():
    cfg = _empty_section()
    cfg["api"] = {"provider": "custom", "base_url": "http://x", "model": None,
                  "api_key": "sk-REAL-USER-KEY"}
    assert normalize(cfg) is True
    assert cfg["services"][0]["model"] == ""
    assert cfg["services"][0]["api_key"] == "sk-REAL-USER-KEY"
    assert is_configured(cfg) is False


def test_a_field_of_the_wrong_type_falls_back_to_the_provider_default():
    cfg = _empty_section()
    cfg["services"] = [new_service("openai", [])]
    cfg["services"][0].update(model=5, api_key="k", thinking="yes")
    cfg["default_service"] = cfg["services"][0]["id"]
    assert normalize(cfg) is True
    assert cfg["services"][0]["model"] == ""
    assert cfg["services"][0]["thinking"] is False
    assert cfg["services"][0]["api_key"] == "k"
    assert is_configured(cfg) is False


def test_a_provider_of_the_wrong_type_drops_the_entry():
    cfg = _empty_section()
    cfg["services"] = [{"id": "aaaaaaaa", "name": "X", "provider": ["openai"],
                        "model": "m"}]
    assert normalize(cfg) is True
    assert cfg["services"] == []


def test_an_id_or_name_of_the_wrong_type_is_regenerated():
    cfg = _empty_section()
    cfg["services"] = [{"id": 7, "name": 42, "provider": "openai", "model": "m",
                        "api_key": "k", "thinking": False}]
    assert normalize(cfg) is True
    assert cfg["services"][0]["id"] != 7 and len(cfg["services"][0]["id"]) == 8
    assert cfg["services"][0]["name"] == PROVIDERS["openai"].short_name


def test_a_default_service_of_the_wrong_type_falls_back_to_the_first_entry():
    cfg = _empty_section()
    cfg["services"] = [new_service("openai", [])]
    cfg["default_service"] = []
    assert normalize(cfg) is True
    assert cfg["default_service"] == cfg["services"][0]["id"]


def test_a_slot_of_the_wrong_type_falls_back_to_the_default():
    cfg = _empty_section()
    cfg["services"] = [new_service("openai", [])]
    cfg["default_service"] = cfg["services"][0]["id"]
    cfg["service_slots"] = {SLOT_INCOMING: ["x"], SLOT_OUTGOING: 3,
                            SLOT_REGION: None}
    assert normalize(cfg) is True
    assert cfg["service_slots"] == {slot: None for slot in SLOTS}


def test_two_services_sharing_a_name_load_with_distinct_names():
    """分派下拉只顯示名稱、再靠名稱換回 id：同名會讓使用者選到另一筆服務。"""
    cfg = _empty_section()
    first = new_service("openai", [])
    second = dict(new_service("openai", [first]), name=first["name"])
    cfg["services"] = [first, second]
    cfg["default_service"] = first["id"]
    assert normalize(cfg) is True
    names = [s["name"] for s in cfg["services"]]
    assert names == [first["name"], f"{first['name']} (2)"]


def test_every_provider_label_and_description_resolve():
    """卡片的標題與說明都要有文案：新增一家忘了補，UI 會直接顯示文案 key 本身。"""
    before = i18n.current_language()
    try:
        i18n.set_language(i18n.SOURCE_LANGUAGE)
        for key, provider in PROVIDERS.items():
            assert i18n.t(provider.label_key) != provider.label_key, key
            assert i18n.t(provider.desc_key) != provider.desc_key, key
    finally:
        i18n.set_language(before)
