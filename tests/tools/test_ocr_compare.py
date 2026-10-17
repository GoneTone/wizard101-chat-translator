"""OCR 新舊對照：report 要逐張標出差異並統計張數。"""
import json

import pytest

from tools.ocr_compare import write_report


def test_report_marks_differences_and_counts(tmp_path):
    (tmp_path / "python.json").write_text(
        json.dumps({"a.png": "Items", "b.png": "It ems"}), encoding="utf-8")
    (tmp_path / "rust.json").write_text(
        json.dumps({"a.png": "Items", "b.png": "Items"}), encoding="utf-8")

    report = write_report(tmp_path).read_text(encoding="utf-8")

    assert "= a.png" in report
    assert "⚠ b.png" in report
    assert "It ems" in report
    assert "共 2 張，相同 1 張，不同 1 張" in report


def test_report_covers_union_and_marks_missing(tmp_path):
    (tmp_path / "python.json").write_text(json.dumps({"a.png": "x"}), encoding="utf-8")
    (tmp_path / "rust.json").write_text(json.dumps({"b.png": "y"}), encoding="utf-8")

    report = write_report(tmp_path).read_text(encoding="utf-8")

    assert "⚠ a.png" in report and "⚠ b.png" in report
    assert "(missing)" in report
    assert "共 2 張，相同 0 張，不同 2 張" in report


def test_report_exits_with_hint_when_json_missing(tmp_path):
    (tmp_path / "rust.json").write_text("{}", encoding="utf-8")
    with pytest.raises(SystemExit, match="baseline"):
        write_report(tmp_path)
