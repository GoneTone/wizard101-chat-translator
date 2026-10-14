"""WizChatReader 的 hook 事件流：記錄 → ChatEvent、廣播收斂、外來 ChatBox、分頁狀態（不需遊戲）。"""
import asyncio
import io
import struct
from types import SimpleNamespace

from src.reader import mem_reader
from src.reader.chat_hook import HookRecord, RingRead
from src.reader.chat_tabs import CUR_TAB_OFFSET, LABEL_OFFSET, TAB_BUTTONS, TabState
from src.reader.mem_reader import WizChatReader
from src.reader.message_log import MessageLog
from tests.chat_hook_helpers import RING, THIS, _FakeRing, _units, _wstring
from tests.chatlog_helpers import _broadcast, _own, _say, _system

BOX = 0xA000


def _rec(seq, text, tab=0, this=BOX):
    return HookRecord(seq, 0x1407443B1, this, tab, _units(text), _units(text), text)


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


def test_broadcast_split_across_polls_is_held_and_collapsed(monkeypatch):
    notice = _broadcast("D9ABF8", "[Server Message] maintenance")
    hi = _rec(1, _say(1, "Amy", "hi"))
    head = [_rec(2, notice, tab=0), _rec(3, notice, tab=1)]
    full = head + [_rec(4, notice, tab=2), _rec(5, notice, tab=3)]
    buf = io.StringIO()
    r, _ = _reader(monkeypatch, _ring(hi, *head), _ring(*full), buf=buf)
    r.emit_system = True
    assert _texts(r.read_new()) == [((0,), "[Amy] hi")]
    assert r._last_seq == 1
    assert _texts(r.read_new()) == [((0, 1, 2, 3), "[Server Message] maintenance")]
    assert buf.getvalue().count("[seq=2 ") == 1, "保留的記錄不該重複寫進 messages.log"


def test_a_held_tail_is_released_when_nothing_follows(monkeypatch):
    # 同一段尾巴只保留一輪：遊戲真的只寫了 0、1 兩筆時不能永遠卡住
    line = _say(1, "Amy", "hi")
    pair = [_rec(1, line, tab=0), _rec(2, line, tab=1)]
    r, _ = _reader(monkeypatch, _ring(*pair), _ring(*pair))
    assert r.read_new() == []
    assert _texts(r.read_new()) == [((0,), "[Amy] hi"), ((1,), "[Amy] hi")]
    assert r._last_seq == 2


def test_a_lone_main_tab_line_is_not_held(monkeypatch):
    r, _ = _reader(monkeypatch, _ring(_rec(1, _say(1, "Amy", "hi"))))
    assert _texts(r.read_new()) == [((0,), "[Amy] hi")]
    assert r._last_seq == 1


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
    buf = io.StringIO()
    r, _ = _reader(monkeypatch, _ring(_rec(1, line)), _ring(_rec(2, line)), buf=buf,
                   chatbox=None)
    finds = iter([None, BOX])
    monkeypatch.setattr(r, "_find_chatbox", lambda: next(finds))
    assert r.read_new() == []
    assert _texts(r.read_new()) == [((0,), "[Amy] hi")]
    assert buf.getvalue().startswith("[seq=1 tab=0 no-chatbox]")


def test_missing_chatbox_is_refound_once_per_poll(monkeypatch):
    line = _say(1, "Amy", "hi")
    r, logged = _reader(monkeypatch, _ring(*[_rec(i, line) for i in (1, 2, 3)]), chatbox=None)
    finds = []
    monkeypatch.setattr(r, "_find_chatbox", lambda: finds.append(1))
    assert r.read_new() == []
    assert len(finds) == 1
    dropped = [m for m in logged if "main chat box not found" in m]
    assert len(dropped) == 1 and "dropped=3" in dropped[0]


def test_chatbox_moving_clears_foreign_and_button_caches(monkeypatch):
    # 新的主 ChatBox 可能落在先前判定為外來的位址上
    moved = 0xB000
    line = _say(1, "Amy", "hi")
    r, _ = _reader(monkeypatch, _ring(_rec(1, line, this=moved)))
    r._foreign.add(moved + 0x100)
    r._buttons = [(0, 0x10000)]
    monkeypatch.setattr(r, "_find_chatbox", lambda: moved)
    assert _texts(r.read_new()) == [((0,), "[Amy] hi")]
    assert r._foreign == set()
    assert r._buttons is None


def test_foreign_address_is_rechecked_after_the_window(monkeypatch):
    moved = 0xB000
    line = _say(1, "Amy", "hi")
    r, _ = _reader(monkeypatch, *[_ring(_rec(i, line, this=moved)) for i in (1, 2, 3)])
    clock = [100.0]
    monkeypatch.setattr(mem_reader, "time", SimpleNamespace(monotonic=lambda: clock[0]))
    found = [BOX]
    finds = []
    monkeypatch.setattr(r, "_find_chatbox", lambda: finds.append(1) or found[0])
    assert r.read_new() == []
    found[0] = moved  # 主 ChatBox 重建在先前判定為外來的位址
    clock[0] += mem_reader.FOREIGN_RECHECK_SECONDS - 0.1
    assert r.read_new() == []
    assert len(finds) == 1, "限頻窗內不該重找"
    clock[0] += 0.2
    assert _texts(r.read_new()) == [((0,), "[Amy] hi")]
    assert len(finds) == 2 and r._chatbox == moved and r._foreign == set()


def test_refinding_the_same_chatbox_is_a_no_op(monkeypatch):
    r, logged = _tab_reader(monkeypatch, {"MainFilterButton": "主要的"}, current=0)
    r._chatbox, r._buttons = BOX, None  # 分頁讀取失敗後的狀態
    r._foreign.add(0xB000)
    assert r.tab_state() == TabState(0, ((0, "主要的"),))
    assert r._foreign == {0xB000}
    assert not any("main chat box" in m for m in logged)
    r._loop.close()


def test_lost_records_are_logged_with_the_count(monkeypatch):
    buf = io.StringIO()
    r, logged = _reader(monkeypatch, RingRead([_rec(5, _say(1, "Amy", "hi"))], 5, 3), buf=buf)
    assert len(r.read_new()) == 1
    assert any("lost 3" in m and "slot=3" in m and "window=1-5" in m for m in logged)
    assert "lost 3" in buf.getvalue()


def test_truncated_lines_are_logged_with_the_original_length(monkeypatch):
    line = _say(1, "Amy", "hi")
    r, logged = _reader(monkeypatch, _ring(_rec(1, line)._replace(length=1500)))
    assert len(r.read_new()) == 1
    assert any("truncated" in m and "length=1500" in m for m in logged)


def test_non_bmp_text_is_not_reported_as_truncated(monkeypatch):
    # emoji 在 UTF-16 是兩個單位：拿 Python 字元數比會誤判截斷
    r, logged = _reader(monkeypatch, _ring(_rec(1, _say(1, "Amy", "nice 😂👀"))))
    assert _texts(r.read_new()) == [((0,), "[Amy] nice 😂👀")]
    assert not any("truncated" in m for m in logged)


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
    monkeypatch.setattr(r, "_find_chatbox", lambda: THIS)
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


def test_repeated_chatbox_lookup_failures_log_once(monkeypatch):
    r, logged = _tab_reader(monkeypatch, {}, current=0)

    def boom():
        raise OSError("root window unreadable")

    monkeypatch.setattr(r, "_find_chatbox", boom)
    assert [r.tab_state() for _ in range(3)] == [None] * 3
    assert sum("root window unreadable" in m for m in logged) == 1
    r._loop.close()


def test_tab_buttons_are_refound_after_the_chatbox_moves(monkeypatch):
    moved = 0xB000
    r, _ = _tab_reader(monkeypatch, {"MainFilterButton": "主要的"}, current=0, chatbox=moved)
    r._chatbox, r._buttons = BOX, [(0, 0xDEAD0000)]  # 舊 ChatBox 的按鈕，已不可讀
    assert r._chatbox_verdict(_rec(1, _say(1, "Amy", "hi"), this=moved)) is None
    assert r.tab_state() == TabState(0, ((0, "主要的"),))
    r._loop.close()


def test_no_visible_tab_buttons_are_not_rescanned_every_poll(monkeypatch):
    r, _ = _tab_reader(monkeypatch, {}, current=0)
    scans = []

    async def counting():
        scans.append(1)
        return []

    monkeypatch.setattr(r, "_find_tab_buttons", counting)
    r._chatbox, r._buttons = BOX, None
    assert r.tab_state() == TabState(0, ())
    assert r.tab_state() == TabState(0, ())
    assert len(scans) == 1
    r._loop.close()


def test_out_of_range_current_tab_is_a_read_failure(monkeypatch):
    # 登出入後讀到已釋放的舊 ChatBox：不回報垃圾分頁，下一輪重找
    r, logged = _tab_reader(monkeypatch, {"MainFilterButton": "主要的"}, current=0x41414141)
    assert r.tab_state() is None
    assert r._buttons is None
    assert any("read failed" in m for m in logged)
    r._loop.close()


def test_tab_state_is_none_when_not_connected():
    assert WizChatReader(0x1).tab_state() is None

