"""精靈步驟門檻的純邏輯測試（UI 互動靠實機驗證）。"""
from src.ui.wizard import STEP_API, STEP_PREFS, can_advance


def test_step_api_requires_valid_form():
    assert not can_advance(STEP_API, api_test_passed=True, api_errors=["error.need_api_key"])


def test_step_api_requires_passed_or_skipped_test():
    assert not can_advance(STEP_API, api_test_passed=False, api_errors=[])
    assert can_advance(STEP_API, api_test_passed=True, api_errors=[])


def test_step_prefs_always_advances():
    assert can_advance(STEP_PREFS, api_test_passed=False, api_errors=[])


def test_can_advance_allows_language_step():
    from src.ui.wizard import STEP_LANG, can_advance

    # 語言頁沒有必填欄位，永遠可以往下一步
    assert can_advance(STEP_LANG, api_test_passed=False, api_errors=["error.need_model"])


def test_language_change_marks_restart_and_applies_language(root):
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        wizard = SetupWizard(root, cfg)
        wizard._on_language_change("en-US")
        assert cfg["ui_language"] == "en-US"
        assert i18n.current_language() == "en-US"
        assert wizard.restart is True
    finally:
        i18n.set_language(before)


def test_language_change_updates_untouched_target_language(root):
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        cfg["target_language"] = "繁體中文（台灣）"   # 使用者還沒動過
        wizard = SetupWizard(root, cfg)
        wizard._on_language_change("en-US")
        assert cfg["target_language"] == "English"
    finally:
        i18n.set_language(before)


def test_language_change_keeps_customised_target_language(root):
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        cfg["target_language"] = "日本語"   # 使用者自己選過了
        wizard = SetupWizard(root, cfg)
        wizard._on_language_change("en-US")
        assert cfg["target_language"] == "日本語"
    finally:
        i18n.set_language(before)


def test_language_change_keeps_the_half_filled_service(root):
    """回歸：在 API 步驟貼好金鑰後退回第一步換介面語言，精靈會整個重建 ——
    重建出來的表單必須還帶著剛才填的值，不能逼使用者重貼一次金鑰。"""
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        wizard = SetupWizard(root, cfg)
        # 直接呼叫 STEP_API 的 handler 是刻意的：這幾則測的是換語言，不是步驟導航
        wizard._pick_provider("claude")
        wizard._service_form._model.set("claude-x")
        wizard._service_form._api_key.set("sk-ant-secret")
        wizard._on_language_change("en-US")
        rebuilt = SetupWizard(root, cfg)
        assert rebuilt._service_form.values()["provider"] == "claude"
        assert rebuilt._service_form.values()["model"] == "claude-x"
        assert rebuilt._service_form.values()["api_key"] == "sk-ant-secret"
        rebuilt._win.destroy()
    finally:
        i18n.set_language(before)


def test_language_change_retitles_an_untouched_service(root):
    # 自動取的名字就是服務商短名：使用者沒改過就跟著介面語言走（與 target_language 同一招）
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        wizard = SetupWizard(root, cfg)
        # 直接呼叫 STEP_API 的 handler 是刻意的：這幾則測的是換語言，不是步驟導航
        wizard._pick_provider("custom")   # 只有自訂端點的短名要翻譯
        wizard._on_language_change("en-US")
        assert cfg["services"][0]["name"] == "Custom endpoint"
    finally:
        i18n.set_language(before)


def test_language_change_retitles_a_suffixed_untouched_service(root):
    """回歸：draft 的自動名稱撞到既有服務時會帶序號（如「自訂端點 (2)」），
    這仍算使用者沒改過名字，換介面語言時要照樣跟著換（潛在陷阱，非實際回報個案）。"""
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.services import new_service
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        cfg["services"] = [new_service("custom", [])]   # 名稱＝zh-TW 的自訂端點短名
        wizard = SetupWizard(root, cfg)
        # 直接呼叫 STEP_API 的 handler 是刻意的：這幾則測的是換語言，不是步驟導航
        wizard._pick_provider("custom")   # draft 撞名，取到「自訂端點 (2)」
        assert wizard._service_form.values()["name"] == "自訂端點 (2)"
        wizard._on_language_change("en-US")
        assert cfg["services"][1]["name"] == "Custom endpoint"
    finally:
        i18n.set_language(before)


def test_language_change_only_retitles_the_service_being_edited(root):
    """回歸：清單裡不只精靈編輯的那一筆，改名不能靠位置認人 ——
    索引 0 那個與精靈無關的服務必須原封不動。"""
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.services import new_service
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        # 兩筆都是自訂端點（唯一短名隨語言變的一家）且都停在自動取的名字，
        # 「誰該被改名」的差別就只剩下位置
        bystander = new_service("custom", [])
        edited = new_service("custom", [bystander])
        edited["name"] = bystander["name"]
        cfg["services"] = [bystander, edited]
        cfg["default_service"] = edited["id"]   # 精靈編輯的不是索引 0 那筆
        wizard = SetupWizard(root, cfg)
        wizard._on_language_change("en-US")
        assert cfg["services"][0]["name"] == "自訂端點"
        assert cfg["services"][1]["name"] == "Custom endpoint"
    finally:
        i18n.set_language(before)


def test_language_change_keeps_a_service_name_the_user_typed(root):
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        wizard = SetupWizard(root, cfg)
        # 直接呼叫 STEP_API 的 handler 是刻意的：這幾則測的是換語言，不是步驟導航
        wizard._pick_provider("custom")
        wizard._service_form.set_name("戰鬥用")
        wizard._on_language_change("en-US")
        assert cfg["services"][0]["name"] == "戰鬥用"
    finally:
        i18n.set_language(before)


def test_language_change_from_en_bootstrap_follows_to_zh_cn(root):
    """回歸測試：英文系統首次啟動時 bootstrap_language 把 target_language 設成介面語言的
    自稱（"English"），精靈再切到 zh-CN 時 `target_language == old_default` 才比對得到。
    修復前 target_language 永遠停在「繁體中文（台灣）」，這條分支形同死碼。"""
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.main import bootstrap_language
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        assert cfg["ui_language"] is None  # 首次執行
        language = bootstrap_language(cfg, config_existed=False, detect=lambda: "en-US")
        i18n.set_language(language)
        assert cfg["target_language"] == i18n.language_name("en-US")

        wizard = SetupWizard(root, cfg)
        wizard._on_language_change("zh-CN")
        assert cfg["target_language"] == i18n.language_name("zh-CN")
        assert cfg["target_language"] == "简体中文（中国）"
    finally:
        i18n.set_language(before)


def test_bootstrap_language_first_run_no_config_file():
    """真正首次執行（config 檔案原本不存在）：介面語言依偵測，target_language 也跟著換。"""
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.main import bootstrap_language

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    assert cfg["ui_language"] is None

    language = bootstrap_language(cfg, config_existed=False, detect=lambda: "en-US")

    assert language == "en-US"
    assert cfg["target_language"] == i18n.language_name("en-US")


def test_bootstrap_language_upgrading_user_keeps_target_language():
    """回歸測試：升級使用者的 config 已存在，只是舊版沒有 ui_language（被回填成 None），
    不代表沒走過精靈：介面語言仍依偵測，但使用者選好的 target_language 不可被覆蓋。"""
    from src.main import bootstrap_language

    cfg = {"ui_language": None, "target_language": "日本語"}

    language = bootstrap_language(cfg, config_existed=True, detect=lambda: "en-US")

    assert language == "en-US"
    assert cfg["target_language"] == "日本語"


def test_bootstrap_language_returning_user_uses_saved_language():
    """回頭使用者：config 已有真實的 ui_language，直接採用、不呼叫 detect，target_language 不動。"""
    from src.main import bootstrap_language

    cfg = {"ui_language": "zh-CN", "target_language": "日本語"}
    calls = []

    language = bootstrap_language(cfg, config_existed=True, detect=lambda: calls.append(1) or "en-US")

    assert language == "zh-CN"
    assert calls == []
    assert cfg["target_language"] == "日本語"


def test_language_step_shows_the_translators(root, monkeypatch):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.i18n import t
    from src.ui import form as form_module
    from src.ui.wizard import STEP_PREFS, SetupWizard

    monkeypatch.setattr(form_module, "translators",
                        lambda code: "[A](https://a.example)")
    wizard = SetupWizard(root, copy.deepcopy(DEFAULT_CONFIG))
    label, names = wizard._translators_row.pack_slaves()
    assert label.cget("text") == t("credit.translators")
    assert [w.cget("text") for w in names.pack_slaves()] == ["A"]
    order = [str(w) for w in wizard._body.pack_slaves()]
    assert (order.index(str(wizard._help_translate_link))
            == order.index(str(wizard._translators_row)) + 1)

    # 掛名與邀請連結只屬於語言頁：往後的步驟不該還留著
    wizard._step = STEP_PREFS
    wizard._show_step()
    assert wizard._translators_row is None
    assert wizard._help_translate_link is None
    wizard._win.destroy()


def test_language_step_without_translators_shows_no_row(root, monkeypatch):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.ui import form as form_module
    from src.ui.wizard import SetupWizard

    monkeypatch.setattr(form_module, "translators", lambda code: "")
    wizard = SetupWizard(root, copy.deepcopy(DEFAULT_CONFIG))
    assert wizard._translators_row is None
    wizard._win.destroy()


def test_language_step_links_to_crowdin_even_without_translators(root, monkeypatch):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.i18n import t
    from src.ui import form as form_module
    from src.ui.wizard import SetupWizard
    from src.updater import CROWDIN_URL

    monkeypatch.setattr(form_module, "translators", lambda code: "")
    opened = []
    monkeypatch.setattr(form_module.webbrowser, "open", opened.append)
    wizard = SetupWizard(root, copy.deepcopy(DEFAULT_CONFIG))
    link = wizard._help_translate_link
    assert link.cget("text") == t("credit.help_translate")
    order = [str(w) for w in wizard._body.pack_slaves()]
    assert order.index(str(link)) == order.index(str(wizard._ui_language)) + 1
    link.event_generate("<Button-1>")
    assert opened == [CROWDIN_URL]
    wizard._win.destroy()


def _three_services():
    """三組填好的服務（openai／claude／custom），回傳那份清單。"""
    from src.services import new_service

    services: list[dict] = []
    for provider in ("openai", "claude", "custom"):
        service = new_service(provider, services)
        service.update(model="m", api_key=f"sk-{provider}")
        services.append(service)
    services[-1]["base_url"] = "http://x"
    return services


def test_finishing_the_wizard_keeps_the_other_services(root):
    """回歸：精靈也是設定壞掉時的救援路徑 —— 按完成不能把其他服務與它們的金鑰刪掉。"""
    import copy

    from src.config import DEFAULT_CONFIG
    from src.services import SLOT_REGION
    from src.ui.wizard import SetupWizard

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["services"] = _three_services()
    ids = [s["id"] for s in cfg["services"]]
    cfg["default_service"] = ids[1]              # 預設是第二組
    cfg["service_slots"][SLOT_REGION] = ids[2]
    wizard = SetupWizard(root, cfg)
    assert wizard._service_form.values()["id"] == ids[1]   # 草稿取自預設那一筆，不是第一筆
    wizard._service_form._model.set("claude-x")
    wizard._finish()
    assert [s["id"] for s in cfg["services"]] == ids       # 三組都還在、順序不變
    assert cfg["services"][0]["api_key"] == "sk-openai"
    assert cfg["services"][2]["api_key"] == "sk-custom"
    assert cfg["services"][1]["model"] == "claude-x"       # 編輯過的那一筆就地更新
    assert cfg["default_service"] == ids[1]
    assert cfg["service_slots"][SLOT_REGION] == ids[2]     # 既有的用途分派不被清掉


def test_finish_stores_the_region_hotkey(root):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.ui.wizard import SetupWizard

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    wizard = SetupWizard(root, cfg)
    wizard._step = STEP_PREFS
    wizard._show_step()
    assert wizard._region_hotkey.value() == "ctrl+shift+space"
    wizard._region_hotkey.set_value("ctrl+alt+r")
    wizard._finish()
    assert wizard.completed is True
    assert cfg["region_hotkey"] == "ctrl+alt+r"


def test_finish_rejects_identical_hotkeys(root, monkeypatch):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.i18n import t
    from src.ui import wizard as wizard_module
    from src.ui.wizard import SetupWizard

    warnings = []
    monkeypatch.setattr(wizard_module.messagebox, "showwarning",
                        lambda title, message, parent=None: warnings.append(message))
    cfg = copy.deepcopy(DEFAULT_CONFIG)
    wizard = SetupWizard(root, cfg)
    wizard._step = STEP_PREFS
    wizard._show_step()
    wizard._region_hotkey.set_value(cfg["hotkey"])
    wizard._finish()
    assert wizard.completed is False
    assert warnings == [t("error.hotkeys_same")]
    assert cfg["region_hotkey"] == "ctrl+shift+space"
    wizard._win.destroy()


def test_language_change_does_not_let_the_rename_collide(root):
    """自動改名同樣要去重：清單裡已經有一筆叫新語言短名的服務時，改名不能撞上它 ——
    分派下拉只顯示名稱，同名的兩筆會讓使用者選到另一家。"""
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.services import new_service
    from src.ui.wizard import SetupWizard

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        edited = new_service("custom", [])          # 名稱＝zh-TW 的自訂端點短名
        i18n.set_language("en-US")
        other = new_service("custom", [])           # 名稱＝en-US 的自訂端點短名
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        cfg["services"] = [edited, other]
        cfg["default_service"] = edited["id"]
        wizard = SetupWizard(root, cfg)
        wizard._on_language_change("en-US")
        names = [s["name"] for s in cfg["services"]]
        assert names == [f"{other['name']} (2)", other["name"]]
    finally:
        i18n.set_language(before)


def test_switching_provider_in_the_wizard_dedupes_against_cfg_services(root):
    """精靈把 cfg["services"] 傳給表單：編輯中的服務切到 Claude，
    要跟清單裡另一筆已經叫 Claude 的服務去重。"""
    import copy

    from src.config import DEFAULT_CONFIG
    from src.services import new_service

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    existing_claude = new_service("claude", [])
    openai_service = new_service("openai", [existing_claude])
    cfg["services"] = [existing_claude, openai_service]
    cfg["default_service"] = openai_service["id"]
    wizard = _wizard_on_api_step(root, cfg)
    assert wizard._service_form.values()["id"] == openai_service["id"]
    wizard._service_form.set_provider("claude")
    assert wizard._service_form.values()["name"] == "Claude (2)"
    wizard._win.destroy()


def _wizard_on_api_step(root, cfg):
    """停在 API 步驟的精靈（第二步的兩種樣態都從這裡看）。"""
    from src.ui.wizard import STEP_API, SetupWizard

    wizard = SetupWizard(root, cfg)
    wizard._step = STEP_API
    wizard._show_step()
    return wizard


def _cards_on_body(wizard):
    from src.ui.provider_picker import ProviderCards

    return [w for w in wizard._body.pack_slaves() if isinstance(w, ProviderCards)]


def test_step_api_asks_for_a_provider_before_the_form(root):
    """沒有服務可編輯時第二步先選服務商：表單還不存在，〔下一步〕也就無從驗起。"""
    import copy

    from src.config import DEFAULT_CONFIG

    wizard = _wizard_on_api_step(root, copy.deepcopy(DEFAULT_CONFIG))
    assert wizard._service_form is None
    assert len(_cards_on_body(wizard)) == 1
    assert str(wizard._next_btn.cget("state")) == "disabled"
    wizard._win.destroy()


def test_picking_a_provider_replaces_the_cards_with_the_form(root):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.services import PROVIDERS

    wizard = _wizard_on_api_step(root, copy.deepcopy(DEFAULT_CONFIG))
    _cards_on_body(wizard)[0].pack_slaves()[1].event_generate("<Button-1>")
    assert wizard._service_form is not None
    assert wizard._service_form.values()["provider"] == list(PROVIDERS)[1]
    assert _cards_on_body(wizard) == []
    assert wizard._service_form in wizard._body.pack_slaves()
    wizard._win.destroy()


def test_an_existing_service_skips_the_cards(root):
    """救援路徑：設定壞掉的老使用者被送回精靈時，服務商早就選過了，直接編輯那一筆。"""
    import copy

    from src.config import DEFAULT_CONFIG

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["services"] = _three_services()
    cfg["default_service"] = cfg["services"][1]["id"]
    wizard = _wizard_on_api_step(root, cfg)
    assert _cards_on_body(wizard) == []
    assert wizard._service_form.values()["id"] == cfg["services"][1]["id"]
    wizard._win.destroy()


def test_language_change_while_picking_rebuilds_into_the_cards(root):
    """還在選服務商時換語言：沒有服務可寫進 cfg，重建出來的精靈仍停在卡片。
    同一步驟負責的其他欄位照寫。"""
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        wizard = _wizard_on_api_step(root, cfg)
        wizard._region_hotkey.set_value("ctrl+alt+r")
        wizard._on_language_change("en-US")
        assert cfg["services"] == []
        assert cfg["default_service"] is None
        assert cfg["region_hotkey"] == "ctrl+alt+r"
        rebuilt = _wizard_on_api_step(root, cfg)
        assert rebuilt._service_form is None
        assert len(_cards_on_body(rebuilt)) == 1
        rebuilt._win.destroy()
    finally:
        i18n.set_language(before)


def test_language_change_after_picking_rebuilds_into_the_form(root):
    import copy

    from src import i18n
    from src.config import DEFAULT_CONFIG

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        wizard = _wizard_on_api_step(root, cfg)
        wizard._pick_provider("claude")
        wizard._on_language_change("en-US")
        rebuilt = _wizard_on_api_step(root, cfg)
        assert _cards_on_body(rebuilt) == []
        assert rebuilt._service_form.values()["provider"] == "claude"
        rebuilt._win.destroy()
    finally:
        i18n.set_language(before)
