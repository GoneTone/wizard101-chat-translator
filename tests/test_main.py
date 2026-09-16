"""main 的純邏輯：設定摘要（啟動與套用設定共用）。"""
import copy
import subprocess
import sys
from pathlib import Path

from src import main
from src.config import DEFAULT_CONFIG, active_api
from src.main import config_summary


def test_config_summary_covers_the_default_config_without_the_api_key():
    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["api"]["custom"].update(base_url="http://x", model="gemma", api_key="sk-secret")
    cfg["api"]["provider"] = "custom"
    summary = config_summary(cfg, active_api(cfg))
    assert "model=gemma" in summary
    assert "sk-secret" not in summary
    # 每個使用者可調的設定都要在摘要裡，回報問題時才不必追問
    for key in ("target_language", "hotkey", "region_hotkey",
                "paste_hotkey", "auto_show_input", "poll_interval", "fade_seconds",
                "max_messages", "overlay_alpha", "translate_system_messages"):
        assert f"{key}=" in summary


def test_splash_closes_before_focusing_an_existing_instance(monkeypatch):
    """第二份實例：啟動畫面要在既有視窗被喚起「之前」關掉，否則會蓋在它上面。

    這條路徑在建立任何視窗之前就 return，所以測得到；另外兩個出口（首次執行精靈、
    正常啟動）會進 Tk 與 mainloop，改由打包後的實機驗證涵蓋。
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
    """啟動路徑不得載入 anthropic（約 0.7 秒，只有 Claude 官方 provider 用得到）。

    必須另起乾淨的直譯器問：同一個 process 內別的測試早就把 anthropic 載進來了。
    這條性質會被任何一次無心的 import 悄悄破壞，且不會有別的測試變紅。
    """
    code = "import src.main, sys; print('anthropic' in sys.modules)"
    out = subprocess.run([sys.executable, "-c", code], capture_output=True,
                         text=True, cwd=ROOT, check=True)
    assert out.stdout.strip() == "False", f"stdout={out.stdout!r} stderr={out.stderr!r}"
