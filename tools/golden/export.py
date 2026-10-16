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
from src.translation.clients import _OpenAICompatClient, error_detail, rejected_parameter
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
        b'{"x": -0, "y": 123456789012345678901234567890, "w": [-0.0, 1E2, 1.5e-7, -12],'
        b' "fade_seconds": -0, "poll_interval": 1}',
        b'{"x": -0, "y": -123456789012345678901234567890, "max_messages": 1e400}',
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


_TARGETS = ["繁體中文 (台灣)", "English", "日本語", "Español", " english "]


def export_prompts_systems() -> list[dict]:
    """四條 system prompt：input 為 {kind, target, strict}，output 為提示詞全文。"""
    from src.translation import prompts as p
    cases = []
    for target in _TARGETS:
        cases.append({"input": {"kind": "incoming", "target": target, "strict": False},
                      "output": p.build_incoming_system(target)})
        cases.append({"input": {"kind": "outgoing", "target": target, "strict": False},
                      "output": p.build_outgoing_system(target)})
        cases.append({"input": {"kind": "region", "target": target, "strict": False},
                      "output": p.build_region_system(target)})
        for strict in (False, True):
            cases.append({"input": {"kind": "system_message", "target": target, "strict": strict},
                          "output": p.build_system_message_system(target, strict)})
    return cases


def export_prompts_turns() -> list[dict]:
    """`build_turns`／`example_turns`／`is_game_language` 與常數。"""
    from src.translation import prompts as p
    ex = p.example_turns(("你好", "hello"))
    turns = [
        (["[A] hi", "[B] 你好"], "[C] 在嗎", p.CONTEXT_INTRO_INCOMING, None),
        ([], "[C] 在嗎", p.CONTEXT_INTRO_INCOMING, None),
        (["a"], "b", p.CONTEXT_INTRO_OUTGOING, p.FEWSHOT_OUTGOING),
        ([], "b", p.CONTEXT_INTRO_OUTGOING, p.FEWSHOT_OUTGOING),
        (["x", "y"], "z", "intro", ex),
        ([], "z", "intro", []),
    ]
    cases = [{"input": {"fn": "build_turns", "context": c, "text": t, "intro": i, "examples": e or []},
              "output": p.build_turns(c, t, i, e)} for c, t, i, e in turns]
    for pair in (("你好", "hello"), None, ("", "")):
        cases.append({"input": {"fn": "example_turns", "pair": pair},
                      "output": p.example_turns(pair)})
    for lang in ["English", "english", "  ENGLISH  ", "English (US)", "", "Español", "ſ", "日本語"]:
        cases.append({"input": {"fn": "is_game_language", "lang": lang},
                      "output": p.is_game_language(lang)})
    cases.append({"input": {"fn": "constants"}, "output": {
        "outgoing_language": p.OUTGOING_LANGUAGE, "prompt_revision": p.PROMPT_REVISION,
        "intro_incoming": p.CONTEXT_INTRO_INCOMING, "intro_outgoing": p.CONTEXT_INTRO_OUTGOING,
        "ack": p.CONTEXT_ACK, "fewshot": p.FEWSHOT_OUTGOING}})
    return cases


_SENDER_TEXTS = [
    "[Amy] 你好", "[Amy] hello (world)", "[贾斯廷 渡鸦] 木头", "plain text", "[A][B] hi",
    "[WTS] gear", "[Amy]   spaced", "", "[Amy]", "[" + "x" * 40 + "] ok", "[" + "x" * 41 + "] ok",
    "[Amy] 😀 test", "[Amy] é combining", "[Amy] line sep", "[Amy] Straße strasse",
    "[Amy] \x0b vt", "[ß] x", "[Amy]　全形空白", "[Amy]\x1fctl", "[A] x", "[A]  x",
]


def export_postprocess_strip_think() -> list[dict]:
    """`strip_think`：input 為文字。"""
    from src.translation.postprocess import strip_think
    texts = [
        "", "plain", "<think>x</think>y", "<THINK>x</THINK>\n\n y", "a<think>\nmulti\nline</think>   b",
        "<think>a</think><think>b</think>c", "<think>unclosed", "no open </think> tag",
        "<think></think>", "<Think>é</Think>\t　x", "x <think>😀</think> y",
        "<think>a</think>\x0b\x1c\x85z", "<thinK>x</think>k", "<think>a</THINK>b</think>c",
        "[Amy] <think>t</think>[Amy] 你好", "<think>a\r\nb</think>\r\nc", "<think\n>x</think>y",
        "<thınk>x</think>y", "<think>a</thinK>b",
    ]
    texts.append("<th" + chr(0x130) + "nk>x</th" + chr(0x130) + "nk>y")
    return [{"input": t, "output": strip_think(t)} for t in texts]


def export_postprocess_tidy() -> list[dict]:
    """`tidy_parentheses`：input 為 {source, translated}。"""
    from src.translation.postprocess import tidy_parentheses
    pairs = [
        ("[A] I got a Mystic Wood", "[A] 我拿到神秘木頭 (Mystic Wood)"),
        ("[A] 我拿到神秘木头", "[A] I got a mystic wood (Mystic Wood)"),
        ("[A] 木頭", "[A] Wood (木頭)"),
        ("[A] Malistaire", "[A] 瑪莉斯泰（Malistaire）"),
        ("[A] Malistaire", "[A] 瑪莉斯泰（Malistaire）（Malistaire）"),
        ("[A] Malistaire", "[A] Malistaire（Malistaire）"),
        ("[A] 好", "[A] 好（Siren）"),
        ("[A] 好", "[A] 好 (Siren) 好 (Siren)"),
        ("[A] 好", "[A] (好)"),
        ("[A] lvl 50 (50)", "[A] 等級 50 (50)"),
        ("[A] hi", "[A] 你好 :) (smile)"),
        ("[A] hi", "[A] 你好 (^_^)"),
        ("hi", "(Siren)"),
        ("hi", "（Siren）"),
        ("[A] hi", "[Amy (x)] 你好 (y)"),
        ("[A] café", "[A] 咖啡 (café)"),
        ("[A] cafe", "[A] 咖啡 (café)"),
        ("[A] Español", "[A] hola (ñandú) (木頭)"),
        ("[A] 木头", "[A] hola (木頭) amigo (木头)"),
        ("[A] ß", "[A] x (SS)"),
        ("[A] straße", "[A] 街 (STRASSE)"),
        ("[A] a-b", "[A] 甲 (A B)"),
        ("[A] 🐉", "[A] 龍 (🐉) (dragon)"),
        ("[A] x", "[A] 文字 (foo)"),
        ("[A] x", "[A] 日本語 (nihongo)\n(行)"),
        ("[A] ², x", "[A] 好 (²)"),
        ("[A] x", "[A] 好 (ée)"),
        ("", ""),
        ("[A] 日本語", "[A] 日本語（にほんご）"),
        ("[A] Привет", "[A] 你好 (Привет) (Hello)"),
        ("迷幻木头", "迷幻木頭(Mystic Wood)"),
        ("Proud Pegasus Statue", "驕傲的飛馬雕像(Proud Pegasus Statue)"),
        ("Talk to Bartleby at 天国大本营", "和 Bartleby 談談，地點在天國大本營（Heavenly HQ）"),
        ("[Amy] 迷幻木头", "[Amy] 迷幻木頭(Mystic Wood)"),
        ("迷幻木头", "迷幻木頭（Mystic Wood）"),
        ("你获得了 {0} 金币！", "你獲得了 {0} 金幣！"),
        ("熔岩百合", "熔岩百合（一種材料）"),
        ("lava lily", "溶岩ユリ（素材の一つ）"),
        ("你获得了迷幻木头和熔岩百合", "你獲得了迷幻木頭(Mystic Wood)和熔岩百合(Lava Lily)"),
        ("do i use sirens", "do I use Sirens (塞壬)"),
        ("你获得了:风暴", "You gained: Storm (风暴)"),
        ("hi", "hola (está aquí)"),
        ("Wizard" + chr(0x2019) + "s Hat", "巫師帽（Wizard's Hat）"),
        ("ship of fools", "愚者之船（Ship-of-Fools）"),
        ("[Amy] wts Fire Dragon (rank 5)", "[Amy] 出售火龍（Fire Dragon）（等級 5）"),
        ("[Amy] wts Fire Dragon (rank 5)", "[Amy] 出售火龍（Fire Dragon）(rank 5)"),
        ("[Amy] brb (dinner)", "[Amy] 等我一下（吃晚餐）"),
        ("[Amy] gg (╯°□°)╯", "[Amy] 好遊戲 (╯°□°)╯"),
        ("[Amy] hi (: ok :)", "[Amy] 嗨 (: ok :)"),
        ("Storm Lord (Rank 7)", "風暴領主（Storm Lord）(Rank 7)"),
        ("你获得了 鱼鳞 (x{0})", "你獲得了 魚鱗 (x{0})"),
        ("(Requires Level 45)", "（需要等級 45）"),
        ("(Requires Level 45)", "(Requires Level 45)"),
        ("farm Malistaire", "刷 Malistaire（Malistaire）"),
        ("help me in novus", "help me in Novus (Novus)"),
        ("[Amy] lvl 50 (50)", "[Amy] 等級 50（50）"),
    ]
    return [{"input": {"source": s, "translated": t}, "output": tidy_parentheses(s, t)}
            for s, t in pairs]


def export_postprocess_sender() -> list[dict]:
    """`sender_of`：input 為文字。"""
    from src.translation.postprocess import sender_of
    return [{"input": t, "output": sender_of(t)} for t in _SENDER_TEXTS]


def export_postprocess_restore() -> list[dict]:
    """`restore_sender`：input 為 {source, translated}。"""
    from src.translation.postprocess import restore_sender
    pairs = [
        ("[Amy] hello", "[艾米] 你好"), ("[Amy] hello", "你好"), ("[Amy] hello", "  你好"),
        ("[Amy] [WTS] gear", "[Amy] [WTS] 裝備"), ("[Amy] [WTS] gear", "[WTS] 裝備"),
        ("[Amy] [WTS] gear", "[X] [Y] 裝備"), ("plain", "[X] 你好"), ("[Amy] hi", ""),
        ("[Amy] hi", "[Amy]"), ("[贾斯廷 渡鸦] 木头", "[賈斯汀 渡鴉] 木頭"),
        ("[Amy] hi", "[B]　你好"), ("[A][B] hi", "[C] 你好"),
        ("[A] [B] hi", "[C][D] 你好"), ("[A] hi", "[" + "x" * 41 + "] 你好"),
        ("[A] hi", "[x\ny] 你好"), ("[A] 😀", "[B] 😀"), ("[A] hi", "\x0b[B] 你好"),
        ("[A]   hi", "[B]\x1c你好"), ("[ß] hi", "[ss] 你好"),
        ("[贾斯廷 渡鸦] hi", "[賈斯汀 渡鴉] 嗨"), ("[Amy] hi", "嗨"),
        ("[Amy] [WTS] gear", "[出售] 裝備"), ("[Amy] [WTS] gear", "[艾米] [出售] 裝備"),
        ("hi", "嗨"),
    ]
    return [{"input": {"source": s, "translated": t}, "output": restore_sender(s, t)}
            for s, t in pairs]


def export_postprocess_latin() -> list[dict]:
    """`uses_latin_script`：input 為語言名稱。"""
    from src.translation.postprocess import uses_latin_script
    langs = ["English", "Español", "日本語", "繁體中文 (台灣)", "한국어", "Русский", "", "ไทย",
             "Français", "العربية", "中文 (Chinese)", "Ελληνικά", "ａｂｃ", "ß", "Türkçe", "K"]
    return [{"input": lang, "output": uses_latin_script(lang)} for lang in langs]


def export_postprocess_stray() -> list[dict]:
    """`has_stray_latin`：input 為 {source, translated, target}。"""
    from src.translation.postprocess import has_stray_latin
    zh = "繁體中文 (台灣)"
    cases = [
        ("[A] 雪刺帽", "[A] Snowspike Hat", zh), ("[A] 雪刺帽", "[A] 雪刺帽", zh),
        ("[A] 收到 [Amy] 的邀請", "[A] 收到 Amy 的邀請", zh),
        ("[A] Received a friend request from Amy", "[A] Received a friend request from Amy", zh),
        ("[A] 收到 Amy 的邀請", "[A] 收到 Amy 的邀請 Calamity", zh),
        ("[A] hi", "[A] 你好 Hi", zh), ("[A] hi", "[A] 你好 hi", zh),
        ("[A] HI  there", "[A] 你好 hi there", zh), ("[A] x", "[A] 你好", zh),
        ("[A] x", "[A] 1234", zh), ("[A] x", "[A] ...", zh), ("[A] x", "[A] x y", zh),
        ("[A] Straße", "[A] 街 STRASSE", zh), ("[A] ß", "[A] 好 SS", zh),
        ("[A] x", "[A] café", zh), ("[A] x", "[A] 你好 café", zh),
        ("[A] x", "[A] Snowspike Hat", "English"), ("[A] x", "[A] Snowspike", "Español"),
        ("[A] 木頭", "[A] 木頭 wood", "日本語"), ("[A] wood-x", "[A] 木 wood-x", "日本語"),
        ("[A] it's", "[A] 好 it’s", zh), ("[A] a.b, c", "[A] 好 a.b, c", zh),
        ("", "", zh), ("[A] ", "[A] ABC", zh), ("[A] x y", "[A] 好 x y", zh),
        ("[A] x　y", "[A] 好 x y", zh), ("[A] 😀", "[A] 😀 ok", zh),
        ("[A] x", "[A] 好 ok²", zh), ("[A] x", "[A] ² A", zh),
        ("[A] x", "[A] ① A", zh), ("[A] x\x1fy", "[A] 好 x y", zh),
        ("雪刺帽", "Snowspike Hat", "繁體中文（台灣）"),
        ("卡拉米蒂 现在等级 {0}！", "Calamity 現在等級 {0}！", "繁體中文（台灣）"),
        ("雪刺帽", "雪刺帽", "繁體中文（台灣）"),
        ("death skeleturion", "死亡骷髏戰士 (Death Skeleturion)", "繁體中文（台灣）"),
        ("雪刺帽", "Snowspike Hat", "English"), ("雪刺帽", "Sombrero de Nieve", "Español"),
        ("[Amy] 雪刺帽", "[Amy] 雪刺帽", "繁體中文（台灣）"),
        ("[艾米] 雪刺帽", "[艾米] Snowspike Hat", "繁體中文（台灣）"),
        ("You received Snowspike Hat", "You received Snowspike Hat", "繁體中文（台灣）"),
        ("You received Snowspike Hat", "你獲得了 Snowspike Hat", "繁體中文（台灣）"),
        ("收到 [Amy] 的伙伴邀请", "Received a friend request from Amy", "繁體中文（台灣）"),
        ("收到 [Amy] 的伙伴邀请", "收到 Amy 的夥伴邀請", "繁體中文（台灣）"),
        ("你获得了 Lava Lily", "你獲得了  lava lily", "繁體中文（台灣）"),
        ("收到 [Amy] 的伙伴邀请", "收到 Snowspike Hat 的夥伴邀請", "繁體中文（台灣）"),
        ("雪刺帽", "Snowspike Hat", "日本語"), ("雪刺帽", "Snowspike Hat", "한국어"),
        ("雪刺帽", "Snowspike Hat", "Русский"), ("雪刺帽", "Snowspike Hat", "ภาษาไทย"),
        ("雪刺帽", "Snowspike Hat", "Ελληνικά"),
        ("你获得了 {0} 金币！", "{0} ゴールドを手に入れた！", "日本語"),
    ]
    return [{"input": {"source": s, "translated": t, "target": g},
             "output": has_stray_latin(s, t, g)} for s, t, g in cases]


_LINE_TEXTS = [
    "", "a", "a\nb", "a\n\nb", "  a  \n  b  ", "a\r\nb\rc", "a\x0bb\x0cc", "a\x1cb\x1dc\x1ed",
    "a\x85b", "a b c", "a  \n 　 \nb", "\n\n", "😀\né", "  \t ",
    "1. a\n2. b", "1. a\r\n2．b\n3、c\n4) d", "x y ", "a\x0b\n\x0cb", "﻿a\nb",
    "ß\n x ",
]


def export_postprocess_lines() -> list[dict]:
    """`nonblank_lines`／`number_lines`：input 為文字。"""
    from src.translation.postprocess import nonblank_lines, number_lines
    cases = []
    for t in [*_LINE_TEXTS, "a\n\n b \n"]:
        cases.append({"input": {"fn": "nonblank_lines", "text": t}, "output": nonblank_lines(t)})
        lines, numbered = number_lines(t)
        cases.append({"input": {"fn": "number_lines", "text": t}, "output": [lines, numbered]})
    return cases


_NUM_OUTPUTS = [
    "", "1. a\n2. b", "1.a\n2．b\n3、c\n4)d", "  2 . x\n1 、 y", "1. a\n1. b", "01. a\n002) b",
    "no numbers\nat all", "1. a\nplain\n3. c", "１．全形\n２、二", "１２. x", "٣. arabic\n١. one",
    "1. \n2. b", "1.\n2.", "a\n1. b", "1. a  2. b", "1. a\r\n2. b\r3. c",
    "99999999999999999999999. big\n1. a", "1. 😀\n2. é", "𝟏. math\n𝟐. two",
    "1) a ) b", "1。 a", " 1. nbsp", "1. x",
]


def export_postprocess_numbering() -> list[dict]:
    """`numbered_entries`／`unnumber_lines`。"""
    from src.translation.postprocess import numbered_entries, unnumber_lines
    cases = []
    extra = ["1) 甲\n２．乙\n3、丙", "甲\n乙", "一整段", "以下是翻譯：\n1. 甲\n\n3. 丙"]
    for out in [*_NUM_OUTPUTS, *extra]:
        entries = numbered_entries(out)
        cases.append({"input": {"fn": "numbered_entries", "output": out},
                      "output": [[str(k), v] for k, v in entries.items()]})
        for originals in (["o1", "o2"], ["o1", "o2", "o3"], ["only"], []):
            cases.append({"input": {"fn": "unnumber_lines", "output": out, "originals": originals},
                          "output": unnumber_lines(out, originals)})
    return cases


EXPORTERS["prompts_systems"] = export_prompts_systems
EXPORTERS["prompts_turns"] = export_prompts_turns
EXPORTERS["postprocess_strip_think"] = export_postprocess_strip_think
EXPORTERS["postprocess_tidy"] = export_postprocess_tidy
EXPORTERS["postprocess_sender"] = export_postprocess_sender
EXPORTERS["postprocess_restore"] = export_postprocess_restore
EXPORTERS["postprocess_latin"] = export_postprocess_latin
EXPORTERS["postprocess_stray"] = export_postprocess_stray
EXPORTERS["postprocess_lines"] = export_postprocess_lines
EXPORTERS["postprocess_numbering"] = export_postprocess_numbering


def export_clients_error_detail() -> list[dict]:
    """`error_detail`：input 為錯誤回應 body，output 為單行說明。"""
    url = "https://platform.openai.com/account/api-keys"
    texts = [
        '{"error": {"message": "Incorrect API key provided: sk-abc***"}}',
        '{"error": "model not found"}',
        '{"message": "quota exceeded"}',
        '{"error": {"code": 42}}',
        '{"error": {"message": 5}}',
        '{"error": {"message": "inner"}, "message": "outer"}',
        '{"error": 7, "message": "fallback"}',
        '{\n  "error": {\n    "message": "multi\nline   text"\n  }\n}',
        "[1, 2]",
        "42",
        "null",
        "  plain text \n body ",
        "<html><head><title>404 Not Found</title></head><body><h1>404 Not Found</h1>"
        "<hr><center>nginx</center></body></html>",
        "line one\r\nline two\n\n\tline three",
        "word " * 60 + "\n  " + url + " tail",
        "中文  錯誤　訊息",
        "",
    ]
    return [{"input": t, "output": error_detail(t)} for t in texts]


def export_clients_rejected_parameter() -> list[dict]:
    """`rejected_parameter`：input 為 400 說明文字，output 為參數名或 null。"""
    texts = [
        "Unrecognized request argument supplied: reasoning_effort",
        "Unsupported parameter: 'max_tokens' is not supported with this model.",
        "Unsupported value: 'temperature' does not support 0 with this model.",
        "Unsupported parameter: temperature",
        "Unsupported parameter: 'max_completion_tokens' is not supported.",
        "bad request",
        "",
        "prefix Unrecognized request argument supplied: stream_options suffix",
    ]
    return [{"input": t, "output": rejected_parameter(t)} for t in texts]


def export_clients_openai_body() -> list[dict]:
    """`_OpenAICompatClient._body`：input 為組合參數，output 為請求 JSON。"""
    turns = [{"role": "user", "content": "[A] hi"}, {"role": "assistant", "content": "譯文"}]
    cases = []
    for official in (False, True):
        for thinking in (False, True):
            for deterministic in (False, True):
                for max_tokens in (512, 4096):
                    client = _OpenAICompatClient(base_url="http://x", model="m",
                                                 thinking=thinking, official=official,
                                                 client=object())
                    cases.append({
                        "input": {"official": official, "thinking": thinking,
                                  "deterministic": deterministic, "max_tokens": max_tokens,
                                  "system": "sys", "turns": turns},
                        "output": client._body("sys", turns, max_tokens, deterministic),
                    })
    return cases


EXPORTERS["clients_error_detail"] = export_clients_error_detail
EXPORTERS["clients_rejected_parameter"] = export_clients_rejected_parameter
EXPORTERS["clients_openai_body"] = export_clients_openai_body


def _pair_json(pair) -> list[str] | None:
    return None if pair is None else list(pair)


def _set_json(examples) -> dict:
    return {name: _pair_json(getattr(examples, name))
            for name in ("incoming", "system", "region")}


def _set_from_json(data: dict):
    from src.translation.examples import ExampleSet
    return ExampleSet(*(None if data[name] is None else tuple(data[name])
                        for name in ("incoming", "system", "region")))


def _repr_inputs() -> list[str]:
    import random

    pool = ["a", "Z", " ", "'", '"', "\\", "\t", "\n", "\r", "\x00", "\x07", "\x1b", "\x7f",
            "\x80", "\x85", "\x9f", "\xa0", "\xad", "é", "ß", "中", "あ", "한", "😀", "𝒳",
            "\U0001f1fa", "\u2028", "\u2029", "\u200b", "\u200d", "\ufeff", "\u2003",
            "\u3000", "\ue000", "\U000f0000", "\U0010ffff", "\u0378", "\U0001fffe", "\u061c",
            "\u180e", "\u2060", "\u00ad", "\ufffe", "\ud7ff", "̀"]
    fixed = ["", "'", '"', "'\"", "it's", 'say "hi"', "both ' and \"", "back\\slash",
             "\'", "tab\there", "line\nbreak", "cr\rlf\r\n", "\x00\x01\x1f", "del\x7f",
             "nbsp\xa0here", "zero\u200bwidth", "bom\ufeff", "ls\u2028ps\u2029", "emoji 😀",
             "非BMP 𝒳", "[Amy] 火貓（Fire Cat）", "Kai {0} gold", "unassigned \u0378",
             "private \ue000", "noncharacter \U0001fffe", "plain ascii"]
    rng = random.Random(20261017)
    cases = list(fixed)
    while len(cases) < 200:
        cases.append("".join(rng.choice(pool) for _ in range(rng.randint(1, 8))))
    return cases


def export_pyrepr() -> list[dict]:
    """`repr(str)`：input 為字串，output 為 Python 的 repr。"""
    return [{"input": s, "output": repr(s)} for s in _repr_inputs()]


def _example_sets() -> list:
    from src.translation.examples import EMPTY, GAME_LANGUAGE_EXAMPLES, ExampleSet
    return [
        EMPTY,
        GAME_LANGUAGE_EXAMPLES,
        ExampleSet(("s", "o"), None, None),
        ExampleSet(None, ("it's", 'say "hi"'), None),
        ExampleSet(None, None, ("a\nb", "c\\d\te")),
        ExampleSet(("both ' and \"", "\u200b\ufeff"), ("😀", "\x7f"), ("\u2028", "\u0378")),
        ExampleSet(("[Amy] 火貓", "[Amy] Fire Cat (火貓)"), ("", ""), ("x", "y")),
    ]


def export_examples_digest() -> list[dict]:
    """`ExampleSet.digest()`：input 為三條路徑，output 為 12 碼 sha1。"""
    return [{"input": _set_json(e), "output": {"digest": e.digest(), "repr": repr(e)}}
            for e in _example_sets()]


def export_examples_parse() -> list[dict]:
    """`parse_generated`：input 為模型輸出，output 為（範例集, 失敗說明）。"""
    from src.translation.examples import parse_generated
    good = "\n".join([
        "1. [Amy] 知らないよ、Kai と新しい鎧を手に入れて、"
        "巨像大道（Colossus Boulevard）で火猫（Fire Cat）を覚えた、笑",
        "2. Kai が火猫（Fire Cat）を教えてくれた！巨像大道（Colossus Boulevard）で {0} ゴールドを獲得した。",
        "3. 火猫（Fire Cat）に話しかける",
        "4. 巨像大道（Colossus Boulevard）へ行く",
        "5. オプション",
        "6. そしてあなたは",
    ])
    fr = "\n".join([
        "1. [Amy] Je sais pas, Kai et moi on a eu de nouvelles armures et appris Chat de Feu "
        "(Fire Cat) au boulevard du Colosse (Colossus Boulevard), lol",
        "2. Kai t'a appris Chat de Feu (Fire Cat) ! Tu as gagné {0} or au boulevard du Colosse "
        "(Colossus Boulevard).",
        "3. Parle au Chat de Feu (Fire Cat)",
        "4. Va au boulevard du Colosse (Colossus Boulevard)",
        "5. OPTIONS",
        "6. et ensuite tu dois",
    ])
    lines = good.splitlines()
    shifted = "\n".join(f"{i + 1}.{line[2:]}" for i, line in enumerate(lines, 1))
    outputs = [
        "", good, fr, "\n".join(lines[:3]), good.replace("{0} ゴールド", "ゴールド"),
        good.replace("火猫（Fire Cat）を覚えた", "火猫を覚えた"),
        good.replace("3. 火猫（Fire Cat）に話しかける", "3. 火猫に話しかける"),
        fr.replace("Chat de Feu (Fire Cat)", "Fire Cat (Fire Cat)"),
        "\n".join(line[3:] for line in lines[:3]),
        good.replace("6. そしてあなたは", "6. and then you must"),
        good.replace("5. オプション", "5. OPTIONS"),
        "\n".join(line for line in lines if not line.startswith("5.")),
        good.replace("5. オプション", "5. オプション（OPTIONS）"),
        "\n".join(line for line in lines if not line.startswith("3.")),
        "1. 以下是翻譯：\n" + shifted,
        "以下是翻譯：\n```\n" + good + "\n```",
        "\n".join(line[3:] for line in lines),
        good.replace("4. 巨像大道（Colossus Boulevard）へ行く", "4. "),
        good.replace("（", "(").replace("）", ")"),
        good.replace("（Fire Cat）", "（ Fire Cat ）"),
        good.replace("{0}", "{0} {0}"),
        good.replace("\n", "\r\n"),
        good.replace("\n", "\u2028"),
        good.replace("Kai が", "が"),
    ]
    return [{"input": o, "output": {**_set_json(ex), "failures": failures}}
            for o in outputs for ex, failures in [parse_generated(o)]]


def export_examples_store_bytes() -> list[dict]:
    """`ExampleStore.put` 寫出的檔案：input 為依序的 put，output 為檔案內容。"""
    import contextlib
    import io
    import tempfile

    from src.translation.examples import ExampleStore
    sets = _example_sets()
    scenarios = [
        [("fp-a", sets[2])],
        [("fp-a", sets[2]), ("fp-b", sets[5]), ("fp-a", sets[4])],
        [("empty", sets[0])],
        [("日本語|model|e4", sets[6]), ("quote\"'\\\n\t\x1f", sets[3]), ("\u2028\x7f😀", sets[1])],
        [(f"fp-{i}", sets[2 + i % 5]) for i in range(35)],
    ]
    cases = []
    for puts in scenarios:
        with tempfile.TemporaryDirectory() as tmp, contextlib.redirect_stderr(io.StringIO()):
            path = Path(tmp) / "translation-examples.json"
            store = ExampleStore(path)
            for fingerprint, examples in puts:
                store.put(fingerprint, examples)
            text = path.read_bytes().decode("utf-8") if path.exists() else None
        cases.append({"input": [{"fingerprint": fp, "examples": _set_json(e)} for fp, e in puts],
                      "output": text})
    return cases


def export_examples_game_language() -> list[dict]:
    """`GAME_LANGUAGE_EXAMPLES`：output 為三條路徑、repr 與 digest（進使用者的快取指紋）。"""
    from src.translation.examples import GAME_LANGUAGE_EXAMPLES as g
    return [{"input": "GAME_LANGUAGE_EXAMPLES",
             "output": {**_set_json(g), "repr": repr(g), "digest": g.digest()}}]


def export_examples_generation_request() -> list[dict]:
    """`generation_request`：input 為目標語言，output 為（system 提示詞, 對話輪）。"""
    from src.translation.examples import generation_request
    return [{"input": t, "output": {"system": system, "turns": turns}}
            for t in ("日本語", "Deutsch", " Español ", "繁體中文（台灣）", "English")
            for system, turns in [generation_request(t)]]


EXPORTERS["pyrepr"] = export_pyrepr
EXPORTERS["examples_digest"] = export_examples_digest
EXPORTERS["examples_parse"] = export_examples_parse
EXPORTERS["examples_store_bytes"] = export_examples_store_bytes
EXPORTERS["examples_game_language"] = export_examples_game_language
EXPORTERS["examples_generation_request"] = export_examples_generation_request


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
