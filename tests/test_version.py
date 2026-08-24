import re
import tomllib
from pathlib import Path

import src

PYPROJECT = Path(__file__).resolve().parents[1] / "pyproject.toml"


def test_version_is_semver():
    assert re.fullmatch(r"\d+\.\d+\.\d+", src.__version__), src.__version__


def test_pyproject_version_matches_package():
    # pyproject 的 version 純屬中繼資料（package = false，執行期讀不到），
    # 執行期一律用 src.__version__；這裡把兩者釘在一起，避免放版時只改一邊。
    meta = tomllib.loads(PYPROJECT.read_text(encoding="utf-8"))
    assert meta["project"]["version"] == src.__version__
