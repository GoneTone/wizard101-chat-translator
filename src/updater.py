"""更新檢查：回答「GitHub 上有沒有比目前這版更新的 release」。

只負責查詢與比較，不碰 UI、不碰 config，也不下載或安裝任何東西 —— 要不要提醒、
怎麼提醒由呼叫端決定（啟動路徑走 overlay 橫幅，設定視窗走「關於」分頁）。
"""
import re
from dataclasses import dataclass

import httpx

from src import __version__
from src.log import log

GITHUB_REPO = "GoneTone/wizard101-chat-translator"
PROJECT_URL = f"https://github.com/{GITHUB_REPO}"
RELEASES_URL = f"{PROJECT_URL}/releases/latest"
ISSUES_URL = f"{PROJECT_URL}/issues"
AUTHOR_URL = "https://github.com/GoneTone"
CROWDIN_URL = "https://crowdin.com/project/wizard101-chat-translator"
LATEST_API = f"https://api.github.com/repos/{GITHUB_REPO}/releases/latest"

_TIMEOUT = 10.0   # 啟動路徑不等它，但也不能讓手動檢查的按鈕卡住不放
# GitHub API 要求帶 User-Agent，缺了會被拒（403）
_HEADERS = {"Accept": "application/vnd.github+json",
            "User-Agent": f"wizard101-chat-translator/{__version__}"}

# 只取前三段數字，後綴（-beta.1、+build）一律忽略：/releases/latest 已排除
# pre-release，這裡容忍後綴只是為了不因為 tag 寫法而整個解析失敗。
_VERSION_RE = re.compile(r"^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?")
_RELEASE = (1,)   # 沒有預發布後綴＝正式版，依 SemVer 排在同版號的預發布之後


def _precedence_key(text: str) -> tuple | None:
    """SemVer 優先序的可比較鍵 `(三段版號, 預發布鍵)`；解析不出來回 None。

    build metadata（`+build.5`）依 SemVer 不參與比較，正則不收它即可忽略。"""
    match = _VERSION_RE.match(text.strip()) if text else None
    if match is None:
        return None
    major, minor, patch, prerelease = match.groups()
    numbers = (int(major), int(minor), int(patch))
    if prerelease is None:
        return numbers, _RELEASE
    # 數字識別碼比數值、字母識別碼比 ASCII，且數字一律低於字母 —— 由第一格的 0／1
    # 表達，後兩格才是同類之間的比較值。前綴相同時欄位多者較新由 tuple 比較自然成立。
    identifiers = tuple((0, int(part), "") if part.isdigit() else (1, 0, part)
                        for part in prerelease.split("."))
    return numbers, (0, identifiers)


def parse_version(text: str) -> tuple[int, int, int] | None:
    """把 `v0.2.0` 之類的版本字串解析成三段整數；解析不出來回 None。
    只回版號本身，預發布後綴的優先序見 `_precedence_key`。"""
    key = _precedence_key(text)
    return None if key is None else key[0]


def is_newer(latest: str, current: str) -> bool:
    """latest 是否嚴格新於 current（含 SemVer 預發布優先序：`0.2.0-rc.1` < `0.2.0`）。
    任一邊解析不出來就回 False —— 寧可漏提醒也不要誤報把使用者導去下載頁。"""
    newer, mine = _precedence_key(latest), _precedence_key(current)
    if newer is None or mine is None:
        log(f"[update] version unparsable: latest={latest!r} current={current!r}")
        return False
    return newer > mine


@dataclass(frozen=True)
class Release:
    """GitHub 上的一個 release：版本號（已去 `v` 前綴）與該 release 的網頁網址。"""
    version: str
    url: str


class UpdateCheckError(Exception):
    """檢查更新失敗（連線不通、配額用盡、回應無法解析）。
    「沒有新版」與「還沒發過 release」是正常結果，不走這個例外。"""


def _fetch(http) -> Release | None:
    try:
        resp = http.get(LATEST_API, headers=_HEADERS)
    except httpx.HTTPError as exc:
        raise UpdateCheckError(str(exc)) from exc
    if resp.status_code == 404:
        log("[update] no release published yet (404)")
        return None
    if resp.status_code != 200:
        raise UpdateCheckError(f"HTTP {resp.status_code}")
    try:
        data = resp.json()
        tag = str(data["tag_name"])
        url = str(data.get("html_url") or RELEASES_URL)
    except (ValueError, TypeError, KeyError, AttributeError) as exc:
        raise UpdateCheckError(f"unexpected response: {exc}") from exc
    return Release(version=tag.lstrip("vV"), url=url)


def fetch_latest_release(client=None) -> Release | None:
    """取 GitHub 上最新的正式 release（`/releases/latest` 已排除 draft 與 pre-release）；
    尚未發過任何 release（404）時回 None。`client` 供測試注入。"""
    if client is not None:
        return _fetch(client)
    # follow_redirects：repo 改名／搬家時 GitHub API 回 301，httpx 預設不跟隨，
    # 檢查更新會變成一律回報失敗
    with httpx.Client(timeout=_TIMEOUT, follow_redirects=True) as http:
        return _fetch(http)


def check_for_update(current: str = __version__, client=None) -> Release | None:
    """回傳「比 current 新的 release」，沒有新版（含尚未發版）時回 None。
    查詢失敗拋 UpdateCheckError，由呼叫端決定要顯示錯誤還是靜默。"""
    log("[update] checking latest release")
    release = fetch_latest_release(client)
    if release is None:
        return None
    newer = is_newer(release.version, current)
    log(f"[update] latest={release.version} current={current} newer={newer}")
    return release if newer else None
