"""把 Python 行為匯出成 golden fixture，供 Rust 測試逐筆對照。

每個 exporter 回傳 `[{"input": ..., "output": ...}, ...]`，寫到
`rust/tests/golden/<name>.json`；`--check` 只比對不寫入。
"""
import argparse
import copy
import json
import sys
from collections.abc import Callable
from datetime import UTC, datetime, timedelta, timezone
from pathlib import Path

from src import i18n
from src.logs.files import session_header, trim_log_sessions, utc_stamp
from src.ui.fonts import font_family

GOLDEN_DIR = Path(__file__).resolve().parents[2] / "rust" / "tests" / "golden"

EXPORTERS: dict[str, Callable[[], list[dict]]] = {}

_LOG_NOW = datetime(2026, 8, 22, 12, 0, 0, tzinfo=UTC)


def _iso(dt: datetime) -> str:
    return dt.astimezone(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")


def export_logs_trim() -> list[dict]:
    """`trim_log_sessions`：input 為 {text, now}，output 為修剪後文字。"""
    nl = chr(10)

    def sess(dt: datetime, *lines: str) -> str:
        return session_header(dt) + nl + "".join(ln + nl for ln in lines)

    now = _LOG_NOW
    old = sess(now - timedelta(days=10), "[app] old stuff")
    recent = sess(now - timedelta(days=2), "[app] recent", "[reader] attached")
    texts = [
        "",
        recent,
        sess(now - timedelta(days=7, hours=1), "[app] just expired"),
        sess(now - timedelta(days=7), "[app] exactly on cutoff"),
        old + recent,
        recent + old + recent,
        "[app] running without header" + nl + sess(now, "[app] new format"),
        "===== session started not-a-date =====" + nl + "[app] noise" + nl + sess(now, "[app] good"),
        sess(now, "[app] no trailing newline").rstrip(nl),
        sess(now, "[app] 中文 日本語"),
    ]
    return [{"input": {"text": t, "now": _iso(now)}, "output": trim_log_sessions(t, now)}
            for t in texts]


def export_logs_stamp() -> list[dict]:
    """`utc_stamp`：input 為帶偏移的 ISO 時間，output 為毫秒時戳。"""
    cases = [
        datetime(2026, 8, 25, 9, 12, 3, 412345, tzinfo=UTC),
        datetime(2026, 8, 25, 17, 12, 3, 0, tzinfo=timezone(timedelta(hours=8))),
        datetime(2026, 1, 1, 0, 0, 0, 999999, tzinfo=UTC),
        datetime(2026, 10, 17, 1, 2, 3, 456000, tzinfo=UTC),
        datetime(2026, 12, 31, 23, 30, 0, 5000, tzinfo=timezone(timedelta(hours=-5))),
    ]
    return [{"input": dt.isoformat(), "output": utc_stamp(dt)} for dt in cases]


EXPORTERS["logs_trim"] = export_logs_trim
EXPORTERS["logs_stamp"] = export_logs_stamp

LOCALES = sorted(path.stem for path in (Path(i18n.__file__).parent).glob("*.json"))
_EXTRA_TAGS = [
    "zh", "zh-HK", "zh-SG", "zh-MO", "zh-Hant", "zh-Hans", "zh-Hant-CN", "zh-Hans-TW",
    "zh-CHT", "zh_TW", "zh-tw", "ZH-hk", "zh-yue", "yue-HK", "yue", "wuu", "cmn-Hans-CN",
    "zh-Latn", "en", "en-GB", "en-AU", "EN-us", "en_US", "pt", "es-419", "fr-CA",
    "de-AT", "sr-Latn-RS", "nb", "nn", "nb-NO", "no", "iw", "und", "xx-XX", "x-", "",
]


def export_i18n_best_match() -> list[dict]:
    """`best_match`：input 為偏好標籤清單，output 為語言碼。"""
    lists = [[tag] for tag in LOCALES + _EXTRA_TAGS]
    lists += [[], ["xx-XX", "zh-CN", "en-US"], ["xx-XX", "yy-YY"], ["ja-JP", "zh-HK"],
              ["fr-FR", "zh-SG", "en-GB"], ["", "zh-TW"]]
    return [{"input": tags, "output": i18n.best_match(tags)} for tags in lists]


def export_i18n_available() -> list[dict]:
    """`available_languages`：output 為依序的 [語言碼, 自稱] 配對。"""
    return [{"input": None, "output": [[c, n] for c, n in i18n.available_languages().items()]}]


def export_i18n_t() -> list[dict]:
    """`t`：input 為 {lang, key, args}，output 為字串（含缺 key、缺參數的退路）。"""
    import re
    keys = set()
    for code in ("en-US", "zh-TW", "zh-CN"):
        keys |= set(i18n._load(code))
    cases = []
    for lang in ("zh-TW", "en-US", "zh-CN", "ja-JP"):
        i18n.set_language(lang)
        for key in sorted(keys) + ["no.such.key"]:
            names = set(re.findall(r"\{(\w+)\}", i18n._load("en-US").get(key, "")))
            full = {n: f"<{n}>" for n in sorted(names)}
            for args in (full, {}):
                cases.append({"input": {"lang": lang, "key": key, "args": args},
                              "output": i18n.t(key, **args)})
                if not names:
                    break
    i18n.set_language("en-US")
    return cases


def export_fonts_family() -> list[dict]:
    """`font_family`：input 為語言標籤，output 為字族。"""
    tags = LOCALES + ["zh", "zh-HK", "zh-Hant", "zh-SG", "ja", "ko", "xx-XX", "yue"]
    return [{"input": tag, "output": font_family(tag)} for tag in tags]


EXPORTERS["i18n_best_match"] = export_i18n_best_match
EXPORTERS["i18n_available"] = export_i18n_available
EXPORTERS["i18n_t"] = export_i18n_t
EXPORTERS["fonts_family"] = export_fonts_family


def _canon_ids(cfg: dict, source: str, *texts: str) -> list[str]:
    """把 `source` 裡沒有的服務 id（隨機產生的）依出現順序換成 `@id1`、`@id2`……"""
    services = cfg.get("services")
    ids = []
    for service in services if isinstance(services, list) else []:
        sid = service.get("id") if isinstance(service, dict) else None
        if isinstance(sid, str) and sid not in source and sid not in ids:
            ids.append(sid)
    out = []
    for text in texts:
        for n, sid in enumerate(ids, 1):
            text = text.replace(sid, f"@id{n}")
        out.append(text)
    return out


def _canon_cfg(cfg: dict, source: str) -> dict:
    return json.loads(_canon_ids(cfg, source, json.dumps(cfg, ensure_ascii=False))[0])


def _config_in_english():
    """匯出期間固定介面語言（自訂端點的預設名稱走語言檔）。"""
    i18n.set_language("en-US")
    from src import config
    return config


_SVC_OPENAI = {"id": "s1", "name": "GPT", "provider": "openai", "model": "gpt-x",
               "api_key": "SK-OPENAI", "thinking": False}
_SVC_CLAUDE = {"id": "s2", "name": "Claude", "provider": "claude", "model": "claude-x",
               "api_key": "SK-CLAUDE", "effort": "auto"}
_SVC_CUSTOM = {"id": "s3", "name": "Local", "provider": "custom", "base_url": "http://x",
               "model": "gemma", "api_key": "", "thinking": True}
_FULL_API = {"provider": "claude",
             "openai": {"model": "gpt-x", "api_key": "sk-1", "thinking": False},
             "claude": {"model": "claude-x", "api_key": "sk-ant", "effort": "auto"},
             "custom": {"base_url": "", "model": "", "api_key": "", "thinking": False}}
_EMPTY_API = {"provider": "openai",
              "openai": {"model": "", "api_key": "", "thinking": False},
              "claude": {"model": "", "api_key": "", "effort": "auto"},
              "custom": {"base_url": "", "model": "", "api_key": "", "thinking": False}}


def _load_inputs() -> list[bytes]:
    def js(value) -> bytes:
        return json.dumps(value, ensure_ascii=False).encode("utf-8")

    config = _config_in_english()
    current = copy.deepcopy(config.DEFAULT_CONFIG)
    current.update(services=[_SVC_OPENAI, _SVC_CLAUDE], default_service="s1",
                   service_slots={"incoming": None, "outgoing": "s2", "region": None},
                   ui_language="ja-JP", target_language="日本語")
    return [
        b"",
        b'{"hotkey": "f8",}',
        b'\xff\xfe{"hotkey": "f8"}',
        '\ufeff{"hotkey": "f8"}'.encode("utf-8"),
        b"[1, 2]",
        b'"just a string"',
        b"null",
        b"{}",
        js({"fade_seconds": 30}),
        js({"hotkey": "ctrl+alt+t", "game_path": r"D:\Games\Wizard101"}),
        js({"poll_interval": 0.5}),
        js(current),
        json.dumps(current, ensure_ascii=False, indent=2).replace("\n", "\r\n").encode(),
        js({"fade_seconds": True, "max_messages": "300", "type_delay": None,
            "overlay_alpha": [1], "max_parallel_translations": {}}),
        js({"fade_seconds": -5, "max_messages": 99999, "type_delay": 9.0,
            "overlay_alpha": 0.1, "max_parallel_translations": 0}),
        js({"fade_seconds": 5.5, "max_messages": 10.0, "type_delay": 0,
            "overlay_alpha": 1, "max_parallel_translations": 8.0, "poll_interval": 1}),
        js({"services": [{**_SVC_OPENAI, "thinking": 0}], "default_service": "s1"}),
        js({"services": [{**_SVC_OPENAI, "thinking": 1}], "default_service": "s1"}),
        js({"services": [{**_SVC_OPENAI, "id": 7, "name": 42, "model": 5}]}),
        js({"api": {"provider": "custom", "base_url": "http://127.0.0.1:8000",
                    "model": "gemma", "api_key": "sk-1"}}),
        js({"api": {"base_url": "http://127.0.0.1:8000", "model": "m1"}}),
        js({"api": {"provider": None, "model": "g", "api_key": "SK-NULL"}}),
        js({"api": {"provider": "mistral", "model": "g", "api_key": "SK-MISTRAL-FLAT"}}),
        js({"api": _FULL_API}),
        js({"api": _EMPTY_API}),
        js({"services": [_SVC_CLAUDE], "default_service": "s2", "api": _EMPTY_API}),
        js({"api": {"provider": "mistral", "mistral": {"api_key": "SK-MISTRAL"},
                    "openai": {"model": "gpt-x", "api_key": "SK-OPENAI"}}}),
        js({"api": "junk", "hotkey": "x"}),
        js({"services": [{"id": "aaaaaaaa", "name": "X", "provider": "mistral",
                          "model": "g", "api_key": "sk-REAL-USER-KEY", "quirk": True}]}),
        js({"services": [_SVC_OPENAI, {**_SVC_OPENAI, "name": "Other"},
                         {**_SVC_CLAUDE, "name": "GPT"}],
            "default_service": "nope",
            "service_slots": {"incoming": "s9", "outgoing": ["x"], "region": 3, "bogus": "x"}}),
        js({"services": [_SVC_OPENAI], "default_service": []}),
        js({"services": "SK-PASTED", "unsupported_services": "SK-OTHER"}),
        js({"services": dict(_SVC_OPENAI)}),
        js({"unsupported_services": [{"provider": "openai", "name": "GPT",
                                      "api_key": "sk-3"}],
            "services": [_SVC_OPENAI]}),
        js({"services": [{**_SVC_CUSTOM, "name": "我的服務 \"引號\" 'q' \\ \u0001 \u2028 😀",
                          "stale": 1}],
            "target_language": "日本語",
            "extra": [1e-05, 1e16, 123.0, -0.0, 1e100, 0.30000000000000004, 0.0001,
                      1.5e300, 12345678901234567, -0, 1e2, 5e-324]}),
        b'{"hotkey": "a", "fade_seconds": 5, "hotkey": "b", "poll_interval": 1}',
        js({"overlay": {"x": 10, "extra": 1}, "bubble_position": None,
            "service_slots": {"incoming": "nope"}, "poll_interval": 2}),
    ]


def export_config_load() -> list[dict]:
    """`load_config`：input 為檔案原始 bytes（hex），output 為結果與寫回的 bytes（hex，沒寫回為 null）。"""
    import tempfile
    config = _config_in_english()
    cases = []
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "config.json"
        for raw in _load_inputs():
            path.write_bytes(raw)
            cfg = config.load_config(path)
            after = path.read_bytes()
            source = raw.decode("utf-8", errors="replace")
            cfg_text, after_text = _canon_ids(
                cfg, source, json.dumps(cfg, ensure_ascii=False),
                after.decode("utf-8", errors="replace"))
            cases.append({"input": raw.hex(), "output": {
                "config": json.loads(cfg_text),
                "written": None if after == raw else after_text.encode("utf-8").hex()}})
    return cases


def _normalize_inputs() -> list[dict]:
    def section(**extra):
        return {"services": [], "default_service": None,
                "service_slots": {"incoming": None, "outgoing": None, "region": None},
                **extra}

    unknown = {"id": "x1", "name": "Mistral", "provider": "mistral", "api_key": "SK-MISTRAL"}
    promoted = {"id": "p1", "name": "ChatGPT", "provider": "openai", "model": "gpt-x",
                "api_key": "SK-PROMOTED", "thinking": False}
    claude_api = {"provider": "claude",
                  "claude": {"model": "claude-x", "api_key": "SK-CLAUDE-OLD", "effort": "auto"}}
    return [
        section(api={"provider": "custom", "base_url": "http://127.0.0.1:8000",
                     "model": "gemma", "api_key": "sk-1"}),
        section(api={"base_url": "http://127.0.0.1:8000", "model": "m1"}),
        section(api=_FULL_API),
        section(api=_EMPTY_API),
        section(services=[_SVC_CLAUDE], default_service="s2", api=_EMPTY_API),
        section(services=[{"id": "aaaaaaaa", "name": "X", "provider": "mistral", "model": "g",
                           "api_key": "sk-REAL-USER-KEY", "quirk": 1}]),
        section(services=[_SVC_OPENAI, unknown], default_service="x1",
                service_slots={"incoming": "x1", "outgoing": None, "region": None}),
        section(unsupported_services="sk-PASTED-IN-THE-WRONG-PLACE",
                services=["sk-ANOTHER-STRAY-KEY"]),
        section(services=[{**_SVC_OPENAI, "name": "OpenAI"}],
                unsupported_services=[{"provider": "openai", "name": "OpenAI", "api_key": "sk-3"}]),
        section(services=[unknown], api={"provider": "openai", "openai": {
            "model": "gpt-x", "api_key": "SK-OPENAI", "thinking": False}}),
        section(services=[_SVC_CLAUDE], default_service="s2", api={"provider": "openai", "openai": {
            "model": "gpt-stale", "api_key": "SK-STALE", "thinking": False}}),
        section(services="SK-PASTED"),
        section(services={"a": {"provider": "openai", "api_key": "SK-IN-DICT"}}),
        section(services="SK-PASTED", unsupported_services=[
            {"provider": "openai", "name": "ChatGPT", "api_key": "sk-3"}]),
        section(services=dict(_SVC_OPENAI)),
        section(services=dict(unknown)),
        section(unsupported_services=[promoted], api=claude_api),
        section(unsupported_services=[{"id": "zzz1", "provider": "openai",
                                       "api_key": "SK-NONAME"}], api=claude_api),
        section(unsupported_services=[promoted], api={**claude_api, "provider": "nonsense"}),
        section(api={"provider": "mistral", "mistral": {"model": "g", "api_key": "SK-MISTRAL"}}),
        section(api={"provider": "mistral", "model": "g", "api_key": "SK-MISTRAL-FLAT"}),
        section(api={"provider": None, "model": "g", "api_key": "SK-NULL"}),
        section(api={"provider": "mistral", "mistral": {"api_key": "SK-MISTRAL"},
                     "openai": {"model": "gpt-x", "api_key": "SK-OPENAI", "thinking": False}}),
        section(services=[_SVC_OPENAI], default_service="s1",
                api={"provider": "mistral", "mistral": {"model": "g", "api_key": "SK-MISTRAL"}}),
        section(services=[{"provider": "openai"}],
                api={"provider": "openai", "openai": {"model": "m", "api_key": "SK-REAL-OLD"}}),
        section(services=[{"provider": "openai", "model": 7, "api_key": None}],
                api={"provider": "openai", "openai": {"model": "m", "api_key": "SK-REAL-OLD"}}),
        {"default_service": None, "service_slots": {}},
        {"unsupported_services": [{"provider": "openai", "api_key": "SK-1"}]},
        {},
        {"api": {"provider": "openai", "openai": {"model": "m", "api_key": "SK-2"}}},
        section(services="SK-PASTED-OVER-THE-LIST", api={"provider": "openai", "openai": {
            "model": "gpt-x", "api_key": "SK-OPENAI", "thinking": False}}),
        section(services=[{"id": "aaaaaaaa", "name": "C", "provider": "claude",
                           "model": "claude-x", "thinking": True}]),
        section(services=[{**_SVC_OPENAI, "name": "A", "id": "dup"},
                          {**_SVC_OPENAI, "name": "B", "id": "dup"}]),
        section(services=[_SVC_OPENAI], default_service="nope"),
        section(services=[_SVC_OPENAI], default_service="s1",
                service_slots={"incoming": "nope", "outgoing": None, "region": None,
                               "bogus": "x"}),
        section(services=[_SVC_OPENAI], default_service="s1"),
        section(api={"provider": "openai", "openai": {"model": "gpt-x", "api_key": None}}),
        section(api={"provider": "custom", "base_url": "http://x", "model": None,
                     "api_key": "sk-REAL-USER-KEY"}),
        section(services=[{**_SVC_OPENAI, "model": 5, "thinking": "yes"}], default_service="s1"),
        section(services=[{**_SVC_OPENAI, "thinking": 0}], default_service="s1"),
        section(services=[{"id": "aaaaaaaa", "name": "X", "provider": ["openai"], "model": "m"}]),
        section(services=[{**_SVC_OPENAI, "id": 7, "name": 42}]),
        section(services=[_SVC_OPENAI], default_service=[]),
        section(services=[_SVC_OPENAI], default_service="s1",
                service_slots={"incoming": ["x"], "outgoing": 3, "region": None}),
        section(services=[_SVC_OPENAI, {**_SVC_OPENAI, "id": "s9"}], default_service="s1"),
        section(services=[{"provider": "custom"}, {"provider": "custom", "name": ""}]),
        section(services=[_SVC_OPENAI], default_service="s1", service_slots="junk"),
    ]


def export_config_normalize() -> list[dict]:
    """`normalize`：input 為 cfg，output 為 {changed, changed_again, cfg}（隨機 id 已正規化）。"""
    config = _config_in_english()
    cases = []
    for cfg in _normalize_inputs():
        source = json.dumps(cfg, ensure_ascii=False)
        work = copy.deepcopy(cfg)
        changed = config.normalize(work)
        again = config.normalize(work)
        cases.append({"input": cfg, "output": {"changed": changed, "changed_again": again,
                                               "cfg": _canon_cfg(work, source)}})
    return cases


def export_config_resolve() -> list[dict]:
    """`resolve`：input 為 {cfg, slot}，output 為攤平的服務（另附選中服務的 id／name）。"""
    from src.config.services import SLOTS, find, resolve
    _config_in_english()
    cfgs = [
        {"services": [], "default_service": None, "service_slots": dict.fromkeys(SLOTS)},
        {"services": [_SVC_OPENAI, _SVC_CLAUDE, _SVC_CUSTOM], "default_service": "s1",
         "service_slots": {"incoming": None, "outgoing": "s2", "region": "s3"}},
        {"services": [_SVC_CLAUDE, _SVC_CUSTOM], "default_service": "s3",
         "service_slots": {"incoming": "s2", "outgoing": None, "region": None}},
    ]
    cases = []
    for cfg in cfgs:
        for slot in SLOTS:
            chosen = find(cfg, cfg["service_slots"].get(slot)) or find(cfg, cfg["default_service"])
            out = resolve(cfg, slot)
            out.update(id=(chosen or {}).get("id", ""), name=(chosen or {}).get("name", ""))
            cases.append({"input": {"cfg": cfg, "slot": slot}, "output": out})
    return cases


def export_config_validate() -> list[dict]:
    """`validate_service`：input 為服務，output 為錯誤文案 key 清單。"""
    from src.config.services import validate_service
    services = [
        {"provider": "openai", "model": "", "api_key": ""},
        {"provider": "openai", "model": "m", "api_key": "k"},
        {"provider": "claude", "model": "  ", "api_key": "\u3000"},
        {"provider": "gemini", "model": "g", "api_key": "\x1c\t"},
        {"provider": "custom", "base_url": "", "model": "m", "api_key": ""},
        {"provider": "custom", "base_url": "http://x", "model": "\u2028", "api_key": ""},
        _SVC_CUSTOM, _SVC_CLAUDE,
    ]
    return [{"input": s, "output": validate_service(s)} for s in services]


def export_config_summary() -> list[dict]:
    """`config_summary`：input 為 cfg，output 為摘要字串。"""
    config = _config_in_english()
    base = copy.deepcopy(config.DEFAULT_CONFIG)
    cases = [copy.deepcopy(base)]
    cfg = copy.deepcopy(base)
    cfg.update(services=[_SVC_OPENAI, _SVC_CLAUDE, _SVC_CUSTOM], default_service="s3",
               service_slots={"incoming": None, "outgoing": "s2", "region": "s1"},
               unsupported_services=[{"provider": "mistral"}, "x"], ui_language="ja-JP",
               target_language="日本語", max_parallel_translations=2.0, overlay_alpha=1.0,
               paste_hotkey=False, hotkey=["odd"])
    cases.append(cfg)
    cases.append({**copy.deepcopy(cfg), "target_language": "it's \"q\" \\ \n\t\x7f\xa0",
                  "region_hotkey": None})
    return [{"input": c, "output": config.config_summary(c)} for c in cases]


def export_config_bootstrap_language() -> list[dict]:
    """`bootstrap_language`：detect 注入固定值；output 為語言碼、目標語言與 detect 是否被呼叫。"""
    config = _config_in_english()
    cases = []
    for ui_language in (None, "ja-JP"):
        for existed in (False, True):
            for detected in ("ja-JP", "xx-XX"):
                cfg = {"ui_language": ui_language, "target_language": "Original"}
                called = []

                def detect(value=detected, called=called):
                    called.append(True)
                    return value

                code = config.bootstrap_language(cfg, existed, detect=detect)
                cases.append({"input": {"ui_language": ui_language, "config_existed": existed,
                                        "detected": detected},
                              "output": {"code": code, "target_language": cfg["target_language"],
                                         "detect_called": bool(called)}})
    return cases


EXPORTERS["config_load"] = export_config_load
EXPORTERS["config_normalize"] = export_config_normalize
EXPORTERS["config_resolve"] = export_config_resolve
EXPORTERS["config_validate"] = export_config_validate
EXPORTERS["config_summary"] = export_config_summary
EXPORTERS["config_bootstrap_language"] = export_config_bootstrap_language


def _render(cases: list[dict]) -> str:
    return json.dumps(cases, ensure_ascii=False, indent=1) + "\n"


def main(argv: list[str] | None = None, out_dir: Path = GOLDEN_DIR) -> int:
    """匯出（或以 --check 比對）指定名稱的 fixture，缺省為全部；回傳結束碼。"""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("names", nargs="*", help="exporter 名稱，缺省為全部")
    parser.add_argument("--check", action="store_true", help="只比對已提交的檔案，不同就非零結束")
    args = parser.parse_args(argv)

    unknown = [n for n in args.names if n not in EXPORTERS]
    if unknown:
        print(f"unknown exporter: {', '.join(unknown)}", file=sys.stderr)
        return 2

    stale = []
    for name in args.names or EXPORTERS:
        text = _render(EXPORTERS[name]())
        path = out_dir / f"{name}.json"
        if args.check:
            if not path.is_file() or path.read_bytes() != text.encode("utf-8"):
                stale.append(name)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(text.encode("utf-8"))
    if stale:
        print(f"golden fixtures out of date: {', '.join(stale)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
