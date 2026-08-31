"""更新檢查：版本解析比較（純函式）與 GitHub API 取用（假 client）。"""
from src.updater import is_newer, parse_version


def test_parse_version_accepts_plain_and_v_prefixed():
    assert parse_version("0.1.0") == (0, 1, 0)
    assert parse_version("v0.2.3") == (0, 2, 3)
    assert parse_version(" v1.20.300 ") == (1, 20, 300)


def test_parse_version_ignores_suffixes():
    # /releases/latest 已排除 pre-release，但 tag 本身仍可能帶後綴
    assert parse_version("v0.2.0-beta.1") == (0, 2, 0)
    assert parse_version("0.2.0+build.5") == (0, 2, 0)


def test_parse_version_rejects_unparsable_text():
    for text in ("", "latest", "v1.2", "1.2.x", "release-2026"):
        assert parse_version(text) is None


def test_is_newer_compares_numerically():
    assert is_newer("0.2.0", "0.1.0") is True
    assert is_newer("v0.10.0", "0.9.9") is True      # 字串比大小會判錯的例子
    assert is_newer("0.1.0", "0.1.0") is False
    assert is_newer("0.1.0", "0.2.0") is False


def test_is_newer_is_false_when_either_side_is_unparsable():
    # 寧可漏提醒也不要誤報：橫幅會把使用者導去下載頁
    assert is_newer("nightly", "0.1.0") is False
    assert is_newer("0.2.0", "unknown") is False
