"""app.log 保留清理的純邏輯測試：以 session 標頭切段、只留 7 天內的段落。"""
from datetime import datetime, timedelta, timezone

from src.main import LOG_RETENTION_DAYS, session_header, trim_log_sessions

NOW = datetime(2026, 8, 22, 12, 0, 0, tzinfo=timezone.utc)


def _session(dt: datetime, *lines: str) -> str:
    return session_header(dt) + "\n" + "".join(f"{ln}\n" for ln in lines)


def test_recent_session_kept():
    text = _session(NOW - timedelta(hours=3), "[app] startup", "[reader] attached")
    assert trim_log_sessions(text, NOW) == text


def test_expired_session_dropped():
    old = NOW - timedelta(days=LOG_RETENTION_DAYS, hours=1)
    text = _session(old, "[app] startup")
    assert trim_log_sessions(text, NOW) == ""


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
