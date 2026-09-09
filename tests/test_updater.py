"""更新檢查：版本解析比較（純函式）與 GitHub API 取用（假 client）。"""
import httpx
import pytest

from src.updater import (
    LATEST_API,
    RELEASES_URL,
    Release,
    UpdateCheckError,
    check_for_update,
    fetch_latest_release,
    is_newer,
    parse_version,
)


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


def test_is_newer_ranks_a_release_above_its_own_prereleases():
    # SemVer：0.2.0-rc.1 < 0.2.0。跑 rc 版的人要收得到正式版釋出的提醒
    assert is_newer("0.2.0", "0.2.0-rc.1") is True
    assert is_newer("0.2.0-rc.1", "0.2.0") is False


def test_is_newer_compares_numeric_prerelease_identifiers_as_numbers():
    assert is_newer("0.2.0-rc.2", "0.2.0-rc.1") is True
    assert is_newer("0.2.0-rc.10", "0.2.0-rc.9") is True   # 字典序會判錯的例子


def test_is_newer_ranks_alphanumeric_prerelease_identifiers_above_numeric_ones():
    # SemVer：數字識別碼低於字母識別碼；前綴相同時欄位多的較新
    assert is_newer("0.2.0-alpha.beta", "0.2.0-alpha.1") is True
    assert is_newer("0.2.0-beta", "0.2.0-alpha") is True
    assert is_newer("0.2.0-rc.1", "0.2.0-rc") is True


def test_build_metadata_does_not_affect_precedence():
    # SemVer 明定 build metadata 不參與優先序比較
    assert is_newer("0.2.0+build.2", "0.2.0+build.1") is False
    assert is_newer("0.2.0-rc.1+build.2", "0.2.0-rc.1") is False


def test_is_newer_is_false_when_either_side_is_unparsable():
    # 寧可漏提醒也不要誤報：橫幅會把使用者導去下載頁
    assert is_newer("nightly", "0.1.0") is False
    assert is_newer("0.2.0", "unknown") is False


_TAG_URL = "https://github.com/GoneTone/wizard101-chat-translator/releases/tag/v0.2.0"


class FakeResponse:
    def __init__(self, status_code=200, payload=None):
        self.status_code = status_code
        self._payload = {"tag_name": "v0.2.0", "html_url": _TAG_URL} \
            if payload is None else payload

    def json(self):
        if self._payload is _BROKEN_JSON:
            raise ValueError("not json")
        return self._payload


_BROKEN_JSON = object()


class FakeClient:
    """只認 get 的假 httpx client；記下被打的網址與標頭供斷言。"""

    def __init__(self, response=None, raises=None):
        self._response = response if response is not None else FakeResponse()
        self._raises = raises
        self.calls = []

    def get(self, url, headers=None):
        self.calls.append((url, headers))
        if self._raises is not None:
            raise self._raises
        return self._response


def test_fetch_latest_release_reads_tag_and_url():
    client = FakeClient()
    release = fetch_latest_release(client=client)
    assert release == Release(version="0.2.0", url=_TAG_URL)
    url, headers = client.calls[0]
    assert url == LATEST_API
    assert headers["Accept"] == "application/vnd.github+json"
    assert "wizard101-chat-translator" in headers["User-Agent"]


def test_fetch_latest_release_returns_none_when_no_release_exists():
    # 404＝尚未發過任何 release（也涵蓋 repo 尚未公開）：不是錯誤
    assert fetch_latest_release(client=FakeClient(FakeResponse(status_code=404))) is None


@pytest.mark.parametrize("status", [403, 429, 500])
def test_fetch_latest_release_raises_on_other_status_codes(status):
    with pytest.raises(UpdateCheckError) as exc:
        fetch_latest_release(client=FakeClient(FakeResponse(status_code=status)))
    assert str(status) in str(exc.value)


def test_fetch_latest_release_raises_when_connection_fails():
    client = FakeClient(raises=httpx.ConnectError("no route to host"))
    with pytest.raises(UpdateCheckError):
        fetch_latest_release(client=client)


def test_fetch_latest_release_raises_on_unusable_payload():
    with pytest.raises(UpdateCheckError):
        fetch_latest_release(client=FakeClient(FakeResponse(payload={})))
    with pytest.raises(UpdateCheckError):
        fetch_latest_release(client=FakeClient(FakeResponse(payload=_BROKEN_JSON)))


def test_fetch_latest_release_falls_back_to_releases_page_without_html_url():
    client = FakeClient(FakeResponse(payload={"tag_name": "v0.3.0"}))
    assert fetch_latest_release(client=client) == Release(version="0.3.0",
                                                          url=RELEASES_URL)


def test_check_for_update_returns_release_only_when_newer():
    client = FakeClient()
    assert check_for_update(current="0.1.0", client=client).version == "0.2.0"
    assert check_for_update(current="0.2.0", client=client) is None
    assert check_for_update(current="9.9.9", client=client) is None


def test_check_for_update_returns_none_when_no_release_exists():
    client = FakeClient(FakeResponse(status_code=404))
    assert check_for_update(current="0.1.0", client=client) is None


def test_announce_update_queues_the_banner_when_newer_exists():
    import queue as queue_module

    from src.main import announce_update

    class FakeOverlay:
        def __init__(self):
            self.shown = []

        def set_update(self, release):
            self.shown.append(release)

    release = Release(version="0.2.0", url=_TAG_URL)
    ui_queue = queue_module.Queue()
    overlay = FakeOverlay()

    announce_update(ui_queue, overlay, checker=lambda: release)

    # 背景執行緒只把回呼排進 ui_queue，由主執行緒取出後才碰 tkinter
    assert overlay.shown == []
    ui_queue.get_nowait()()
    assert overlay.shown == [release]


def test_announce_update_stays_quiet_without_a_newer_release():
    import queue as queue_module

    from src.main import announce_update

    ui_queue = queue_module.Queue()
    announce_update(ui_queue, object(), checker=lambda: None)
    assert ui_queue.empty()


def test_announce_update_swallows_check_failures():
    # 檢查更新失敗絕不能影響啟動與收訊
    import queue as queue_module

    from src.main import announce_update

    def boom():
        raise UpdateCheckError("HTTP 403")

    ui_queue = queue_module.Queue()
    announce_update(ui_queue, object(), checker=boom)
    assert ui_queue.empty()
