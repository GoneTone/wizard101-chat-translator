"""把 Python 行為匯出成 golden fixture，供 Rust 測試逐筆對照。

每個 exporter 回傳 `[{"input": ..., "output": ...}, ...]`，寫到
`rust/tests/golden/<name>.json`；`--check` 只比對不寫入。
"""
import argparse
import json
import sys
from collections.abc import Callable
from pathlib import Path

GOLDEN_DIR = Path(__file__).resolve().parents[2] / "rust" / "tests" / "golden"

EXPORTERS: dict[str, Callable[[], list[dict]]] = {}


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
