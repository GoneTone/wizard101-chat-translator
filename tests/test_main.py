"""main 的純邏輯：設定摘要（啟動與套用設定共用）。"""
import copy
import subprocess
import sys
from pathlib import Path

import pytest

from src import main
from src.config import DEFAULT_CONFIG
from src.main import config_summary
from tests.config_helpers import configured_cfg


def test_config_summary_lists_every_slot_and_never_leaks_the_key():
    cfg = configured_cfg("custom", base_url="http://x", model="gemma",
                         api_key="sk-secret")
    summary = config_summary(cfg)
    assert "sk-secret" not in summary
    assert "has_key=True" in summary
    assert "incoming=default" in summary and "region=default" in summary
    # 隔離筆數：不帶內容，但「服務不見了」的回報要看得出有沒有東西被隔離
    assert "quarantined=0" in summary
    # 每個使用者可調的設定都要在摘要裡，回報問題時才不必追問
    for key in ("target_language", "hotkey", "region_hotkey",
                "paste_hotkey", "auto_show_input", "poll_interval", "fade_seconds",
                "max_messages", "overlay_alpha", "translate_system_messages"):
        assert f"{key}=" in summary


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


def _stub_translation(monkeypatch):
    """換掉會開執行緒與碰磁碟的兩個元件，其餘（三個翻譯器、指紋）維持真貨。"""
    monkeypatch.setattr(main, "TranslationPool", _FakePool)
    monkeypatch.setattr(main, "TranslationCache", _FakeCache)


def test_each_slot_gets_its_own_translator_and_the_cache_follows_incoming(monkeypatch):
    """三個用途各接到自己那格的服務，兩條翻譯池都吃收訊那格，快取指紋也綁收訊。"""
    from src.services import SLOT_INCOMING, SLOT_OUTGOING, SLOT_REGION
    from src.translation.cache import fingerprint_of

    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools = main.build_translation(cfg, lambda *a: None)

    assert translators[SLOT_INCOMING]._impl.model == "incoming-model"
    assert translators[SLOT_OUTGOING]._impl.model == "outgoing-model"
    assert translators[SLOT_REGION]._impl.model == "region-model"
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
    translators, _cache, pools = main.build_translation(cfg, lambda *a: None)
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
    translators, cache, pools = main.build_translation(cfg, lambda *a: None)
    fingerprint_before = cache.fingerprint

    find(cfg, cfg["service_slots"][SLOT_REGION])["model"] = "region-model-2"
    main.reconfigure_translation(cfg, translators, cache, pools)

    assert translators[SLOT_REGION]._impl.model == "region-model-2"
    assert translators[SLOT_INCOMING]._impl.model == "incoming-model"
    assert cache.fingerprint == fingerprint_before


def test_changing_the_incoming_slot_invalidates_the_cache(monkeypatch):
    from src.services import SLOT_INCOMING, find
    from src.translation.cache import fingerprint_of

    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools = main.build_translation(cfg, lambda *a: None)

    find(cfg, cfg["service_slots"][SLOT_INCOMING])["model"] = "incoming-model-2"
    main.reconfigure_translation(cfg, translators, cache, pools)

    assert translators[SLOT_INCOMING]._impl.model == "incoming-model-2"
    assert cache.fingerprint == fingerprint_of("openai", "incoming-model-2",
                                               cfg["target_language"])


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
    translators, cache, pools = main.build_translation(cfg, lambda *a: None)
    before = {slot: tr._impl for slot, tr in translators.items()}

    cfg["overlay_alpha"] = 0.5
    main.reconfigure_translation(cfg, translators, cache, pools)

    assert {slot: tr._impl for slot, tr in translators.items()} == before


def test_changing_only_the_region_slot_rebuilds_only_that_translator(monkeypatch):
    """「我只改了區域翻譯那一格」不該連收訊與發話的連線一起拆掉。"""
    from src.services import SLOT_INCOMING, SLOT_OUTGOING, SLOT_REGION, find

    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools = main.build_translation(cfg, lambda *a: None)
    before = {slot: tr._impl for slot, tr in translators.items()}

    find(cfg, cfg["service_slots"][SLOT_REGION])["model"] = "region-model-2"
    main.reconfigure_translation(cfg, translators, cache, pools)

    assert translators[SLOT_REGION]._impl is not before[SLOT_REGION]
    assert translators[SLOT_INCOMING]._impl is before[SLOT_INCOMING]
    assert translators[SLOT_OUTGOING]._impl is before[SLOT_OUTGOING]


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
    translators, cache, pools = main.build_translation(cfg, lambda *a: None)
    before = translators[SLOT_OUTGOING]._impl

    find(cfg, cfg["service_slots"][SLOT_OUTGOING])[field] = value
    main.reconfigure_translation(cfg, translators, cache, pools)

    assert translators[SLOT_OUTGOING]._impl is not before
    assert check(translators[SLOT_OUTGOING]._impl)


def test_changing_the_target_language_rebuilds_every_translator(monkeypatch):
    """目標語言也是 reconfigure 的參數：三格都要跟上。"""
    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools = main.build_translation(cfg, lambda *a: None)
    before = {slot: tr._impl for slot, tr in translators.items()}

    cfg["target_language"] = "日本語"
    main.reconfigure_translation(cfg, translators, cache, pools)

    for slot, tr in translators.items():
        assert tr._impl is not before[slot]
        assert tr.target_language == "日本語"


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
    translators, cache, pools = main.build_translation(cfg, lambda *a: None)
    before = cache.fingerprint

    cfg["service_slots"][SLOT_INCOMING] = remote["id"]
    main.reconfigure_translation(cfg, translators, cache, pools)

    assert cache.fingerprint != before


# --- 雙開：聊天框事件只認前景客戶端，Ctrl+V 單行判定看前景視窗 ---
class _FakeInputBox:
    def __init__(self):
        self.calls: list = []

    def set_anchor(self, rect):
        self.calls.append(("anchor", rect))

    def clear_anchor(self):
        self.calls.append("clear_anchor")

    def show(self):
        self.calls.append("show")

    def hide(self):
        self.calls.append("hide")


def _events(auto_show=True, foreground=0xA):
    box = _FakeInputBox()
    tracker = main.ChatInputTracker()
    ev = main.GameInputEvents({"auto_show_input": auto_show}, box, tracker,
                              foreground=lambda: foreground)
    return ev, box, tracker


def test_open_in_the_foreground_window_anchors_and_shows():
    ev, box, tracker = _events(foreground=0xA)
    ev.opened(0xA, (1, 2, 3, 4))
    assert box.calls == [("anchor", (1, 2, 3, 4)), "show"]
    assert tracker.is_open(0xA)


def test_open_in_a_background_window_is_tracked_but_not_shown():
    # 使用者正在玩 B，A 的聊天框開了：不彈框、不改錨點，但記得 A 開著（貼上判定要用）
    ev, box, tracker = _events(foreground=0xB)
    ev.opened(0xA, (1, 2, 3, 4))
    assert box.calls == []
    assert tracker.is_open(0xA)


def test_close_from_another_window_does_not_hide_the_box():
    ev, box, tracker = _events(foreground=0xB)
    ev.opened(0xB, None)
    ev.opened(0xA, None)     # A 在背景開著
    ev.closed(0xA)           # A 關了：不能收掉為 B 呼出的框
    assert "hide" not in box.calls
    assert not tracker.is_open(0xA) and tracker.is_open(0xB)
    ev.closed(0xB)
    assert box.calls[-2:] == ["clear_anchor", "hide"]


def test_auto_show_off_still_tracks_and_anchors():
    ev, box, tracker = _events(auto_show=False, foreground=0xA)
    ev.opened(0xA, (1, 2, 3, 4))
    ev.closed(0xA)
    assert box.calls == [("anchor", (1, 2, 3, 4)), "clear_anchor"]


def test_paste_single_line_follows_the_foreground_window(monkeypatch):
    # 在 B 貼多行、只有 B 的聊天框開著：要走單行；A 開著、前景是 B 且 B 沒開：多行
    seen = []
    monkeypatch.setattr(main, "paste_clipboard",
                        lambda hwnd, delay, single_line: seen.append((hwnd, single_line)))
    monkeypatch.setattr(main.win32gui, "GetForegroundWindow", lambda: 0xB)
    tracker = main.ChatInputTracker()
    tracker.opened(0xB)
    main.on_paste_hotkey({"type_delay": 0}, tracker).join()
    tracker.closed(0xB)
    tracker.opened(0xA)
    main.on_paste_hotkey({"type_delay": 0}, tracker).join()
    assert seen == [(0xB, True), (0xB, False)]


def test_build_app_spawns_readers_through_the_supervisor():
    """接線只在 build_app 裡（完整啟動才跑得到），以原始碼釘住關鍵 token。"""
    source = (ROOT / "src" / "main.py").read_text(encoding="utf-8")
    assert "target=supervise" in source
    assert "MessageLog(message_stream, slot=slot)" in source
    assert "overlay.set_multi_client" in source
