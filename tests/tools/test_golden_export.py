"""golden fixture 匯出工具的測試。"""
from tools.golden import export


def test_check_detects_drift(monkeypatch, tmp_path):
    """--check 在 fixture 與重新產生的內容不同時回傳非零。"""
    monkeypatch.setitem(export.EXPORTERS, "fake", lambda: [{"input": 1, "output": 2}])

    assert export.main(["fake"], out_dir=tmp_path) == 0
    assert export.main(["--check", "fake"], out_dir=tmp_path) == 0

    (tmp_path / "fake.json").write_text("[]", encoding="utf-8", newline="\n")
    assert export.main(["--check", "fake"], out_dir=tmp_path) != 0


def test_unknown_exporter_exits_2_and_writes_nothing(tmp_path, capsys):
    """未知的 exporter 名稱回傳 2、說明寫到 stderr，且不產生任何檔案。"""
    assert export.main(["no_such_exporter"], out_dir=tmp_path) == 2
    assert "unknown exporter: no_such_exporter" in capsys.readouterr().err
    assert list(tmp_path.iterdir()) == []
