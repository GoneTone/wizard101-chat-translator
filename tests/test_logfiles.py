"""log 檔基礎設施的純邏輯測試：每行 UTC 時戳前綴、session 分段保留。"""
import io
from datetime import UTC, datetime, timedelta, timezone

from src.logfiles import (
    LOG_RETENTION_DAYS,
    TimestampedStream,
    session_header,
    trim_log_sessions,
    utc_stamp,
)

NOW = datetime(2026, 8, 22, 12, 0, 0, tzinfo=UTC)


# --- utc_stamp ---
def test_utc_stamp_has_millisecond_precision():
    ts = datetime(2026, 8, 25, 9, 12, 3, 412345, tzinfo=UTC)
    assert utc_stamp(ts) == "2026-08-25T09:12:03.412Z"


def test_utc_stamp_converts_to_utc():
    ts = datetime(2026, 8, 25, 17, 12, 3, 0, tzinfo=timezone(timedelta(hours=8)))
    assert utc_stamp(ts) == "2026-08-25T09:12:03.000Z"


# --- TimestampedStream ---
def _stream():
    buf = io.StringIO()
    stamps = iter(["T1 ", "T2 ", "T3 "])
    return buf, TimestampedStream(buf, stamp=lambda: next(stamps).strip())


def test_each_line_gets_a_stamp_prefix():
    buf, out = _stream()
    out.write("[app] one\n[reader] two\n")
    assert buf.getvalue() == "T1 [app] one\nT2 [reader] two\n"


def test_split_writes_share_one_line_prefix():
    buf, out = _stream()
    out.write("[app] hello")      # print 先寫內文……
    out.write("\n")               # ……再單獨寫換行，仍算同一行
    assert buf.getvalue() == "T1 [app] hello\n"


def test_next_line_after_split_write_gets_new_stamp():
    buf, out = _stream()
    out.write("[app] a\n")
    out.write("[app] b\n")
    assert buf.getvalue() == "T1 [app] a\nT2 [app] b\n"


def test_blank_line_is_stamped():
    buf, out = _stream()
    out.write("\n[app] after blank\n")
    assert buf.getvalue() == "T1 \nT2 [app] after blank\n"


def test_empty_write_emits_nothing():
    buf, out = _stream()
    out.write("")
    assert buf.getvalue() == ""


def test_flush_and_close_reach_the_wrapped_stream():
    buf, out = _stream()
    out.write("[app] x\n")
    out.flush()
    assert buf.getvalue() == "T1 [app] x\n"
    out.close()
    assert buf.closed


# --- session 分段保留（沿用原 app.log 行為）---
def _session(dt: datetime, *lines: str) -> str:
    return session_header(dt) + "\n" + "".join(f"{ln}\n" for ln in lines)


def test_recent_session_kept():
    text = _session(NOW - timedelta(hours=3), "[app] startup", "[reader] attached")
    assert trim_log_sessions(text, NOW) == text


def test_expired_session_dropped():
    old = NOW - timedelta(days=LOG_RETENTION_DAYS, hours=1)
    assert trim_log_sessions(_session(old, "[app] startup"), NOW) == ""


def test_mixed_sessions_keep_only_recent():
    old = _session(NOW - timedelta(days=10), "[app] old stuff")
    recent = _session(NOW - timedelta(days=2), "[app] recent stuff")
    assert trim_log_sessions(old + recent, NOW) == recent


def test_headerless_legacy_content_dropped():
    legacy = "[app] running without header\n"
    recent = _session(NOW, "[app] new format")
    assert trim_log_sessions(legacy + recent, NOW) == recent


def test_unparseable_header_section_dropped():
    bad = "===== session started not-a-date =====\n[app] noise\n"
    recent = _session(NOW, "[app] good")
    assert trim_log_sessions(bad + recent, NOW) == recent


def test_empty_text_stays_empty():
    assert trim_log_sessions("", NOW) == ""


def test_concurrent_writers_keep_every_line_stamped():
    # 多執行緒同時寫（reader／翻譯 worker／更新檢查）：實機 app.log 曾出現沒時戳的行
    import threading

    buf = io.StringIO()
    stream = TimestampedStream(buf, stamp=lambda: "T")

    def writer(tag):
        for i in range(300):
            stream.write(f"[{tag}] line {i}\n")

    threads = [threading.Thread(target=writer, args=(t,)) for t in "abcd"]
    for th in threads:
        th.start()
    for th in threads:
        th.join()
    lines = buf.getvalue().splitlines()
    assert len(lines) == 1200
    assert all(line.startswith("T [") for line in lines)


def test_log_writes_a_whole_line_in_one_call(monkeypatch):
    # print 會把內文與換行分兩次 write，行首時戳的狀態就可能被別的執行緒插隊
    from src.log import log

    class Recorder:
        def __init__(self):
            self.calls = []

        def write(self, text):
            self.calls.append(text)

    rec = Recorder()
    monkeypatch.setattr("sys.stderr", rec)
    log("[x] hello")
    assert rec.calls == ["[x] hello\n"]
