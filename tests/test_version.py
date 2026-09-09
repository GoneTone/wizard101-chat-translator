import re
import tomllib
from pathlib import Path

import src

ROOT = Path(__file__).resolve().parents[1]
PYPROJECT = ROOT / "pyproject.toml"
RELEASE_WORKFLOW = ROOT / ".github" / "workflows" / "release-windows.yml"

# 允許預發布後綴（0.2.0-rc.1）：放版 workflow 收這種 tag，見下面那個測試。
SEMVER_RE = re.compile(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?")


def test_version_is_semver():
    assert SEMVER_RE.fullmatch(src.__version__), src.__version__


def test_release_workflow_tags_are_accepted_here():
    """workflow 收得下的 tag，這裡也必須收得下。

    放版流程是「把 tag 的版本寫進 `src/__init__.py` → 跑測試」，兩邊的格式一漂移，
    預發布放版就會在測試步驟才爆掉，而版本號 commit 已經 push 出去了。"""
    tag_pattern = re.search(r"notmatch '(\^v.+\$)'",
                            RELEASE_WORKFLOW.read_text(encoding="utf-8")).group(1)
    tag_re = re.compile(tag_pattern)
    for tag in ("v0.1.0", "v0.2.0-rc.1", "v10.20.30-beta.2"):
        assert tag_re.fullmatch(tag), f"workflow 不收 {tag}"
        assert SEMVER_RE.fullmatch(tag[1:]), f"tag {tag} 寫進 __version__ 後會被本檔擋下"


def test_pyproject_version_matches_package():
    # pyproject 的 version 純屬中繼資料（package = false，執行期讀不到），
    # 執行期一律用 src.__version__；這裡把兩者釘在一起，避免放版時只改一邊。
    meta = tomllib.loads(PYPROJECT.read_text(encoding="utf-8"))
    assert meta["project"]["version"] == src.__version__
