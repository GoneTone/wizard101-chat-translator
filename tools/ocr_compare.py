"""OCR 新舊對照：Python 版（baseline）與 Rust 版各自辨識同一批圖，再產生並排報告。

用法：`python -m tools.ocr_compare baseline|report <dir>`；Rust 側見 `rust/examples/ocr_compare.rs`。
"""
import json
import sys
from pathlib import Path

from src.region.ocr import recognize


def write_baseline(folder: Path) -> Path:
    """辨識 `folder/*.png`，寫出 `python.json`（{檔名: 文字}）。"""
    results = {}
    for p in sorted(folder.glob("*.png")):
        try:
            results[p.name] = recognize(p.read_bytes())
        except Exception as exc:
            print(f"failed: {p.name}: {exc}", file=sys.stderr)
            results[p.name] = f"(error: {type(exc).__name__}: {exc})"
    dest = folder / "python.json"
    dest.write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
    return dest


_MISSING = "(missing)"


def _load(path: Path, hint: str) -> dict:
    if not path.is_file():
        sys.exit(f"{path} not found: {hint}")
    return json.loads(path.read_text(encoding="utf-8"))


def write_report(folder: Path) -> Path:
    """讀 `python.json` 與 `rust.json`，寫出 `report.md`（相同標 `=`，不同標 `⚠`）。"""
    python, rust = (_load(folder / name, hint) for name, hint in (
        ("python.json", "run `baseline` first"), ("rust.json", "run the Rust example first")))
    lines = ["# OCR 新舊對照", ""]
    same = 0
    names = sorted(python.keys() | rust.keys())
    for name in names:
        old, new = python.get(name, _MISSING), rust.get(name, _MISSING)
        if old == new:
            same += 1
            lines += [f"= {name}", "", "```", old, "```", ""]
        else:
            lines += [f"⚠ {name}", "", "Python：", "```", old, "```", "Rust：", "```", new, "```", ""]
    total = len(names)
    lines.append(f"共 {total} 張，相同 {same} 張，不同 {total - same} 張")
    dest = folder / "report.md"
    dest.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return dest


def main(argv: list[str]) -> None:
    commands = {"baseline": write_baseline, "report": write_report}
    if len(argv) != 2 or argv[0] not in commands:
        sys.exit("usage: python -m tools.ocr_compare baseline|report <dir>")
    print(commands[argv[0]](Path(argv[1])))


if __name__ == "__main__":
    main(sys.argv[1:])
