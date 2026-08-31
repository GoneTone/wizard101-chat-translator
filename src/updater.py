"""更新檢查：回答「GitHub 上有沒有比目前這版更新的 release」。

只負責查詢與比較，不碰 UI、不碰 config，也不下載或安裝任何東西——要不要提醒、
怎麼提醒由呼叫端決定（啟動路徑走 overlay 橫幅，設定視窗走「關於」分頁）。
"""
import re
import sys

from src import __version__

GITHUB_REPO = "GoneTone/wizard101-chat-translator"
PROJECT_URL = f"https://github.com/{GITHUB_REPO}"
RELEASES_URL = f"{PROJECT_URL}/releases/latest"
AUTHOR_URL = "https://github.com/GoneTone"
LATEST_API = f"https://api.github.com/repos/{GITHUB_REPO}/releases/latest"

# 只取前三段數字，後綴（-beta.1、+build）一律忽略：/releases/latest 已排除
# pre-release，這裡容忍後綴只是為了不因為 tag 寫法而整個解析失敗。
_VERSION_RE = re.compile(r"^v?(\d+)\.(\d+)\.(\d+)")


def parse_version(text: str) -> tuple[int, int, int] | None:
    """把 `v0.2.0` 之類的版本字串解析成可比較的三段整數；解析不出來回 None。"""
    match = _VERSION_RE.match(text.strip()) if text else None
    if match is None:
        return None
    major, minor, patch = match.groups()
    return int(major), int(minor), int(patch)


def is_newer(latest: str, current: str) -> bool:
    """latest 是否嚴格新於 current。

    任一邊解析不出來就回 False——寧可漏提醒也不要誤報，畢竟提醒會把使用者
    導去下載頁。"""
    newer, mine = parse_version(latest), parse_version(current)
    if newer is None or mine is None:
        print(f"[update] version unparsable: latest={latest!r} current={current!r}",
              file=sys.stderr)
        return False
    return newer > mine
