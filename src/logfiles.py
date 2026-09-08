"""log 檔基礎設施：session 分段標頭、過期段落清理、每行 UTC 時戳。

`app.log`（診斷輸出）與 `messages.log`（收訊原始內容）共用這裡的規則——
每次啟動寫一行 session 標頭、只保留近 LOG_RETENTION_DAYS 天的段落，
輸出經 TimestampedStream 包裝後每行前綴一個 UTC＋0 時戳。
"""
import os
import re
import threading
from datetime import UTC, datetime, timedelta
from pathlib import Path

from src.config import app_dir
from src.log import log

SESSION_HEADER_PREFIX = "===== session started "
LOG_RETENTION_DAYS = 7        # log 保留天數（以 session 標頭日期判斷）
_LOG_HARD_CAP = 5 * 1024 * 1024   # 異常灌爆保險絲：超過就先砍到尾端再清理
_LOG_KEEP_TAIL = 1 * 1024 * 1024
_HEADER_TS_FORMAT = "%Y-%m-%dT%H:%M:%SZ"
_STAMP_FORMAT = "%Y-%m-%dT%H:%M:%S.%f"


def utc_stamp(now: datetime | None = None) -> str:
    """行首時戳（UTC＋0、毫秒）。poll 只隔零點幾秒，秒級解析度分不出先後。"""
    now = (now or datetime.now(UTC)).astimezone(UTC)
    return now.strftime(_STAMP_FORMAT)[:-3] + "Z"


def session_header(now: datetime) -> str:
    """log 的啟動分段標頭（UTC＋0）。"""
    return f"{SESSION_HEADER_PREFIX}{now.strftime(_HEADER_TS_FORMAT)} ====="


def trim_log_sessions(text: str, now: datetime) -> str:
    """以 session 標頭把 log 切段，只保留 LOG_RETENTION_DAYS 內開始的段落。
    無標頭的開頭內容（舊格式）與標頭解析失敗的段落一併視為過期丟棄。"""
    cutoff = now - timedelta(days=LOG_RETENTION_DAYS)
    keep: list[str] = []
    keeping = False
    for line in text.splitlines(keepends=True):
        if line.startswith(SESSION_HEADER_PREFIX):
            token = line[len(SESSION_HEADER_PREFIX):].split(" ")[0]
            try:
                ts = datetime.strptime(token, _HEADER_TS_FORMAT).replace(
                    tzinfo=UTC)
            except ValueError:
                keeping = False
            else:
                keeping = ts >= cutoff
        if keeping:
            keep.append(line)
    return "".join(keep)


def _prepare_log(path: Path, now: datetime) -> None:
    """開檔前清理過期段落；檔案異常肥大時先砍到尾端再清理，避免拖慢啟動。
    寫回一律用 bytes：文字模式會把既有的 CRLF 再翻成 CR CR LF，每次改寫多疊一個 CR，
    舊紀錄在編輯器裡就變成一堆空行。先把行尾正規化，順便修好之前疊壞的檔案。"""
    if not path.exists():
        return
    raw = path.read_bytes()
    if len(raw) > _LOG_HARD_CAP:
        raw = raw[-_LOG_KEEP_TAIL:]
    text = re.sub(r"\r*\n", "\n", raw.decode("utf-8", errors="replace"))
    out = trim_log_sessions(text, now).replace("\n", os.linesep).encode("utf-8")
    if out != raw:
        path.write_bytes(out)


def open_session_log(name: str, now: datetime | None = None):
    """開啟 app_dir() 旁的 log 檔：清掉過期段落後以 append 開檔並寫入 session 標頭。
    資料夾不可寫時退回 devnull——windowed 模式沒有主控台，不能讓開檔失敗把程式帶掉。"""
    now = now or datetime.now(UTC)
    path = app_dir() / name
    try:
        _prepare_log(path, now)
        stream = open(path, "a", encoding="utf-8", buffering=1)  # noqa: SIM115
        stream.write(session_header(now) + "\n")
    except OSError as exc:
        log(f"[log] cannot open {name}: {exc}")
        return open(os.devnull, "w", encoding="utf-8")
    return stream


class TimestampedStream:
    """把每個行首補上 UTC 時戳的輸出包裝（其餘屬性委派給被包住的串流）。
    不能單純對每次 write 加前綴：`print` 把內文與換行分兩次 write，多行 traceback
    則一次 write 進來，故以「是否停在行首」為狀態逐行處理。"""

    def __init__(self, stream, stamp=utc_stamp):
        self._stream = stream
        self._stamp = stamp
        self._at_line_start = True
        # reader／翻譯 worker／更新檢查等執行緒都會寫：不鎖的話「是否在行首」會被
        # 別的執行緒改掉，實機 app.log 出現過沒有時戳的行
        self._lock = threading.Lock()

    def write(self, text: str) -> int:
        with self._lock:
            for part in text.splitlines(keepends=True):
                if self._at_line_start:
                    self._stream.write(f"{self._stamp()} ")
                self._stream.write(part)
                self._at_line_start = part.endswith(("\n", "\r"))
        return len(text)

    def __getattr__(self, name):
        return getattr(self._stream, name)
