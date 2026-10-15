"""main 的接線與啟動流程：各用途的翻譯器與快取、套用設定時的重建、啟動順序。"""
import copy
import subprocess
import sys
from pathlib import Path

import pytest

from src import main
from src.config import DEFAULT_CONFIG
from src.translation.examples import ExampleSet
from tests.config_helpers import configured_cfg


def test_splash_closes_before_focusing_an_existing_instance(monkeypatch):
    """啟動畫面要在既有視窗被喚起之前關掉，否則會蓋在它上面；這條路徑在建立視窗前
    就 return，其餘兩個出口交給實機驗證。
    """
    events = []
    monkeypatch.setattr(main, "redirect_output", lambda: None)
    monkeypatch.setattr(main, "load_config", lambda path: copy.deepcopy(DEFAULT_CONFIG))
    monkeypatch.setattr(main, "bootstrap_language", lambda cfg, existed: "en-US")
    monkeypatch.setattr(main, "set_language", lambda code: None)
    monkeypatch.setattr(main, "app_name", lambda: "Wizard101 Chat Translator")
    monkeypatch.setattr(main, "acquire_single_instance", lambda: None)
    monkeypatch.setattr(main.splash, "close", lambda: events.append("close"))
    monkeypatch.setattr(main, "focus_running_instance",
                        lambda title: events.append("focus") or False)

    main.main()

    assert events == ["close", "focus"]


ROOT = Path(__file__).resolve().parents[1]


def test_startup_path_does_not_import_anthropic():
    """啟動路徑不得載入 anthropic（約 0.7 秒）；必須另起乾淨直譯器問，因為同一個
    process 內別的測試早就載入過。
    """
    code = "import src.main, sys; print('anthropic' in sys.modules)"
    out = subprocess.run([sys.executable, "-c", code], capture_output=True,
                         text=True, cwd=ROOT, check=False)
    # check=False 才讓下面兩個斷言都跑得到：check=True 遇到非 0 結束碼會直接拋
    # CalledProcessError，訊息裡沒有 stdout／stderr，等於白寫了下面的診斷字串
    assert out.returncode == 0, f"stderr={out.stderr!r}"
    assert out.stdout.strip() == "False", f"stdout={out.stdout!r} stderr={out.stderr!r}"


def test_run_py_updates_splash_before_importing_main():
    """`run.py` 存在的理由是這個順序：`import src.main` 要花約 0.3 秒，必須排在
    `splash.update()` 之後，否則畫面會停在 bootloader 寫死的 `Initializing...`。
    """
    source = (ROOT / "run.py").read_text(encoding="utf-8")
    assert (
        "splash.update(splash.PHASE_LOADING)\n"
        "    from src.main import main\n"
    ) in source


def test_main_py_updates_splash_with_the_starting_phase_constant():
    """build 時 `tools/splash_progress.py` 把 `PHASE_STARTING` 的值烤進 Tcl 判斷
    進度條目標；改回字面值會讓耦合悄悄失效，且不會有其他測試變紅。"""
    source = (ROOT / "src" / "main.py").read_text(encoding="utf-8")
    assert "splash.update(splash.PHASE_STARTING)" in source


class _FakePool:
    """翻譯池替身：真的那個會開 worker 執行緒，這裡只記下接到哪個翻譯器與翻譯函式。"""

    def __init__(self, translator, on_result, workers, failed_notice_fn,
                 translate_fn=None, gate=None):
        self.translator = translator
        self.workers = workers
        self.translate_fn = translate_fn

    def resize(self, workers: int) -> None:
        self.workers = workers


class _FakeCache:
    """譯文快取替身：真的那個會讀寫磁碟，這裡只記下目前的指紋。"""

    def __init__(self, fingerprint: str, *args, **kwargs):
        self.fingerprint = fingerprint

    def load(self) -> None:
        pass

    def rebind(self, fingerprint: str) -> None:
        self.fingerprint = fingerprint


def _three_slot_cfg():
    """三個用途各指向一筆不同服務的 cfg（模型不同，才看得出誰接到誰）。"""
    from src.services import SLOT_INCOMING, SLOT_OUTGOING, SLOT_REGION, new_service

    cfg = configured_cfg("openai", model="incoming-model")
    incoming = cfg["services"][0]
    outgoing = new_service("custom", cfg["services"])
    outgoing.update(base_url="http://out", model="outgoing-model")
    region = new_service("custom", [*cfg["services"], outgoing])
    region.update(base_url="http://region", model="region-model")
    cfg["services"] += [outgoing, region]
    cfg["default_service"] = incoming["id"]
    cfg["service_slots"] = {SLOT_INCOMING: incoming["id"],
                            SLOT_OUTGOING: outgoing["id"],
                            SLOT_REGION: region["id"]}
    return cfg


class _FakeCoordinator:
    """範例協調器替身：只記下 ensure 了誰，並留住 on_applied 讓測試直接觸發。"""

    def __init__(self, store, post, spawn=None, on_applied=None):
        self.ensured, self.on_applied = [], on_applied

    def ensure(self, translator):
        self.ensured.append(translator)


class _FakeStore:
    """範例儲存替身：磁碟上什麼都沒有。"""

    def get(self, fingerprint):
        return None


def _build(cfg):
    return main.build_translation(cfg, lambda *a: None, lambda job: job())


def _stub_translation(monkeypatch):
    """換掉會開執行緒與碰磁碟的元件，其餘（三個翻譯器、指紋）維持真貨。"""
    monkeypatch.setattr(main, "TranslationPool", _FakePool)
    monkeypatch.setattr(main, "TranslationCache", _FakeCache)
    monkeypatch.setattr(main, "ExampleCoordinator", _FakeCoordinator)
    monkeypatch.setattr(main, "ExampleStore", _FakeStore)


def test_each_slot_gets_its_own_translator_and_the_cache_follows_incoming(monkeypatch):
    """三個用途各接到自己那格的服務，兩條翻譯池都吃收訊那格，快取指紋也綁收訊。"""
    from src.services import SLOT_INCOMING, SLOT_OUTGOING, SLOT_REGION
    from src.translation.cache import fingerprint_of

    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools, coordinator = _build(cfg)

    assert translators[SLOT_INCOMING]._binding.impl.model == "incoming-model"
    assert translators[SLOT_OUTGOING]._binding.impl.model == "outgoing-model"
    assert translators[SLOT_REGION]._binding.impl.model == "region-model"
    assert [p.translator for p in pools] == [translators[SLOT_INCOMING]] * 2
    assert cache.fingerprint == fingerprint_of("openai", "incoming-model",
                                               cfg["target_language"])


def test_the_system_pool_translates_through_the_incoming_service(monkeypatch):
    """系統訊息那條池自帶翻譯函式（走快取），它接的必須是收訊那格 —— 這個 closure
    改成別格一樣跑得完，只有在這裡釘住才看得出來。"""
    from src.services import SLOT_INCOMING

    _stub_translation(monkeypatch)
    used = []

    def _record(translator, cache, text):
        used.append(translator)
        return "translated"

    monkeypatch.setattr(main, "translate_and_cache", _record)
    cfg = _three_slot_cfg()
    translators, _cache, pools, coordinator = _build(cfg)
    player_pool, system_pool = pools

    assert player_pool.translate_fn is None   # 玩家對話走池內建的翻譯（吃上下文）
    assert system_pool.translate_fn("hi", None) == "translated"
    assert used == [translators[SLOT_INCOMING]]


def test_changing_only_the_region_slot_leaves_incoming_and_the_cache_alone(monkeypatch):
    """只換了區域翻譯用哪一組，收訊的翻譯器與譯文快取都不該被動到
    （快取作廢＝使用者的系統訊息譯文全部重譯一次）。"""
    from src.services import SLOT_INCOMING, SLOT_REGION, find

    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools, coordinator = _build(cfg)
    fingerprint_before = cache.fingerprint

    find(cfg, cfg["service_slots"][SLOT_REGION])["model"] = "region-model-2"
    main.reconfigure_translation(cfg, translators, cache, pools, coordinator)

    assert translators[SLOT_REGION]._binding.impl.model == "region-model-2"
    assert translators[SLOT_INCOMING]._binding.impl.model == "incoming-model"
    assert cache.fingerprint == fingerprint_before


def test_changing_the_incoming_slot_invalidates_the_cache(monkeypatch):
    from src.services import SLOT_INCOMING, find
    from src.translation.cache import fingerprint_of

    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools, coordinator = _build(cfg)

    find(cfg, cfg["service_slots"][SLOT_INCOMING])["model"] = "incoming-model-2"
    main.reconfigure_translation(cfg, translators, cache, pools, coordinator)

    assert translators[SLOT_INCOMING]._binding.impl.model == "incoming-model-2"
    assert cache.fingerprint == fingerprint_of("openai", "incoming-model-2",
                                               cfg["target_language"])


def test_build_translation_ensures_examples_for_incoming_and_region(monkeypatch):
    from src.services import SLOT_INCOMING, SLOT_REGION

    _stub_translation(monkeypatch)
    translators, _cache, _pools, coordinator = _build(_three_slot_cfg())
    assert coordinator.ensured == [translators[SLOT_INCOMING], translators[SLOT_REGION]]


def test_reconfigure_ensures_examples_only_for_rebuilt_slots(monkeypatch):
    from src.services import SLOT_REGION, find

    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools, coordinator = _build(cfg)
    coordinator.ensured.clear()
    find(cfg, cfg["service_slots"][SLOT_REGION])["model"] = "region-model-2"
    main.reconfigure_translation(cfg, translators, cache, pools, coordinator)
    assert coordinator.ensured == [translators[SLOT_REGION]]


def test_reconfigure_without_changes_keeps_examples_and_does_not_ensure(monkeypatch):
    from src.services import SLOT_INCOMING

    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools, coordinator = _build(cfg)
    incoming = translators[SLOT_INCOMING]
    examples = ExampleSet(("s", "o"), None, None)
    incoming.set_examples(examples, incoming.examples_fingerprint)
    coordinator.on_applied(incoming)
    coordinator.ensured.clear()
    main.reconfigure_translation(cfg, translators, cache, pools, coordinator)
    assert coordinator.ensured == [] and incoming.examples == examples
    assert cache.fingerprint.endswith("|x" + examples.digest())


def test_restart_keeps_the_persisted_cache_when_examples_are_already_stored(monkeypatch, tmp_path):
    """範例已在磁碟時，啟動不可先用無範例指紋載入再 rebind —— 那會把上次的快取清空。"""
    from functools import partial

    from src.services import SLOT_INCOMING, SLOT_REGION, resolve
    from src.translation.cache import TranslationCache
    from src.translation.examples import ExampleCoordinator, ExampleStore
    from src.translation.translator import Translator

    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    probe = Translator(**resolve(cfg, SLOT_INCOMING), target_language=cfg["target_language"])
    examples = ExampleSet(("s", "o"), ("s", "o"), ("s", "o"))
    store_path, cache_file = tmp_path / "examples.json", tmp_path / "cache.json"
    ExampleStore(store_path).put(probe.examples_fingerprint, examples)
    region = Translator(**resolve(cfg, SLOT_REGION), target_language=cfg["target_language"])
    ExampleStore(store_path).put(region.examples_fingerprint, examples)
    seeded = TranslationCache(main.incoming_fingerprint(cfg, examples), cache_file)
    seeded.put("hello", "bonjour", seeded.fingerprint)
    seeded.flush()

    monkeypatch.setattr(main, "TranslationCache", lambda fp: TranslationCache(fp, cache_file))
    monkeypatch.setattr(main, "ExampleStore", lambda: ExampleStore(store_path))
    monkeypatch.setattr(main, "ExampleCoordinator",
                        partial(ExampleCoordinator, spawn=lambda job: job()))
    _translators, cache, _pools, _coordinator = _build(cfg)

    assert cache.get("hello") == "bonjour"
    assert "bonjour" in cache_file.read_text(encoding="utf-8")


def test_a_settings_change_applies_the_new_fingerprints_examples_and_never_the_old(
        monkeypatch, tmp_path):
    """真的協調器與翻譯器：改設定前發出的生成晚到，也不能蓋掉新設定從快取取到的範例。"""
    from functools import partial

    from src.services import SLOT_INCOMING, SLOT_REGION, find, resolve
    from src.translation.examples import ExampleCoordinator, ExampleStore, examples_fingerprint
    from src.translation.translator import Translator

    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    old_fp = examples_fingerprint(resolve(cfg, SLOT_INCOMING), cfg["target_language"])
    old, new = ExampleSet(("s", "old"), None, None), ExampleSet(("s", "new"), None, None)
    store = ExampleStore(tmp_path / "examples.json")
    store.put(examples_fingerprint(resolve(cfg, SLOT_REGION), cfg["target_language"]), new)
    jobs = []
    monkeypatch.setattr(main, "ExampleStore", lambda: store)
    monkeypatch.setattr(main, "ExampleCoordinator", partial(ExampleCoordinator, spawn=jobs.append))
    monkeypatch.setattr(Translator, "generate_examples", lambda self: (old_fp, old))
    translators, cache, pools, coordinator = _build(cfg)
    incoming = translators[SLOT_INCOMING]

    find(cfg, cfg["service_slots"][SLOT_INCOMING])["model"] = "incoming-model-2"
    store.put(examples_fingerprint(resolve(cfg, SLOT_INCOMING), cfg["target_language"]), new)
    main.reconfigure_translation(cfg, translators, cache, pools, coordinator)
    assert incoming.examples == new
    jobs[0]()

    assert len(jobs) == 1 and store.get(old_fp) == old
    assert incoming.examples == new
    assert cache.fingerprint == main.incoming_fingerprint(cfg, new)


def test_applying_incoming_examples_rebinds_the_system_message_cache(monkeypatch):
    from src.services import SLOT_INCOMING, SLOT_REGION

    _stub_translation(monkeypatch)
    translators, cache, _pools, coordinator = _build(_three_slot_cfg())
    incoming = translators[SLOT_INCOMING]
    examples = ExampleSet(("s", "o"), None, None)
    incoming.set_examples(examples, incoming.examples_fingerprint)
    coordinator.on_applied(incoming)
    assert cache.fingerprint.endswith("|x" + examples.digest())
    before = cache.fingerprint
    coordinator.on_applied(translators[SLOT_REGION])
    assert cache.fingerprint == before


def test_build_app_wires_each_consumer_to_its_own_slot():
    """輸入框與區域翻譯的接線只在 build_app 裡（完整啟動才跑得到：開執行緒、掛熱鍵），
    改用原始碼釘住 —— 這兩格打錯一個 token 就是整個功能默默走錯服務。"""
    source = (ROOT / "src" / "main.py").read_text(encoding="utf-8")
    assert "translators[SLOT_OUTGOING].translate_outgoing(" in source
    assert "RegionPipeline(translators[SLOT_REGION])" in source


def test_saving_unrelated_settings_rebuilds_no_translator(monkeypatch):
    """只改了與服務無關的設定（拖了不透明度就按儲存）時，三個後端 client 都要原地不動：
    重建會拆掉連線池，飛行中的請求收到 WinSock 斷線、被判成「翻譯伺服器離線」。"""
    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools, coordinator = _build(cfg)
    coordinator.ensured.clear()
    before = {slot: tr._binding.impl for slot, tr in translators.items()}

    cfg["overlay_alpha"] = 0.5
    main.reconfigure_translation(cfg, translators, cache, pools, coordinator)

    assert {slot: tr._binding.impl for slot, tr in translators.items()} == before


def test_changing_only_the_region_slot_rebuilds_only_that_translator(monkeypatch):
    """「我只改了區域翻譯那一格」不該連收訊與發話的連線一起拆掉。"""
    from src.services import SLOT_INCOMING, SLOT_OUTGOING, SLOT_REGION, find

    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools, coordinator = _build(cfg)
    before = {slot: tr._binding.impl for slot, tr in translators.items()}

    find(cfg, cfg["service_slots"][SLOT_REGION])["model"] = "region-model-2"
    main.reconfigure_translation(cfg, translators, cache, pools, coordinator)

    assert translators[SLOT_REGION]._binding.impl is not before[SLOT_REGION]
    assert translators[SLOT_INCOMING]._binding.impl is before[SLOT_INCOMING]
    assert translators[SLOT_OUTGOING]._binding.impl is before[SLOT_OUTGOING]


@pytest.mark.parametrize("field, value, check", [
    ("model", "outgoing-model-2", lambda impl: impl.model == "outgoing-model-2"),
    ("api_key", "sk-new",
     lambda impl: impl._client.headers["Authorization"] == "Bearer sk-new"),
    ("base_url", "http://elsewhere",
     lambda impl: str(impl._client.base_url) == "http://elsewhere"),
])
def test_editing_the_service_a_slot_uses_rebuilds_it(monkeypatch, field, value, check):
    """真的改到服務欄位時照樣重建，新值要真的生效。"""
    from src.services import SLOT_OUTGOING, find

    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools, coordinator = _build(cfg)
    before = translators[SLOT_OUTGOING]._binding.impl

    find(cfg, cfg["service_slots"][SLOT_OUTGOING])[field] = value
    main.reconfigure_translation(cfg, translators, cache, pools, coordinator)

    assert translators[SLOT_OUTGOING]._binding.impl is not before
    assert check(translators[SLOT_OUTGOING]._binding.impl)


def test_changing_the_target_language_rebuilds_every_translator(monkeypatch):
    """目標語言也是 reconfigure 的參數：三格都要跟上，但發話那格不用範例。"""
    from src.services import SLOT_INCOMING, SLOT_REGION

    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools, coordinator = _build(cfg)
    coordinator.ensured.clear()
    before = {slot: tr._binding.impl for slot, tr in translators.items()}

    cfg["target_language"] = "日本語"
    main.reconfigure_translation(cfg, translators, cache, pools, coordinator)

    for slot, tr in translators.items():
        assert tr._binding.impl is not before[slot]
        assert tr.target_language == "日本語"
    assert coordinator.ensured == [translators[SLOT_INCOMING], translators[SLOT_REGION]]


def _twin_endpoint_cfg():
    """兩筆自訂服務、模型同名、端點不同（本機與遠端各有一個 qwen3）。"""
    from src.services import SLOT_INCOMING, new_service

    cfg = configured_cfg("custom", base_url="http://local", model="qwen3")
    remote = new_service("custom", cfg["services"])
    remote.update(base_url="http://remote", model="qwen3")
    cfg["services"].append(remote)
    cfg["service_slots"][SLOT_INCOMING] = cfg["services"][0]["id"]
    return cfg, remote


def test_switching_the_incoming_slot_between_same_named_models_invalidates_the_cache(
        monkeypatch):
    """兩個端點的模型同名時，換掉收訊那格必須讓舊譯文作廢 —— 指紋不含端點的話，
    舊端點翻的系統訊息譯文會繼續命中。"""
    from src.services import SLOT_INCOMING

    _stub_translation(monkeypatch)
    cfg, remote = _twin_endpoint_cfg()
    translators, cache, pools, coordinator = _build(cfg)
    before = cache.fingerprint

    cfg["service_slots"][SLOT_INCOMING] = remote["id"]
    main.reconfigure_translation(cfg, translators, cache, pools, coordinator)

    assert cache.fingerprint != before


def test_build_app_spawns_readers_through_the_supervisor():
    """接線只在 build_app 裡（完整啟動才跑得到），以原始碼釘住關鍵 token。"""
    source = (ROOT / "src" / "main.py").read_text(encoding="utf-8")
    assert "target=supervise" in source
    assert "MessageLog(message_stream, slot=slot)" in source
