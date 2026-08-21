from src.reader.mem_reader import (
    LiveChatReader, after_last_tail, align_append, clean, extract_lines,
    groups_in_blob,
)


def u16(s: str) -> bytes:
    return s.encode("utf-16-le")


SAY = "<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [Wolf] hello world </color>"


def test_clean_strips_markup_and_collapses_space():
    assert clean("<color;FFFFFF><image;Art/x.dds;24;24;FFFFFFFF> [你] hi there </color>") == "[你] hi there"


def test_extract_valid_say_line():
    assert extract_lines(u16("noise" + SAY + "noise")) == ["[Wolf] hello world"]


def test_extract_keeps_sender_and_text():
    line = "<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [艾米 时空之翼] test123 </color>"
    assert extract_lines(u16(line)) == ["[艾米 时空之翼] test123"]


def test_extract_rejects_gui_template_string():
    # 說明/範本字串:GUI/ 前綴、22;22;0x —— 不含真實聊天簽章,應被排除
    tmpl = "<image;GUI/Art/Art_Chat_Say.dds;22;22;0xFFFFFFFF> some help text about chat"
    assert extract_lines(u16(tmpl)) == []


def test_extract_rejects_line_without_sender_bracket():
    no_sender = "<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> just text no bracket </color>"
    assert extract_lines(u16(no_sender)) == []


def _wrap(inner: str) -> bytes:
    return u16(f"<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> {inner} </color>")


def test_extract_keeps_message_with_brackets_and_symbols():
    # 訊息真的含 [] <> 等符號也不能誤擋(過濾器已移除,只驗格式)
    assert extract_lines(_wrap("[Amy] use [fire] then [storm]")) == ["[Amy] use [fire] then [storm]"]


def test_clean_unescapes_player_typed_entities():
    # 玩家打的 < > & 在記憶體存成 HTML 實體,顯示時要還原
    assert extract_lines(_wrap("[Han] i hear you &gt; and more")) == ["[Han] i hear you > and more"]
    assert extract_lines(_wrap("[Amy] a &lt;3 b &amp; c")) == ["[Amy] a <3 b & c"]


def test_extract_keeps_clean_cjk_sender():
    assert extract_lines(_wrap("[本杰明] how do u do the eyes")) == ["[本杰明] how do u do the eyes"]


def test_extract_keeps_clean_english():
    assert extract_lines(_wrap("[Luc BelleMer] i cant even blame you")) == ["[Luc BelleMer] i cant even blame you"]


def test_extract_skips_when_no_close_tag():
    open_only = "<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [Wolf] dangling"
    assert extract_lines(u16(open_only)) == []


def test_extract_dedups_within_blob():
    assert extract_lines(u16(SAY + " " + SAY)) == ["[Wolf] hello world"]


def test_extract_no_dedup_keeps_repeats():
    a = "<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [A] hi </color>"
    blob = u16(a + a)
    assert extract_lines(blob, dedup=False) == ["[A] hi", "[A] hi"]
    assert extract_lines(blob, dedup=True) == ["[A] hi"]


def test_extract_multiple_distinct_lines_in_order():
    a = "<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [A] first </color>"
    b = "<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [B] second </color>"
    assert extract_lines(u16(a + b)) == ["[A] first", "[B] second"]


# --- groups_in_blob ---
def _say(inner: str) -> str:
    return f"<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> {inner} </color>"


def test_groups_in_blob_keeps_repeats_in_order():
    blob = u16(_say("[A] hi") + _say("[B] yo") + _say("[A] hi"))
    groups = groups_in_blob(blob)
    assert len(groups) == 1
    start, end, lines = groups[0]
    assert lines == ["[A] hi", "[B] yo", "[A] hi"]
    assert start == 0 and end > start


def test_groups_in_blob_splits_on_gap():
    gap = b"\x00" * 8000  # 超過 _GROUP_GAP → 分成兩群
    blob = u16(_say("[A] one") + _say("[A] two")) + gap + u16(_say("[B] three") + _say("[B] four"))
    groups = groups_in_blob(blob)
    assert [g[2] for g in groups] == [["[A] one", "[A] two"], ["[B] three", "[B] four"]]


def test_groups_in_blob_keeps_single_marker_group():
    # 空聊天室只有一句時,文件只有 1 個標記 —— 也要看得到,否則無法定位
    blob = u16(_say("[A] alone"))
    assert [g[2] for g in groups_in_blob(blob)] == [["[A] alone"]]


# --- align_append ---
def test_align_pure_append():
    assert align_append(["a", "b"], ["a", "b", "c", "d"]) == ["c", "d"]


def test_align_scroll_and_append():
    assert align_append(["a", "b", "c"], ["b", "c", "d"]) == ["d"]


def test_align_unchanged():
    assert align_append(["a", "b"], ["a", "b"]) == []


def test_align_repeats_detected():
    assert align_append(["x", "np"], ["x", "np", "np"]) == ["np"]


def test_align_misaligned_returns_none():
    assert align_append(["a", "b"], ["x", "y"]) is None
    assert align_append([], ["a"]) is None


# --- after_last_tail ---
def test_after_last_tail_finds_last_occurrence():
    doc = ["a", "b", "a", "b", "c"]
    assert after_last_tail(doc, ["a", "b"]) == ["c"]


def test_after_last_tail_falls_back_to_shorter_suffix():
    doc = ["x", "y", "z"]
    assert after_last_tail(doc, ["q", "y"]) == ["z"]  # 全尾不中 → 用最後 1 行


def test_after_last_tail_none_when_absent():
    assert after_last_tail(["x", "y"], ["q"]) is None


# --- LiveChatReader(以假掃描驗證定錨/輪詢/重定錨流程) ---
def test_clean_converts_emoticon_tag_to_emoji():
    raw = ("<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> "
           "<link;GID:123,Lars,2>[Lars]</link> fire first then "
           "<image;Emoticons/Laughter001.dds;24;24;FFFFFFFF> </color>")
    assert clean(raw) == "[Lars] fire first then 😂"


def test_clean_converts_prefixed_emoticon_names():
    assert clean("<image;Emoticons/Emoticons_Heart.dds;24;24;FFFFFFFF>") == "❤️"
    assert clean("<image;Emoticons/Emoticons_School_Death.dds;24;24;FFFFFFFF>") == "💀"
    assert clean("<image;Emoticons/Eyes001.dds;24;24;FFFFFFFF>") == "👀"
    assert clean("<image;Emoticons/TeaCup001.dds;24;24;FFFFFFFF>") == "🍵"


def test_clean_unknown_emoticon_falls_back_to_name():
    assert clean("<image;Emoticons/Zebra042.dds;24;24;FFFFFFFF>") == ":zebra:"


def test_extract_keeps_line_with_emoji():
    line = ("<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [Lars] ty king "
            "<image;Emoticons/Emoticons_Heart.dds;24;24;FFFFFFFF> </color>")
    assert extract_lines(u16(line)) == ["[Lars] ty king ❤️"]


def test_extract_keeps_astral_emoji_text():
    assert extract_lines(_wrap("[Amy] nice 😂👀")) == ["[Amy] nice 😂👀"]


def test_extract_torn_emoticon_dropped_but_line_kept():
    # 撕裂副本:表情名稱夾入雜字 → 丟棄該表情,行其餘內容保留
    torn = "[莫格瑞姆 霜冻] ok <image;Emoticons/E䍸儱牴耀cons_Laugh.dds;24;24;FFFFFFFF>"
    assert extract_lines(_wrap(torn)) == ["[莫格瑞姆 霜冻] ok"]


def test_extract_keeps_pure_emoticon_message():
    pure = "[Lars] <image;Emoticons/Emoticons_Wink.dds;24;24;FFFFFFFF>"
    assert extract_lines(_wrap(pure)) == ["[Lars] 😉"]


def test_inserted_lines_detects_mid_insertion():
    from src.reader.mem_reader import inserted_lines
    old = ["[A] a", "[B] b", "[C] c", "[D] d", "[E] e"]
    new = ["[A] a", "[B] b", "[你] Test", "[C] c", "[D] d", "[E] e"]
    assert inserted_lines(old, new) == ["[你] Test"]


def test_inserted_lines_trim_only_returns_empty():
    from src.reader.mem_reader import inserted_lines
    old = ["[A] a", "[B] b", "[C] c", "[D] d", "[E] e"]
    assert inserted_lines(old, old[1:]) == []


def test_inserted_lines_dissimilar_returns_none():
    from src.reader.mem_reader import inserted_lines
    assert inserted_lines(["[A] a", "[B] b"], ["[X] x", "[Y] y", "[Z] z"]) is None


def test_inserted_lines_dedupes_section_copies():
    from src.reader.mem_reader import inserted_lines
    old = ["[A] a", "[B] b", "[C] c", "[D] d", "[E] e", "[F] f"]
    new = ["[A] a", "[你] hi", "[B] b", "[C] c", "[D] d", "[你] hi", "[E] e", "[F] f"]
    assert inserted_lines(old, new) == ["[你] hi"]  # 同批的節副本只取一次


def test_inserted_lines_reflow_or_move_not_new():
    from src.reader.mem_reader import inserted_lines
    # 分節文件重繪:整段搬動/重排,行數量不變 → 不是新訊息(冒舊訊息回歸測試)
    old = ["[A] a", "[B] b", "[C] c", "[D] d", "[E] e"]
    moved = ["[C] c", "[D] d", "[E] e", "[A] a", "[B] b"]
    assert inserted_lines(old, moved) == []




# --- LiveChatReader:尾指紋追蹤最長純附加歷史 ---


# --- LiveChatReader:可視窗滾動對齊 ---
class FakeLive(LiveChatReader):
    """以假記憶體(dict: addr -> lines)取代掃描,驗證可視窗滾動/過期快照/自己 buffer。"""

    def __init__(self):
        super().__init__()
        self.mem: dict[int, list[str]] = {}

    def _open(self):
        return 1

    def _close(self, handle):
        pass

    def _full_docs(self, h):
        from src.reader.mem_reader import _MIN_WIN, _MAX_WIN
        return [(a, len(l) * 100, list(l)) for a, l in self.mem.items()
                if _MIN_WIN <= len(l) <= _MAX_WIN]

    def _cached_docs(self, h):
        from src.reader.mem_reader import _MIN_WIN, _MAX_WIN
        return [(a, len(self.mem[a]) * 100, list(self.mem[a]))
                for a in self._addrs if a in self.mem and _MIN_WIN <= len(self.mem[a]) <= _MAX_WIN]


WIN = 0x1000
WIN2 = 0x2000
SELFBUF = 0x3000


def _win(n, tail=()):
    # n 行、多發送者的可視窗
    base = [f"[U{i%4}] msg{i}" for i in range(n)]
    return base + list(tail)


def test_picks_multi_sender_window_over_self_buffer():
    r = FakeLive()
    r.mem = {WIN: _win(20), SELFBUF: ["[你] Test", "[你] hi", "[你] Test"]}
    assert r.read_new() == []                       # 首次:認出可視窗,不翻既有
    assert len(r._win) == 20                         # 選了多發送者的,不是自己 buffer


def test_scrolling_window_emits_new_including_repeats():
    r = FakeLive()
    r.mem = {WIN: _win(20)}
    r.read_new()
    r.mem[WIN] = _win(20) + ["[你] Test"]
    assert r.read_new() == ["[你] Test"]
    r.mem[WIN] = _win(20) + ["[你] Test", "[你] Test"]   # 重複同一句
    assert r.read_new() == ["[你] Test"]
    r.mem[WIN] = _win(20) + ["[你] Test", "[你] Test", "[B] bye"]
    assert r.read_new() == ["[B] bye"]


def test_head_trim_scroll_absorbed():
    r = FakeLive()
    r.mem = {WIN: _win(20)}
    r.read_new()
    # 視窗滿了:頂部丟一行、底部進一行(滾動)
    r.mem[WIN] = _win(20)[1:] + ["[C] new"]
    assert r.read_new() == ["[C] new"]


def test_stale_snapshot_never_re_emits():
    r = FakeLive()
    r.mem = {WIN: _win(20)}
    r.read_new()
    r.mem[WIN] = _win(20) + ["[你] a"]
    assert r.read_new() == ["[你] a"]
    # 過期快照(舊內容,尾端還沒有 a)持續存在 → 不含當前視窗的延續 → 不重播
    r.mem[SELFBUF] = _win(20)                        # 舊視窗副本
    for _ in range(3):
        assert r.read_new() == []


def test_pingpong_two_copies_no_spurious_output():
    # 可視窗雙緩衝:兩份副本,一份較新。有序性保證不在兩份間乒乓冒差異。
    r = FakeLive()
    r.mem = {WIN: _win(20), WIN2: _win(20)}
    r.read_new()
    r.mem[WIN2] = _win(20) + ["[你] one"]            # WIN2 較新
    assert r.read_new() == ["[你] one"]
    # WIN 仍是舊的(倒退)→ 不該再冒 one,也不冒 WIN 的舊尾巴
    for _ in range(3):
        assert r.read_new() == []
