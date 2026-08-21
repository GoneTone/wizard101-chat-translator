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
class FakeLive(LiveChatReader):
    """以假記憶體(dict: addr -> lines)取代真實掃描,驗證尾指紋定位/搬家/殭屍。"""

    def __init__(self):
        super().__init__()
        self.mem: dict[int, list[str]] = {}
        self.full_scans = 0
        self.cached_scans = 0

    def _open(self):
        return 1

    def _close(self, handle):
        pass

    def _full_docs(self, h):
        self.full_scans += 1
        return [(a, len(l) * 100, list(l)) for a, l in self.mem.items()
                if len(l) >= 2]

    def _cached_docs(self, h):
        self.cached_scans += 1
        return [(a, len(self.mem[a]) * 100, list(self.mem[a]))
                for a in self._addrs if a in self.mem and len(self.mem[a]) >= 2]


DOC = 0x1000
TWIN = 0x2000
SNAPSHOT = 0x9000


def _hist(n, extra=()):
    return [f"[A] m{i}" for i in range(n)] + list(extra)


def test_startup_syncs_without_emitting_history():
    r = FakeLive()
    r.mem = {DOC: _hist(30)}
    assert r.read_new() == []            # 首次:既有歷史當基準,不翻
    assert r.anchored and r._counts["[A] m0"] == 1   # 基準已建立


def test_appended_message_emitted_including_repeats():
    r = FakeLive()
    r.mem = {DOC: _hist(30)}
    r.read_new()
    r.mem[DOC] = _hist(30) + ["[B] hi"]
    assert r.read_new() == ["[B] hi"]
    r.mem[DOC] = _hist(30) + ["[B] hi", "[B] hi"]     # 重複同一句
    assert r.read_new() == ["[B] hi"]
    r.mem[DOC] = _hist(30) + ["[B] hi", "[B] hi", "[B] hi"]
    assert r.read_new() == ["[B] hi"]                 # 每則重複都翻


def test_spam_same_line_many_times():
    r = FakeLive()
    r.mem = {DOC: ["[你] Test"] * 20}
    r.read_new()
    for _ in range(3):
        r.mem[DOC] = r.mem[DOC] + ["[你] Test"]
        assert r.read_new() == ["[你] Test"]          # 狂洗同一句也逐則偵測


def test_relocation_pingpong_tracks_new_only():
    # 雙緩衝乒乓:兩份副本都在,輪流被更新成最新;另一份殘留成殭屍。
    r = FakeLive()
    base = _hist(30)
    r.mem = {DOC: list(base), TWIN: list(base)}       # 雙緩衝
    r.read_new()
    r.mem[TWIN] = base + ["[B] one"]                  # TWIN 更新為最新;DOC 殭屍
    assert r.read_new() == ["[B] one"]
    r.mem[DOC] = base + ["[B] one", "[B] two"]        # DOC 更新為最新;TWIN 殭屍
    assert r.read_new() == ["[B] two"]                # 只翻新的,不因乒乓重播


def test_zombie_never_re_emits():
    # 殭屍(較舊副本)一直存在也不會冒舊訊息:多重集合差為空
    r = FakeLive()
    base = _hist(30)
    r.mem = {DOC: list(base), TWIN: list(base)}
    r.read_new()
    r.mem[TWIN] = base + ["[B] new"]                  # 活文件在 TWIN;DOC 殭屍
    assert r.read_new() == ["[B] new"]
    for _ in range(3):                                # 無新訊息 → 空,不重播殭屍舊內容
        assert r.read_new() == []


def test_cached_path_updates_and_full_scan_corrects(monkeypatch):
    import src.reader.mem_reader as mr
    monkeypatch.setattr(mr, "_FULL_EVERY_DOC", 3)
    r = FakeLive()
    base = _hist(30)
    r.mem = {DOC: list(base)}
    r.read_new()                                  # 全掃,快取 DOC
    r.mem[DOC] = base + ["[B] a"]
    assert r.read_new() == ["[B] a"]              # 快取路徑:同位址更新
    assert r.cached_scans >= 1
    # 搬到全新位址(快取讀不到);定期全掃(_FULL_EVERY_DOC=3 輪)會校正抓到
    fresh = 0x5000
    r.mem[fresh] = base + ["[B] a", "[B] b"]
    del r.mem[DOC]
    got = []
    for _ in range(4):
        got += r.read_new()
    assert got == ["[B] b"]                       # 全掃校正後補上,不漏不重
    assert fresh in r._addrs


def test_big_jump_syncs_without_dumping():
    # 一次冒出超過上限的行(對到殭屍後的大量舊內容)→ 只重新同步,不倒舊訊息
    r = FakeLive()
    r.mem = {DOC: _hist(30)}
    r.read_new()
    r.mem[DOC] = _hist(30) + [f"[B] x{i}" for i in range(mr_max_catchup() + 5)]
    assert r.read_new() == []                     # 超量 → 同步不輸出
    r.mem[DOC] = r.mem[DOC] + ["[C] after"]
    assert r.read_new() == ["[C] after"]          # 之後恢復正常


def mr_max_catchup():
    import src.reader.mem_reader as mr
    return mr._MAX_CATCHUP
