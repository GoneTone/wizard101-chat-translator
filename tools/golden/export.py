"""把 Python 行為匯出成 golden fixture，供 Rust 測試逐筆對照。

每個 exporter 回傳 `[{"input": ..., "output": ...}, ...]`，寫到
`rust/tests/golden/<name>.json`；`--check` 只比對不寫入。
"""
import argparse
import json
import sys
from collections.abc import Callable
from datetime import UTC, datetime, timedelta, timezone
from pathlib import Path

from src.logs.files import session_header, trim_log_sessions, utc_stamp

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
