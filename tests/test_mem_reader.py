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


def test_groups_in_blob_drops_single_marker_group():
    blob = u16(_say("[A] alone"))
    assert groups_in_blob(blob) == []


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
class FakeLive(LiveChatReader):
    """以假記憶體(dict: addr -> lines)取代真實掃描。"""

    def __init__(self):
        super().__init__()
        self.mem: dict[int, list[str]] = {}
        self.scans = 0
        self.polls = 0

    def _open(self):
        return 1

    def _close(self, handle):
        pass

    def _scan_groups(self, h):
        self.scans += 1
        for addr, lines in self.mem.items():
            yield addr, len(lines) * 100, tuple(lines)

    def _poll_groups(self, h):
        self.polls += 1
        out = []
        for addr, lines in self.mem.items():
            if addr == self._addr:  # 模擬「讀錨點附近」:只有錨點位址在讀取範圍內
                out.append((addr, len(lines) * 100, list(lines)))
        return out


DOC = 0x1000
SNAPSHOT = 0x9000


def test_discovery_anchors_to_growing_doc_and_emits_growth():
    r = FakeLive()
    r.mem = {DOC: ["[A] a", "[B] b"], SNAPSHOT: ["[Z] old", "[Z] older"]}
    assert r.read_new() == []            # 第一次全掃:只建快照
    r.mem[DOC] = ["[A] a", "[B] b", "[C] c"]   # 活文件成長;快照靜止
    assert r.read_new() == ["[C] c"]     # 第二次全掃:偵測成長、定錨、補翻
    assert r._addr == DOC


def test_anchored_poll_emits_appended_including_repeats():
    r = FakeLive()
    r.mem = {DOC: ["[A] hi"] * 2}
    r.read_new()
    r.mem[DOC] = ["[A] hi"] * 3
    assert r.read_new() == ["[A] hi"]    # 定錨(成長 1 行)
    r.mem[DOC] = ["[A] hi"] * 5
    assert r.read_new() == ["[A] hi", "[A] hi"]  # 已定錨輪詢:重複照實回報
    assert r.scans == 2                  # 之後不再全掃
    assert r.polls == 1


def test_static_snapshots_never_emit():
    r = FakeLive()
    r.mem = {SNAPSHOT: ["[Z] old", "[Z] older"]}
    assert r.read_new() == []
    assert r.read_new() == []            # 快照靜止 → 永不定錨、永不輸出


def test_doc_relocation_reanchors_without_loss_or_duplication():
    r = FakeLive()
    r.mem = {DOC: ["[A] a", "[B] b"]}
    r.read_new()
    r.mem[DOC] = ["[A] a", "[B] b", "[C] c"]
    assert r.read_new() == ["[C] c"]     # 定錨
    # 文件被搬到新位址(舊位址消失),且搬家期間又多了兩行
    del r.mem[DOC]
    new_addr = 0x5000
    r.mem[new_addr] = ["[A] a", "[B] b", "[C] c", "[D] d", "[E] e"]
    assert r.read_new() == []            # 對不齊 1
    assert r.read_new() == []            # 對不齊 2 → 解除定錨
    assert r._addr == 0
    assert r.read_new() == []            # 探索全掃 1(建快照)
    r.mem[new_addr] = r.mem[new_addr] + ["[F] f"]  # 新位址繼續成長
    got = r.read_new()                   # 探索全掃 2:定錨新位址,從已知尾行後補翻
    assert got == ["[D] d", "[E] e", "[F] f"]
    assert r._addr == new_addr


def test_head_trim_scroll_absorbed():
    r = FakeLive()
    r.mem = {DOC: ["[A] a", "[B] b", "[C] c"]}
    r.read_new()
    r.mem[DOC] = ["[A] a", "[B] b", "[C] c", "[D] d"]
    assert r.read_new() == ["[D] d"]
    # 達容量上限:頭部被修剪 + 尾端附加(捲動)
    r.mem[DOC] = ["[C] c", "[D] d", "[E] e"]
    assert r.read_new() == ["[E] e"]


# --- 遊戲表情符號(內嵌 <image;Emoticons/..> 標記 → 保留成 :名稱:) ---
def test_clean_keeps_emoticon_as_name():
    raw = ("<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> "
           "<link;GID:123,Lars,2>[Lars]</link> fire first then "
           "<image;Emoticons/Laughter001.dds;24;24;FFFFFFFF> </color>")
    assert clean(raw) == "[Lars] fire first then :laughter:"


def test_clean_strips_emoticon_prefix_and_digits():
    assert clean("<image;Emoticons/Emoticons_Heart.dds;24;24;FFFFFFFF>") == ":heart:"
    assert clean("<image;Emoticons/Emoticons_School_Death.dds;24;24;FFFFFFFF>") == ":school_death:"
    assert clean("<image;Emoticons/Eyes001.dds;24;24;FFFFFFFF>") == ":eyes:"
    assert clean("<image;Emoticons/TeaCup001.dds;24;24;FFFFFFFF>") == ":teacup:"


def test_extract_keeps_line_with_emoticon_name():
    line = ("<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [Lars] ty king "
            "<image;Emoticons/Emoticons_Heart.dds;24;24;FFFFFFFF> </color>")
    assert extract_lines(u16(line)) == ["[Lars] ty king :heart:"]


def test_extract_keeps_astral_emoji_text():
    # 玩家實際打的 Unicode emoji(非遊戲表情標記)照樣保留
    assert extract_lines(_wrap("[Amy] nice 😂👀")) == ["[Amy] nice 😂👀"]


def test_extract_torn_emoticon_dropped_but_line_kept():
    # 撕裂副本:表情名稱夾入雜字 → 丟棄該表情,行其餘內容保留
    torn = "[莫格瑞姆 霜冻] ok <image;Emoticons/E䍸儱牴耀cons_Laugh.dds;24;24;FFFFFFFF>"
    assert extract_lines(_wrap(torn)) == ["[莫格瑞姆 霜冻] ok"]


def test_extract_keeps_pure_emoticon_message():
    pure = "[Lars] <image;Emoticons/Emoticons_Wink.dds;24;24;FFFFFFFF>"
    assert extract_lines(_wrap(pure)) == ["[Lars] :wink:"]


def test_reanchor_across_entities_does_not_replay_emitted():
    # 重啟後換錨情境:先錨在 A(渲染快取)輸出了 m;A 消失、尾行在新文件裡對不上
    # → fallback 走「兩次全掃的差分」也會算出 m —— 必須被已輸出重疊裁剪掉,不能重播。
    r = FakeLive()
    r.mem = {DOC: ["[A] a", "[B] b"]}
    r.read_new()                                  # 探索全掃 1
    r.mem[DOC] = ["[A] a", "[B] b", "[M] m"]
    assert r.read_new() == ["[M] m"]              # 定錨 + 輸出 m
    del r.mem[DOC]                                # 錨點實體消失
    assert r.read_new() == []                     # 對不齊 1
    assert r.read_new() == []                     # 對不齊 2 → 解錨
    r._lines = ["[Z] not-in-any-doc"]             # 模擬跨實體行集合差異:尾行對不上
    new_addr = 0x7000
    r.mem[new_addr] = ["[Q] q"]
    assert r.read_new() == []                     # 探索全掃 1(建快照)
    r.mem[new_addr] = ["[Q] q", "[M] m"]          # 新實體的差分又是 m(其實是舊訊息)
    assert r.read_new() == []                     # 已輸出過 → 裁掉,不重播
    assert r._addr == new_addr                    # 但仍完成定錨
    r.mem[new_addr] = ["[Q] q", "[M] m", "[N] n"]
    assert r.read_new() == ["[N] n"]              # 之後的新訊息照常輸出


def test_anchored_poll_repeats_not_affected_by_emitted_guard():
    # 已定錨的正常輪詢不套用裁剪:真實的連續重複訊息要照實輸出
    r = FakeLive()
    r.mem = {DOC: ["[A] hi", "[A] hi"]}
    r.read_new()
    r.mem[DOC] = ["[A] hi"] * 3
    assert r.read_new() == ["[A] hi"]             # 定錨
    r.mem[DOC] = ["[A] hi"] * 4
    assert r.read_new() == ["[A] hi"]             # 又一則相同訊息 → 照常輸出
