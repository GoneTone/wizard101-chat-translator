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

    def _poll_groups(self, h, addr):
        self.polls += 1
        if addr in self.mem:  # 模擬「讀該錨點附近」:只有該位址在讀取範圍內
            return [(addr, len(self.mem[addr]) * 100, list(self.mem[addr]))]
        return []


DOC = 0x1000
SNAPSHOT = 0x9000


def test_discovery_anchors_to_growing_doc_and_emits_growth():
    r = FakeLive()
    r.mem = {DOC: ["[A] a", "[B] b", "[C] c"], SNAPSHOT: ["[Z] old", "[Z] older"]}
    assert r.read_new() == []            # 第一次全掃:只建快照
    r.mem[DOC] = ["[A] a", "[B] b", "[C] c", "[D] d"]   # 活文件成長;快照靜止
    assert r.read_new() == ["[D] d"]     # 第二次全掃:偵測成長、定錨、補翻
    assert DOC in r._addrs


def test_anchored_poll_emits_appended_including_repeats():
    r = FakeLive()
    r.mem = {DOC: ["[A] hi"] * 3}
    r.read_new()
    r.mem[DOC] = ["[A] hi"] * 4
    assert r.read_new() == ["[A] hi"]    # 定錨(成長 1 行)
    r.mem[DOC] = ["[A] hi"] * 6
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
    r.mem = {DOC: ["[A] a", "[B] b", "[X] x"]}
    r.read_new()
    r.mem[DOC] = ["[A] a", "[B] b", "[X] x", "[C] c"]
    assert r.read_new() == ["[C] c"]     # 定錨
    # 文件被搬到新位址(舊位址消失),且搬家期間又多了兩行
    del r.mem[DOC]
    new_addr = 0x5000
    r.mem[new_addr] = ["[A] a", "[B] b", "[X] x", "[C] c", "[D] d", "[E] e"]
    assert r.read_new() == []            # 對不齊 1
    assert r.read_new() == []            # 對不齊 2 → 解除定錨
    assert r._addrs == {}
    assert r.read_new() == []            # 探索全掃 1(建快照)
    r.mem[new_addr] = r.mem[new_addr] + ["[F] f"]  # 新位址繼續成長
    got = r.read_new()                   # 探索全掃 2:定錨新位址,從已知尾行後補翻
    assert got == ["[D] d", "[E] e", "[F] f"]
    assert new_addr in r._addrs


def test_head_trim_scroll_absorbed():
    r = FakeLive()
    r.mem = {DOC: ["[A] a", "[B] b", "[C] c"]}
    r.read_new()
    r.mem[DOC] = ["[A] a", "[B] b", "[C] c", "[D] d"]
    assert r.read_new() == ["[D] d"]
    # 達容量上限:頭部被修剪 + 尾端附加(捲動)
    r.mem[DOC] = ["[C] c", "[D] d", "[E] e"]
    assert r.read_new() == ["[E] e"]


# --- 遊戲表情符號(內嵌 <image;Emoticons/..> 標記 → emoji) ---
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


def test_reanchor_across_entities_does_not_replay_emitted():
    # 重啟後換錨情境:先錨在 A(渲染快取)輸出了 m;A 消失、尾行在新文件裡對不上
    # → fallback 走「兩次全掃的差分」也會算出 m —— 必須被已輸出重疊裁剪掉,不能重播。
    r = FakeLive()
    r.mem = {DOC: ["[A] a", "[B] b", "[X] x"]}
    r.read_new()                                  # 探索全掃 1
    r.mem[DOC] = ["[A] a", "[B] b", "[X] x", "[M] m"]
    assert r.read_new() == ["[M] m"]              # 定錨 + 輸出 m
    del r.mem[DOC]                                # 錨點實體消失
    assert r.read_new() == []                     # 對不齊 1
    assert r.read_new() == []                     # 對不齊 2 → 解錨
    r._lines = ["[Z] not-in-any-doc"]             # 模擬跨實體行集合差異:尾行對不上
    new_addr = 0x7000
    r.mem[new_addr] = ["[Q] q", "[R] r", "[S] s"]
    assert r.read_new() == []                     # 探索全掃 1(建快照)
    r.mem[new_addr] = ["[Q] q", "[R] r", "[S] s", "[M] m"]  # 新實體的差分又是 m(舊訊息)
    assert r.read_new() == []                     # 已輸出過 → 裁掉,不重播
    assert new_addr in r._addrs                    # 但仍完成定錨
    r.mem[new_addr] = ["[Q] q", "[R] r", "[S] s", "[M] m", "[N] n"]
    assert r.read_new() == ["[N] n"]              # 之後的新訊息照常輸出


def test_anchored_poll_repeats_not_affected_by_emitted_guard():
    # 已定錨的正常輪詢不套用裁剪:真實的連續重複訊息要照實輸出
    r = FakeLive()
    r.mem = {DOC: ["[A] hi"] * 3}
    r.read_new()
    r.mem[DOC] = ["[A] hi"] * 4
    assert r.read_new() == ["[A] hi"]             # 定錨
    r.mem[DOC] = ["[A] hi"] * 5
    assert r.read_new() == ["[A] hi"]             # 又一則相同訊息 → 照常輸出


def test_fresh_chat_first_messages_all_translated():
    # 空聊天室:第一句出現(基準)、第二句到達 → 定錨並「連第一句一起」補翻
    r = FakeLive()
    r.mem = {}
    assert r.read_new() == []                       # 全掃 1:空
    r.mem[DOC] = ["[A] first"]
    assert r.read_new() == []                       # 新位址、無前次內容 → 還不能定錨
    r.mem[DOC] = ["[A] first", "[B] second"]
    assert r.read_new() == ["[A] first", "[B] second"]  # 成長 → 定錨,基準行一起補翻
    assert DOC in r._addrs
    r.mem[DOC] = ["[A] first", "[B] second", "[C] third"]
    assert r.read_new() == ["[C] third"]            # 之後正常輪詢


def test_relocated_long_doc_does_not_dump_baseline():
    # 搬移的舊文件(基準很長)不能把整份歷史當新訊息倒出來
    r = FakeLive()
    r.mem = {DOC: ["[A] a", "[B] b", "[C] c", "[D] d"]}
    r.read_new()                                    # 全掃 1(建快照)
    r.mem[DOC] = ["[A] a", "[B] b", "[C] c", "[D] d", "[E] e"]
    assert r.read_new() == ["[E] e"]                # 基準 4 行 > 門檻 → 只翻新增


SNAP2 = 0xB000


def test_first_message_in_empty_chat_translated_without_growth():
    # 需求:空聊天室的「第一句」不等第二句、下一次掃描就翻。
    # 真訊息會立刻被渲染成多份副本(文件 + 快照)→ 新文字出現在 >=2 個緩衝即翻。
    r = FakeLive()
    r.mem = {}
    assert r.read_new() == []                        # 全掃 1:空,建基準
    r.mem[DOC] = ["[A] first"]
    r.mem[SNAP2] = ["[A] first"]                     # 同訊息的渲染副本
    assert r.read_new() == ["[A] first"]             # 沒有成長也翻(新穎性)
    r.mem[DOC] = ["[A] first", "[B] second"]
    r.mem[SNAP2] = ["[A] first", "[B] second"]
    assert r.read_new() == ["[B] second"]            # 成長 → 定錨;first 不重複
    assert DOC in r._addrs


def test_single_copy_garbage_not_emitted_by_novelty():
    # 撕裂垃圾每份內容都不同,只會出現在 1 個緩衝 → 新穎性路徑不放行
    r = FakeLive()
    r.mem = {DOC: ["[A] a", "[B] b", "[C] c"]}
    r.read_new()                                     # 建基準
    r.mem[0xC000] = ["[Luc] torn�garbage line"]      # 單份新內容
    assert r.read_new() == []


TWIN = 0xD000


def test_twin_failover_keeps_messages_flowing():
    # 定錨到雙副本;其中一個變殭屍(靜止殘骸),另一個照常成長 → 訊息不中斷
    r = FakeLive()
    base = ["[A] a", "[B] b", "[C] c"]
    r.mem = {DOC: list(base), TWIN: list(base)}
    r.read_new()
    r.mem[DOC] = base + ["[D] d"]
    r.mem[TWIN] = base + ["[D] d"]
    assert r.read_new() == ["[D] d"]              # 定錨,雙副本都被記住
    assert DOC in r._addrs and TWIN in r._addrs
    r.mem[TWIN] = base + ["[D] d", "[E] e"]       # DOC 從此凍結成殭屍;TWIN 繼續活
    assert r.read_new() == ["[E] e"]              # 靠 TWIN 無縫供訊息
    r.mem[TWIN] = base + ["[D] d", "[E] e", "[F] f"]
    assert r.read_new() == ["[F] f"]


def test_idle_verify_recovers_from_zombie_anchor():
    # 「打字沒反應、狀態卻是監聽中」的場景:錨點全是殘骸,真文件已搬走且持續成長。
    # 閒置驗證掃描應當場重定錨並一次補翻漏掉的訊息。
    import src.reader.mem_reader as mr
    r = FakeLive()
    base = ["[A] a", "[B] b", "[C] c"]
    r.mem = {DOC: list(base)}
    r.read_new()
    r.mem[DOC] = base + ["[D] d"]
    assert r.read_new() == ["[D] d"]              # 定錨
    moved = 0xE000                                # 文件搬走並繼續成長;殘骸凍結在 DOC
    r.mem[moved] = base + ["[D] d", "[E] e", "[F] f"]
    for _ in range(mr._IDLE_RECHECK_POLLS - 1):
        assert r.read_new() == []                 # 殘骸靜止,一直「監聽中」沒反應
    assert r.read_new() == ["[E] e", "[F] f"]     # 閒置驗證:找到延伸內容 → 補翻
    assert moved in r._addrs and DOC not in r._addrs


# --- 插入式文件(依頻道分節、訊息插中段、尾端不動) ---
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


def test_insert_mode_doc_anchors_and_streams():
    # 安靜模式:沒有附加式文件,只有一份「中段插入」的聊天總文件 → 也要能定錨並即時翻
    r = FakeLive()
    base = ["[A] a", "[B] b", "[C] c", "[D] d", "[伊莱杰] np"]
    r.mem = {DOC: list(base)}
    r.read_new()                                       # 全掃 1:建快照
    r.mem[DOC] = base[:2] + ["[你] Test"] + base[2:]   # 訊息插中段,尾端不動
    assert r.read_new() == ["[你] Test"]               # 插入式定錨 + 補翻
    assert DOC in r._addrs
    cur = r.mem[DOC]
    r.mem[DOC] = cur[:3] + ["[你] zxqv123"] + cur[3:]  # 定錨後又一則中段插入
    assert r.read_new() == ["[你] zxqv123"]            # 輪詢用整份差分即時翻


def test_inserted_lines_reflow_or_move_not_new():
    from src.reader.mem_reader import inserted_lines
    # 分節文件重繪:整段搬動/重排,行數量不變 → 不是新訊息(冒舊訊息回歸測試)
    old = ["[A] a", "[B] b", "[C] c", "[D] d", "[E] e"]
    moved = ["[C] c", "[D] d", "[E] e", "[A] a", "[B] b"]
    assert inserted_lines(old, moved) == []


def test_insert_anchor_catchup_capped():
    # 停滯快取被改寫成現況:一次冒出大量「新增」= 累積的舊訊息 → 只定錨不輸出
    import src.reader.mem_reader as mr
    r = FakeLive()
    base = [f"[A] old{i}" for i in range(20)]
    r.mem = {DOC: list(base)}
    r.read_new()
    jumped = base + [f"[B] later{i}" for i in range(mr._MAX_INSERT_CATCHUP + 3)]
    jumped = jumped[:3] + jumped[20:] + jumped[3:20]  # 打亂使其走插入式路徑
    r.mem[DOC] = jumped
    assert r.read_new() == []          # 超過上限 → 不把舊訊息倒出來
    assert DOC in r._addrs             # 但已定錨重新同步
    cur = r.mem[DOC]
    r.mem[DOC] = cur[:5] + ["[你] new"] + cur[5:]
    assert r.read_new() == ["[你] new"]  # 之後正常


def test_discovery_prefers_append_doc_over_insert_doc():
    # 兩種文件同時成長時,優先錨定「附加式」(精確、支援重複)
    r = FakeLive()
    append_doc, insert_doc = 0x3000, 0x4000
    a = ["[A] a", "[B] b", "[C] c"]
    big = [f"[H] h{i}" for i in range(10)]
    r.mem = {append_doc: list(a), insert_doc: list(big)}
    r.read_new()
    r.mem[append_doc] = a + ["[M] m"]
    r.mem[insert_doc] = big[:5] + ["[M] m"] + big[5:]
    assert r.read_new() == ["[M] m"]
    assert append_doc in r._addrs and insert_doc not in r._addrs
