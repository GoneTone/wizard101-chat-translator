"""WizChatReader 的 hook 事件流：記錄 → ChatEvent、廣播收斂、外來 ChatBox、分頁狀態（不需遊戲）。"""
import asyncio
import io
import struct

from src.reader import mem_reader
from src.reader.chat_hook import HookRecord, RingRead
from src.reader.chat_tabs import CUR_TAB_OFFSET, LABEL_OFFSET, TAB_BUTTONS, TabState
from src.reader.mem_reader import WizChatReader
from src.reader.message_log import MessageLog
from tests.chatlog_helpers import _broadcast, _own, _say, _system
from tests.test_chat_hook import RING, _FakeRing
from tests.test_chat_tabs import _wstring

BOX = 0xA000


def _rec(seq, text, tab=0, this=BOX):
    return HookRecord(seq, 0x1407443B1, this, tab, len(text), text)


def _reader(monkeypatch, *reads, buf=None, chatbox=BOX):
    """已連上、依序回傳 reads（RingRead）的 reader；回傳 (reader, app.log 行清單)。"""
    logged = []
    monkeypatch.setattr(mem_reader, "log", logged.append)
    r = WizChatReader(0x1, message_log=MessageLog(buf) if buf is not None else None, slot=3)
    r._connected = True
    r._chatbox = chatbox
    queue = list(reads)
    monkeypatch.setattr(r, "_read_records", lambda: queue.pop(0))
    return r, logged


def _ring(*records):
    return RingRead(list(records), records[-1].seq, 0)


def _texts(events):
    return [(e.tabs, e.line.text) for e in events]


def test_records_become_events_in_order(monkeypatch):
    r, _ = _reader(monkeypatch, _ring(_rec(1, _say(1, "Amy", "hi")), _rec(2, _own("yo"), tab=1)))
    assert _texts(r.read_new()) == [((0,), "[Amy] hi"), ((1,), "[你] yo")]


def test_identical_consecutive_lines_both_emit(monkeypatch):
    drop = _system("你获得了 39 金币！")
    r, _ = _reader(monkeypatch, _ring(_rec(1, drop), _rec(2, drop)))
    r.emit_system = True
    events = r.read_new()
    assert _texts(events) == [((0,), "你获得了 39 金币！")] * 2
    assert all(e.line.system for e in events)


def test_system_lines_are_skipped_when_emit_system_off(monkeypatch):
    buf = io.StringIO()
    r, _ = _reader(monkeypatch, _ring(_rec(1, _system("掉寶")), _rec(2, _say(1, "Amy", "hi"))),
                   RingRead([], 2, 0), buf=buf)
    assert _texts(r.read_new()) == [((0,), "[Amy] hi")]
    assert r._last_seq == 2
    r.emit_system = True
    assert r.read_new() == []
    assert "[seq=1 tab=0 system-off]" in buf.getvalue()


def test_broadcast_becomes_one_event_for_all_tabs(monkeypatch):
    notice = _broadcast("D9ABF8", "[Server Message] maintenance")
    buf = io.StringIO()
    r, logged = _reader(monkeypatch, _ring(*[_rec(i + 1, notice, tab=i) for i in range(4)]),
                        buf=buf)
    r.emit_system = True
    assert _texts(r.read_new()) == [((0, 1, 2, 3), "[Server Message] maintenance")]
    out = buf.getvalue()
    assert "[seq=1 tab=0 broadcast]" in out
    assert out.count("broadcast-dup") == 3
    assert any("across 4 tabs" in line for line in logged)


def test_unparseable_lines_are_dropped_and_logged(monkeypatch):
    unknown = ("<color;FFFFFF><image;Art/Art_Unknown_Channel.dds;24;24;FFFFFFFF> "
               "[Lars] hi </color>")
    buf = io.StringIO()
    r, _ = _reader(monkeypatch, _ring(_rec(1, "[DBGM] some debug noise"), _rec(2, unknown)),
                   buf=buf)
    assert r.read_new() == []
    assert buf.getvalue() == f"[seq=2 tab=0 dropped] {unknown}\n"


def test_unknown_chatbox_triggers_one_refind_before_dropping(monkeypatch):
    other = 0xB000
    line = _say(1, "Amy", "hi")
    buf = io.StringIO()
    r, logged = _reader(monkeypatch, _ring(_rec(1, line, this=other)),
                        _ring(_rec(2, line, this=other), _rec(3, line, this=other)), buf=buf)
    finds = []
    monkeypatch.setattr(r, "_find_chatbox", lambda: finds.append(1) or BOX)
    assert r.read_new() == []
    assert r.read_new() == []
    assert len(finds) == 1, "同一個外來 this 不該再重找"
    assert buf.getvalue().count("foreign") == 3
    assert sum("another chat box" in m for m in logged) == 1

    r2, _ = _reader(monkeypatch, _ring(_rec(1, line, this=other)))
    monkeypatch.setattr(r2, "_find_chatbox", lambda: other)
    assert _texts(r2.read_new()) == [((0,), "[Amy] hi")]
    assert r2._chatbox == other


def test_missing_chatbox_does_not_mark_the_record_foreign(monkeypatch):
    # 轉場中找不到 ChatBox 不能把主 ChatBox 永久當成外來
    line = _say(1, "Amy", "hi")
    r, _ = _reader(monkeypatch, _ring(_rec(1, line)), _ring(_rec(2, line)), chatbox=None)
    finds = iter([None, BOX])
    monkeypatch.setattr(r, "_find_chatbox", lambda: next(finds))
    assert r.read_new() == []
    assert _texts(r.read_new()) == [((0,), "[Amy] hi")]


def test_lost_records_are_logged_with_the_count(monkeypatch):
    buf = io.StringIO()
    r, logged = _reader(monkeypatch, RingRead([_rec(5, _say(1, "Amy", "hi"))], 5, 3), buf=buf)
    assert len(r.read_new()) == 1
    assert any("lost 3" in m and "slot=3" in m for m in logged)
    assert "lost 3" in buf.getvalue()


def test_truncated_lines_are_logged_with_the_original_length(monkeypatch):
    line = _say(1, "Amy", "hi")
    r, logged = _reader(monkeypatch, _ring(_rec(1, line)._replace(length=1500)))
    assert len(r.read_new()) == 1
    assert any("truncated" in m and "length=1500" in m for m in logged)


def test_teardown_resets_the_ring_cursor(monkeypatch):
    monkeypatch.setattr(mem_reader, "log", lambda m: None)
    ring = _FakeRing()

    class Client:
        hook_handler = ring

        async def close(self):
            pass

    def connect():
        r._loop = asyncio.new_event_loop()
        r._client = Client()
        r._ring = RING
        r._connected = True

    r = WizChatReader(0x1)
    monkeypatch.setattr(r, "_connect", connect)
    monkeypatch.setattr(r, "_find_chatbox", lambda: 0x7777)
    for seq in range(1, 6):
        ring.put(seq, _say(1, "Amy", f"m{seq}"))
    assert len(r.read_new()) == 5
    r._teardown()
    ring.buf[:] = bytes(len(ring.buf))
    ring.put(1, _say(1, "Amy", "after"))
    assert _texts(r.read_new()) == [((0,), "[Amy] after")]
    r.close()


class _Node:
    def __init__(self, base):
        self.base = base

    async def read_base_address(self):
        return self.base


def _tab_reader(monkeypatch, labels: dict[str, str], current: int, chatbox=BOX):
    """假記憶體：ChatBox 的目前分頁與各按鈕文字（未列出的按鈕是隱藏分頁）。"""
    mem = {chatbox + CUR_TAB_OFFSET: struct.pack("<i", current)}
    nodes = {}
    for i, (name, _tab) in enumerate(TAB_BUTTONS):
        base = 0x10000 * (i + 1)
        nodes[name] = [_Node(base)]
        raw, heap = _wstring(labels.get(name, "<string;GUI2_00001234>"), 23, base + 0x8000)
        mem[base + LABEL_OFFSET] = raw
        mem.update(heap)

    class Handler:
        async def read_bytes(self, addr, size):
            return mem[addr][:size]

    class Root:
        async def get_windows_with_name(self, name):
            return nodes.get(name, [])

    class Client:
        hook_handler = Handler()
        root_window = Root()

    logged = []
    monkeypatch.setattr(mem_reader, "log", logged.append)
    r = WizChatReader(0x1)
    r._connected = True
    r._loop = asyncio.new_event_loop()
    r._client = Client()
    monkeypatch.setattr(r, "_find_chatbox", lambda: chatbox)
    return r, logged


def test_tab_state_reads_current_tab_and_visible_labels(monkeypatch):
    r, logged = _tab_reader(monkeypatch, {"MainFilterButton": "主要的",
                                          "GroupFilterButton": "队伍(2)"}, current=1)
    expected = TabState(1, ((0, "主要的"), (1, "队伍(2)")))
    assert r.tab_state() == expected
    assert r.tab_state() == expected
    assert sum("chat tabs" in m for m in logged) == 1, "分頁狀態沒變不該重複記錄"
    r._loop.close()


def test_tab_state_is_none_without_a_chatbox(monkeypatch):
    r, _ = _tab_reader(monkeypatch, {}, current=0)
    monkeypatch.setattr(r, "_find_chatbox", lambda: None)
    assert r.tab_state() is None
    r._loop.close()


def test_tab_state_is_none_when_not_connected():
    assert WizChatReader(0x1).tab_state() is None

