"""main 的純邏輯：設定摘要（啟動與套用設定共用）。"""
import copy
import subprocess
import sys
from pathlib import Path

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
