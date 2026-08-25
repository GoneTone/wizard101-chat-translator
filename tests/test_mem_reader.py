from src.reader.mem_reader import (
    ChatLine, GameNotRunning, WizChatReader, align_append, align_recover, clean,
    filter_resurfaced, lines_from_chatlog,
)


# --- clean：去標記 / 還原實體 / 表情 ---
def test_clean_strips_markup_and_collapses_space():
    assert clean("<color;FFFFFF><image;Art/x.dds;24;24;FFFFFFFF> [你] hi there </color>") == "[你] hi there"


def test_clean_unescapes_player_typed_entities():
    assert clean("<link;GID:1,H,2>[Han]</link> i hear you &gt; and more") == "[Han] i hear you > and more"
    assert clean("<link;GID:1,A,2>[Amy]</link> a &lt;3 b &amp; c") == "[Amy] a <3 b & c"


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


def _texts(lines):
    """只比對文字內容（顏色另有專門測試）。"""
    return [l.text for l in lines]


# --- lines_from_chatlog：從 chatLog 全文抽玩家發言 ---
def _say(gid: int, name: str, text: str) -> str:
    return (f"<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> "
            f"<link;GID:{gid},{name},2>[{name}]</link> {text} </color>")


def _system(text: str) -> str:
    return f"<color;00FF00><image;Art/Art_Chat_System.dds;24;24;FFFFFFFF> {text}</color>"


def _own(text: str) -> str:
    # 自己的發言：[你] 開頭，帶 Art_Chat 圖示但無 <link;GID>
    return f"<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [你] {text} </color>"


def test_lines_extracts_player_say_with_sender():
    assert _texts(lines_from_chatlog(_say(1, "Wolf", "hello world"))) == ["[Wolf] hello world"]


def test_lines_keeps_cjk_sender_and_spaces():
    log = _say(196751008724053815, "沃尔夫 亡灵骑兵", "wth")
    assert _texts(lines_from_chatlog(log)) == ["[沃尔夫 亡灵骑兵] wth"]


def test_lines_skips_system_messages():
    # 系統訊息（無 <link;GID>）不翻：掉寶/經驗/升等
    log = "\n".join([_say(1, "Amy", "hi"), _system("你獲得了 51 金幣！"),
                     _system("你現在等級 28！")])
    assert _texts(lines_from_chatlog(log)) == ["[Amy] hi"]


def test_lines_skips_debug_rows():
    # 除錯 chatLog 節點的行（[DBGL]/[STAT] 無 <link;GID>）不得誤入
    log = "\n".join(["[DBGL] HandleStatisticUpdate: new health 1783",
                     "[STAT] BuddyListManager::MSG_BuddyEntry Added",
                     _say(1, "Q", "back")])
    assert _texts(lines_from_chatlog(log)) == ["[Q] back"]


def test_lines_keeps_own_message():
    # 自己的發言（[你]，無 link）也要收
    assert _texts(lines_from_chatlog(_own("zztest123"))) == ["[你] zztest123"]
    assert _texts(lines_from_chatlog(_own("測試 訊息 :)"))) == ["[你] 測試 訊息 :)"]


def test_lines_keeps_own_and_others_together():
    log = "\n".join([_say(1, "Amy", "hi"), _own("我回你"), _system("你獲得了 51 金幣！"),
                     "[STAT] noise", _say(2, "Bob", "yo")])
    assert _texts(lines_from_chatlog(log)) == ["[Amy] hi", "[你] 我回你", "[Bob] yo"]


def test_lines_preserves_order_and_repeats():
    log = "\n".join([_say(1, "A", "hi"), _say(2, "B", "yo"), _say(1, "A", "hi")])
    assert _texts(lines_from_chatlog(log)) == ["[A] hi", "[B] yo", "[A] hi"]


def test_lines_keeps_emoticon_line():
    log = (f"<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> "
           f"<link;GID:1,Lars,2>[Lars]</link> ty king "
           f"<image;Emoticons/Emoticons_Heart.dds;24;24;FFFFFFFF> </color>")
    assert _texts(lines_from_chatlog(log)) == ["[Lars] ty king :heart:"]


def test_lines_keeps_astral_emoji_text():
    assert _texts(lines_from_chatlog(_say(1, "Amy", "nice 😂👀"))) == ["[Amy] nice 😂👀"]


def test_lines_empty_when_no_player_chat():
    assert lines_from_chatlog(_system("你獲得了 14 金幣！")) == []
    assert lines_from_chatlog("") == []


# --- 非 Art_Chat 圖示的玩家頻道（房間等）：chat_balloon_* 前綴 ---
def test_lines_keeps_house_channel_message_with_balloon_icon():
    # 房間頻道實測 markup：圖示為 chat_balloon_Owner（房主）、頻道色 FFFF00
    raw = ("<color;FFFF00><image;Art/chat_balloon_Owner.dds;24;24;FFFFFFFF> "
           "[你] Test1</color>")
    assert lines_from_chatlog(raw) == [("[你] Test1", "#ffff00")]


def test_lines_keeps_house_guest_variant():
    raw = ("<color;FFFF00><image;Art/chat_balloon_Guest.dds;24;24;FFFFFFFF> "
           "<link;GID:9,Amy,2>[Amy]</link> hi house</color>")
    assert lines_from_chatlog(raw) == [("[Amy] hi house", "#ffff00")]


def test_lines_skips_debug_rows_without_icon():
    # [DBGM]/[DBGL]/[STAT] 行無圖示：即使去標記後長得像聊天行也不得誤入
    log = "\n".join(["[DBGM] MSG_SendBlob 114 CurrentZone 114</color>",
                     "[DBGM] Loading housing blob: Type:Garden Objects:1</color>"])
    assert lines_from_chatlog(log) == []


# --- 行帶遊戲顏色：<color;RRGGBB> 解析成 ChatLine.color，供 overlay 對齊遊戲顯示色 ---
def test_lines_carry_game_color():
    line, = lines_from_chatlog(_say(1, "Wolf", "hello world"))
    assert line == ("[Wolf] hello world", "#ffffff")
    assert line.text == "[Wolf] hello world"
    assert line.color == "#ffffff"


def test_lines_color_normalized_lowercase_hex():
    raw = "<color;80FF00><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [你] hi </color>"
    assert lines_from_chatlog(raw) == [("[你] hi", "#80ff00")]


def test_lines_color_takes_last_six_of_eight_digit_hex():
    # 帶 alpha 的 AARRGGBB 形式：只取後 6 位當顯示色
    raw = "<color;FF80FF00><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [你] hi </color>"
    assert lines_from_chatlog(raw) == [("[你] hi", "#80ff00")]


def test_lines_color_missing_is_none():
    raw = "<image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [你] hi"
    assert lines_from_chatlog(raw) == [("[你] hi", None)]


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


def test_align_torn_middle_recovers_tail():
    # 撕裂讀取：cur 中段缺一行，恢復路徑以更短的尾段對齊、只回傳其後的新行
    assert align_append(["a", "b", "c", "d"], ["a", "b", "d", "e"]) is None
    assert align_recover(["a", "b", "c", "d"], ["a", "b", "d", "e"]) == ["e"]


def test_align_mid_position_overlap_recovers():
    # 重疊片段不在 cur 開頭（如多控件串接結構變化）也要能對齊，不整份重吐
    assert align_recover(["a", "b"], ["x", "a", "b", "c"]) == ["c"]


def test_align_recover_anchors_at_last_occurrence():
    # 聊天常見重複行（lol/gg）：恢復路徑必須錨定「最後」一次出現，
    # 錨到較早的重複行會把其後整段舊訊息當新行重吐（洪水）
    assert align_recover(["z", "x"], ["x", "old1", "old2", "x", "new"]) == ["new"]


def test_align_recover_none_when_no_overlap():
    assert align_recover(["a", "b"], ["x", "y"]) is None
    assert align_recover([], ["a"]) is None


# --- WizChatReader：以假 chatLog 文字驗證差分流程（不需遊戲） ---
class FakeWiz(WizChatReader):
    """以腳本化的 chatLog 全文序列取代 wizwalker I/O。inputs＝每輪輸入框開關狀態。"""

    def __init__(self, texts, inputs=None):
        super().__init__()
        self.texts = texts
        self.inputs = list(inputs or [])
        self.n = 0
        self._connected = True

    def input_open(self):
        return self.inputs.pop(0) if self.inputs else False

    def _grab_texts(self) -> list[str]:
        i = min(self.n, len(self.texts) - 1)
        self.n += 1
        t = self.texts[i]
        if isinstance(t, Exception):
            raise t
        return t if isinstance(t, list) else [t]  # 腳本給 list＝多個 chatLog 節點


def _log(*lines: str) -> str:
    return "\n".join(lines)


def _say_colored(color: str, name: str, text: str) -> str:
    return (f"<color;{color}><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> "
            f"<link;GID:1,{name},2>[{name}]</link> {text} </color>")


def test_channel_switch_recolor_does_not_reemit():
    # 切頻道時 chatLog 會把同樣的訊息以該頻道的 <color;..> 重新染色：
    # 對齊只看文字，重染色不得被判成「無重疊 → reset」而重吐舊訊息（重複翻譯）
    r = FakeWiz([
        _log(_say_colored("FFFFFF", "A", "hi"), _say_colored("FFFFFF", "B", "yo")),
        _log(_say_colored("8080FF", "A", "hi"), _say_colored("8080FF", "B", "yo")),
        _log(_say_colored("8080FF", "A", "hi"), _say_colored("8080FF", "B", "yo"),
             _say_colored("8080FF", "B", "new")),
    ])
    assert r.read_new() == []              # 基準
    assert r.read_new() == []              # 只是重染色：不得重吐
    emitted = r.read_new()
    assert _texts(emitted) == ["[B] new"]
    assert emitted[0].color == "#8080ff"   # 新行帶「當前讀到」的顏色


def test_filtered_view_flap_does_not_reemit_history():
    # 分頁過濾視圖與完整緩衝共用同一控件輪流出現（實測）：來回翻動不得重吐歷史。
    # f1/f2 是朋友分頁訊息，完整視圖裡它們中間、之後夾著別的訊息。
    full = _log(_say(1, "A", "m1"), _say(2, "F", "f1"), _say(1, "A", "m2"),
                _say(2, "F", "f2"), _say(1, "A", "m3"), _say(1, "A", "m4"))
    friend = _log(_say(2, "F", "f1"), _say(2, "F", "f2"))
    r = FakeWiz([full, friend, full, friend, full])
    assert r.read_new() == []          # 基準＝完整視圖
    assert r.read_new() == []          # 切到朋友分頁：內容是舊訊息子集，不得重吐
    assert r.read_new() == []          # 切回完整視圖：中段錨定後的尾巴全是舊歷史，不得重吐
    assert r.read_new() == []
    assert r.read_new() == []


def test_new_message_during_view_flap_is_emitted():
    # 視圖翻動期間夾帶的真實新訊息要照吐，只剔除退役基準解釋得了的舊行
    full = _log(_say(1, "A", "m1"), _say(2, "F", "f1"), _say(1, "A", "m2"))
    friend = _log(_say(2, "F", "f1"))
    full2 = _log(_say(1, "A", "m1"), _say(2, "F", "f1"), _say(1, "A", "m2"),
                 _say(1, "A", "m_new"))
    r = FakeWiz([full, friend, full2])
    assert r.read_new() == []
    assert r.read_new() == []              # 朋友分頁：舊子集
    assert _texts(r.read_new()) == ["[A] m_new"]   # 切回時只吐真正的新行


def test_message_sent_while_on_filtered_view_is_emitted():
    # 停在朋友分頁時送出的新訊息：從朋友視圖的 append 快路徑照常吐出
    full = _log(_say(1, "A", "m1"), _say(2, "F", "f1"))
    friend = _log(_say(2, "F", "f1"))
    friend2 = _log(_say(2, "F", "f1"), _say(2, "F", "f_new"))
    r = FakeWiz([full, friend, friend2])
    assert r.read_new() == []
    assert r.read_new() == []
    assert _texts(r.read_new()) == ["[F] f_new"]


def test_filter_resurfaced_drops_seen_and_keeps_new():
    seen = {"[A] lol"}
    emitted = [ChatLine("[A] lol", None), ChatLine("[B] new", None)]
    assert filter_resurfaced(emitted, seen) == [("[B] new", None)]


def test_suppression_survives_stable_stretch_on_one_view():
    # 在同一視圖停留多輪後才切換：抑制證據不得被輪替沖掉（退役佇列會、看過集合不會）
    full = _log(_say(1, "A", "m1"), _say(2, "F", "f1"), _say(1, "A", "m2"),
                _say(2, "F", "f2"), _say(1, "A", "m3"))
    friend = _log(_say(2, "F", "f1"), _say(2, "F", "f2"))
    r = FakeWiz([full] + [friend] * 6 + [full, friend, full])
    for _ in range(10):
        assert r.read_new() == []


def test_first_visit_to_unseen_view_absorbs_history_without_emitting():
    # 啟動盲區：App 啟動時基準只蓋到當前分頁；首次切到另一個分頁時，
    # 該視圖的整份歷史對看過集合是生面孔，reset 不得將其當新訊息輸出
    house = _log(_say_colored("FFFF00", "H", "h1"), _say_colored("FFFF00", "H", "h2"))
    main = _log(_say(1, "A", "m1"), _say(2, "B", "m2"), _say(1, "A", "m3"))
    main2 = _log(_say(1, "A", "m1"), _say(2, "B", "m2"), _say(1, "A", "m3"),
                 _say(2, "B", "m_new"))
    r = FakeWiz([house, main, main2])
    assert r.read_new() == []                      # 基準＝房間視圖
    assert r.read_new() == []                      # 首次切到主視圖：歷史吸收、不吐
    assert _texts(r.read_new()) == ["[B] m_new"]   # 之後的新訊息照常


def test_prefix_coincidence_append_burst_is_absorbed():
    # 實機 811→812：朋友視圖 [Test] 恰為主視圖 [Test, ...] 的前綴，append 對齊「成功」
    # 把主視圖其餘舊行當新增吐出。基準 1 行冒出 4 行＝不可能的人為速度 → 吸收
    friend = _log(_own("Test"))
    main = _log(_own("Test"), _say(1, "A", "m1"), _say(2, "B", "m2"),
                _say(1, "A", "m3"), _say(2, "B", "m4"))
    main2 = _log(_own("Test"), _say(1, "A", "m1"), _say(2, "B", "m2"),
                 _say(1, "A", "m3"), _say(2, "B", "m4"), _say(1, "A", "m_new"))
    r = FakeWiz([friend, main, main2])
    assert r.read_new() == []                      # 基準＝朋友視圖（1 行）
    assert r.read_new() == []                      # 前綴巧合暴增：吸收、不吐
    assert _texts(r.read_new()) == ["[A] m_new"]   # 之後新訊息照常


def test_all_seen_append_is_absorbed_as_view_resurface():
    # 視圖 A ⊃ 視圖 B（B 為 A 的前綴子集）：B→A 的 append 吐出的行全在看過集合 → 吸收
    full = _log(_own("f1"), _say(1, "A", "m1"), _say(2, "B", "m2"))
    sub = _log(_own("f1"))
    r = FakeWiz([full, sub, full, sub, full])
    assert r.read_new() == []          # 基準＝完整視圖（m1/m2 進看過集合）
    for _ in range(4):
        assert r.read_new() == []      # 子集⇄完整來回：append 巧合全在集合裡，不吐


def test_small_legit_burst_with_normal_baseline_still_emitted():
    # 正常聊天流：基準夠長時的小批新增（未見過）照常吐出，不受防線影響
    base = [_say(i, f"P{i}", f"line {i}") for i in range(8)]
    r = FakeWiz([_log(*base),
                 _log(*base, _say(9, "X", "burst1"), _say(9, "Y", "burst2"),
                      _say(9, "Z", "burst3"))])
    assert r.read_new() == []
    assert _texts(r.read_new()) == ["[X] burst1", "[Y] burst2", "[Z] burst3"]


def test_single_repeat_append_still_emitted():
    # 單行 append 永不過濾：重複的 lol/gg 是正常聊天，必須照吐
    r = FakeWiz([_log(_say(1, "A", "lol")),
                 _log(_say(1, "A", "lol"), _say(1, "A", "lol"))])
    assert r.read_new() == []
    assert _texts(r.read_new()) == ["[A] lol"]


def test_new_message_in_replacing_view_is_emitted_after_warmup():
    # 實機 869：朋友視圖是單行「置換式」——每句新話取代整個視圖內容、與基準零重疊。
    # 暖機期過後，reset 路徑必須吐出沒見過的行（真新訊息），不得一律吸收
    main = _log(_say(1, "A", "m1"), _say(2, "B", "m2"))
    reads = [main] * 12 + [_log(_own("Test1")), _log(_own("123"))]
    r = FakeWiz(reads)
    for _ in range(12):
        assert r.read_new() == []                  # 基準＋暖機期（無新訊息）
    assert _texts(r.read_new()) == ["[你] Test1"]  # 置換視圖的新話要翻
    assert _texts(r.read_new()) == ["[你] 123"]    # 每一句都要翻


def test_relog_fresh_content_emitted_after_warmup():
    # relog 成全新內容：暖機期過後未見過的行照吐（回復 absorb 改動前的行為）
    reads = [_log(_say(1, "A", "old"))] * 12 + [_log(_say(9, "Z", "fresh"))]
    r = FakeWiz(reads)
    for _ in range(12):
        assert r.read_new() == []
    assert _texts(r.read_new()) == ["[Z] fresh"]


def test_resurfaced_view_after_warmup_still_suppressed():
    # 暖機期過後切分頁：重浮的歷史仍要被看過集合剔除、不重翻
    full = _log(_say(1, "A", "m1"), _say(2, "F", "f1"), _say(1, "A", "m2"))
    friend = _log(_say(2, "F", "f1"))
    reads = [full] * 12 + [friend, full, friend]
    r = FakeWiz(reads)
    for _ in range(15):
        assert r.read_new() == []


def test_first_message_per_channel_with_empty_chat_after_game_restart():
    # 重開遊戲、聊天全空：暖機輪數必須含空讀（否則永不過期），
    # 各頻道（視圖從空白冒出第一句）的第一句話都要翻
    reads = [""] * 12 + [_log(_own("hello a")), _log(_own("hello b"))]
    r = FakeWiz(reads)
    for _ in range(12):
        assert r.read_new() == []                    # 空聊天：基準空、暖機隨輪數過期
    assert _texts(r.read_new()) == ["[你] hello a"]  # 第一個頻道的第一句
    assert _texts(r.read_new()) == ["[你] hello b"]  # 另一頻道視圖的第一句


def test_reconnect_resets_session_state():
    # 遊戲重開（斷線重連）：基準/看過集合屬於上個 session，必須歸零——
    # 否則新 session 第一句與舊訊息同字（Test 等常用字）會被誤判重浮而吞掉
    reads = ([_log(_own("Test"))] * 12
             + [RuntimeError("game closed")]
             + ["", _log(_own("Test"))])
    r = FakeWiz(reads)
    for _ in range(12):
        assert r.read_new() == []
    try:
        r.read_new()
        assert False, "expected GameNotRunning"
    except GameNotRunning:
        pass
    r._connected = True                          # 模擬重連成功
    assert r.read_new() == []                    # 新 session：空聊天重建基準
    assert _texts(r.read_new()) == ["[你] Test"]  # 同字的第一句不得被舊集合吞掉


def test_warmup_expires_within_five_polls():
    # 暖機縮短為 5 輪（0.4s poll 約 2 秒）：第 6 輪起 reset 的新訊息就要翻。
    # 啟動盲區實測都在前 1-3 輪（視圖每輪輪播、看過集合幾輪內學完），5 輪已保守
    main = _log(_say(1, "A", "m1"), _say(2, "B", "m2"))
    reads = [main] * 6 + [_log(_own("first after switch"))]
    r = FakeWiz(reads)
    for _ in range(6):
        assert r.read_new() == []
    assert _texts(r.read_new()) == ["[你] first after switch"]


def test_first_whisper_message_emitted_when_chat_ui_materializes():
    # 實機：掛入時 chatLog 尚不存在（nodes=0），開私訊送第一句時節點 0→2 出現，
    # 舊的節點變動路徑把它連同基準一起吞掉。基準為空時新節點內容要照吐
    r = FakeWiz([[], [_log(_own("whisper 1")), ""]])
    assert r.read_new() == []                        # 基準：0 節點、空
    assert _texts(r.read_new()) == ["[你] whisper 1"]


def test_node_added_after_warmup_emits_unseen_lines():
    # 暖機後開私訊視窗（節點增加）：新節點裡的新訊息要翻，主節點舊內容不重翻
    main = _log(_say(1, "A", "m1"), _say(2, "B", "m2"))
    reads = [[main]] * 7 + [[main, _log(_own("whisper 1"))]]
    r = FakeWiz(reads)
    for _ in range(7):
        assert r.read_new() == []
    assert _texts(r.read_new()) == ["[你] whisper 1"]


def test_node_added_with_only_seen_content_stays_silent():
    # 節點增加但新節點內容全是看過的行（重浮）：不得重翻
    main = _log(_say(1, "A", "m1"), _say(2, "F", "f1"))
    reads = [[main]] * 7 + [[main, _log(_say(2, "F", "f1"))]]
    r = FakeWiz(reads)
    for _ in range(8):
        assert r.read_new() == []


def test_node_removed_stays_silent_then_resumes():
    # 節點減少（關閉私訊視窗）：靜默重建，之後新訊息照常
    main = _log(_say(1, "A", "m1"))
    reads = [[main, _log(_own("w1"))], [main],
             [_log(_say(1, "A", "m1"), _say(1, "A", "m2"))]]
    r = FakeWiz(reads)
    assert r.read_new() == []                    # 基準（2 節點）
    assert r.read_new() == []                    # 節點減少：靜默
    assert _texts(r.read_new()) == ["[A] m2"]    # 之後照常


# --- 輸入框關聯放行：剛送出的訊息與近期舊訊息同字時不得被誤判重浮 ---
def test_same_text_message_after_relogin_emitted_when_input_just_closed():
    # 實機：登出再登入不重啟程序，看過集合殘留舊 session 字樣；登入後在朋友視窗
    # 重打同一句（Test）走 reset 被誤判重浮吞掉。輸入框剛關閉＝使用者剛送出，
    # 視圖尾行同字者關聯放行
    main = _log(_say(1, "A", "m1"), _own("Test"), _say(2, "B", "m2"))
    reads = [main] * 7 + [""] * 20 + [_log(_own("Test"))]
    inputs = [False] * 26 + [True, False]      # 送出前一輪輸入框開啟
    r = FakeWiz(reads, inputs)
    for _ in range(27):
        assert r.read_new() == []
    assert _texts(r.read_new()) == ["[你] Test"]


def test_resurfaced_view_without_input_activity_stays_suppressed():
    # 沒有輸入框活動的重浮（切分頁/視圖還原）：照樣攔住，不因放行機制而重翻
    full = _log(_say(1, "A", "m1"), _say(2, "F", "f1"), _say(1, "A", "m2"))
    friend = _log(_say(2, "F", "f1"))
    reads = [full] * 7 + [friend, full, friend]
    r = FakeWiz(reads)
    for _ in range(10):
        assert r.read_new() == []


def test_correlated_release_only_frees_the_tail_line():
    # 輸入框剛關閉時發生的視圖重浮：只放行尾行（剛送出的那句），其餘舊行照攔
    full = _log(_say(1, "A", "m1"), _own("gg"), _say(1, "A", "m2"))
    reads = [full] * 7 + [""] * 20 + [_log(_say(1, "A", "m1"), _own("gg"))]
    inputs = [False] * 26 + [True, False]
    r = FakeWiz(reads, inputs)
    for _ in range(27):
        assert r.read_new() == []
    assert _texts(r.read_new()) == ["[你] gg"]   # 尾行放行；[A] m1 仍被攔


def test_same_tail_text_after_long_empty_emitted_with_input():
    # 實機：登出前最後一句與登入後第一句同字（Test）——新內容恰等於舊基準尾行，
    # append 誤判「無變化」靜默吞掉。長時間全空後基準視為過期、強制走 reset 語意，
    # 由看過集合＋輸入框關聯放行接手
    main = _log(_say(1, "A", "m1"), _own("Test"))    # 登出前尾行＝Test
    reads = [main] * 7 + [""] * 20 + [_log(_own("Test"))]
    inputs = [False] * 26 + [True, False]
    r = FakeWiz(reads, inputs)
    for _ in range(27):
        assert r.read_new() == []
    assert _texts(r.read_new()) == ["[你] Test"]


def test_refill_after_long_empty_stays_silent():
    # 長時間全空後灌回同樣歷史（超長載入）：全在看過集合，不得重翻
    main = _log(_say(1, "A", "m1"), _say(2, "B", "m2"))
    reads = [main] * 7 + [""] * 20 + [main, main]
    r = FakeWiz(reads)
    for _ in range(len(reads)):
        assert r.read_new() == []


def test_same_text_resend_after_short_empty_is_emitted():
    # 置換式視圖（朋友/私訊視窗只顯示最近一則）重打同一句：新內容與舊基準完全相同，
    # align_append 會判「無變化」靜默吞掉。短暫清空即視基準過期 → 走 reset＋輸入框放行
    reads = [_log(_own("Hi"))] * 8 + ["", ""] + [_log(_own("Hi"))]
    inputs = [False] * 9 + [True, False]
    r = FakeWiz(reads, inputs)
    for _ in range(10):
        assert r.read_new() == []
    assert _texts(r.read_new()) == ["[你] Hi"]


def test_short_empty_refill_still_not_retranslated():
    # 門檻降低後轉場短暫清空也走 reset：填回同樣歷史仍不得重譯（看過集合擋下）
    main = _log(_say(1, "A", "m1"), _say(2, "B", "m2"))
    reads = [main] * 8 + ["", ""] + [main, main]
    r = FakeWiz(reads)
    for _ in range(len(reads)):
        assert r.read_new() == []


def test_first_read_skips_history():
    r = FakeWiz([_log(_say(1, "A", "old1"), _say(1, "A", "old2"))])
    assert r.read_new() == []          # 首次：記錄現況，不回吐既有歷史


def test_appended_lines_emitted():
    r = FakeWiz([
        _log(_say(1, "A", "one")),
        _log(_say(1, "A", "one"), _say(2, "B", "two"), _say(2, "B", "three")),
    ])
    assert r.read_new() == []
    assert _texts(r.read_new()) == ["[B] two", "[B] three"]


def test_repeats_preserved_across_polls():
    r = FakeWiz([
        _log(_say(1, "A", "hi")),
        _log(_say(1, "A", "hi"), _say(1, "A", "hi")),
        _log(_say(1, "A", "hi"), _say(1, "A", "hi"), _say(1, "A", "hi")),
    ])
    assert r.read_new() == []
    assert _texts(r.read_new()) == ["[A] hi"]
    assert _texts(r.read_new()) == ["[A] hi"]


def test_head_trim_scroll_absorbed():
    r = FakeWiz([
        _log(_say(1, "A", "a"), _say(1, "B", "b"), _say(1, "C", "c")),
        _log(_say(1, "B", "b"), _say(1, "C", "c"), _say(1, "D", "d")),
    ])
    assert r.read_new() == []
    assert _texts(r.read_new()) == ["[D] d"]   # 頭部修剪 + 尾端附加（捲動）只吐新行


def test_own_message_emitted():
    r = FakeWiz([
        _log(_say(1, "A", "hi")),
        _log(_say(1, "A", "hi"), _own("me too"), _say(2, "B", "yo")),
    ])
    assert r.read_new() == []
    assert _texts(r.read_new()) == ["[你] me too", "[B] yo"]


def test_system_and_debug_never_emitted():
    r = FakeWiz([
        _log(_say(1, "A", "hi")),
        _log(_say(1, "A", "hi"), _system("你獲得了 51 金幣！"),
             "[DBGL] noise", _say(2, "B", "real")),
    ])
    assert r.read_new() == []
    assert _texts(r.read_new()) == ["[B] real"]


def test_hard_reset_absorbs_then_resumes():
    # 聊天被重置成全新內容（relog／首次切到沒見過的分頁）→ 靜默重建基準不輸出
    # （取捨：relog 後第一批訊息不翻，換取切分頁不重翻整份歷史），之後正常延續
    r = FakeWiz([
        _log(_say(1, "A", "one")),
        _log(_say(9, "Z", "fresh")),                       # 對不齊 → 吸收為新基準
        _log(_say(9, "Z", "fresh"), _say(9, "Z", "next")),
    ])
    assert r.read_new() == []
    assert r.read_new() == []
    assert _texts(r.read_new()) == ["[Z] next"]


def test_input_open_caches_node_and_refinds_after_failure():
    import asyncio

    class FakeNode:
        def __init__(self):
            self.visible = True
            self.fail = False

        async def is_visible(self):
            if self.fail:
                raise RuntimeError("stale node")
            return self.visible

    class FakeRoot:
        def __init__(self, node):
            self.node = node
            self.searches = 0

        async def get_windows_with_name(self, name):
            self.searches += 1
            return [self.node]

    class FakeClient:
        def __init__(self, node):
            self.root_window = FakeRoot(node)

    node = FakeNode()
    r = WizChatReader()
    r._connected = True
    r._loop = asyncio.new_event_loop()
    r._client = FakeClient(node)
    try:
        assert r.input_open() is True
        node.visible = False
        assert r.input_open() is False
        assert r._client.root_window.searches == 1  # 節點已快取，不重搜整棵樹
        node.fail = True
        assert r.input_open() is False              # 節點失效 → 視為關閉
        node.fail = False
        node.visible = True
        assert r.input_open() is True               # 下一輪自動重找
        assert r._client.root_window.searches == 2
    finally:
        r._loop.close()


def test_input_open_false_when_not_connected():
    r = WizChatReader()
    assert r.input_open() is False


def test_node_count_change_rebaselines_without_emitting():
    # chatLog 節點數量變動（UI 事件生出/收掉控件）→ 串接結構改變無法歸因新舊：
    # 靜默重建基準、不回吐；之後恢復正常差分
    r = FakeWiz([
        [_log(_say(1, "A", "a"))],
        [_log(_say(1, "A", "a")), _log(_say(2, "B", "old"))],       # 節點 1→2
        [_log(_say(1, "A", "a")), _log(_say(2, "B", "old"), _say(3, "C", "new"))],
    ])
    assert r.read_new() == []          # 基準
    assert r.read_new() == []          # 節點數變動：重建基準、不吐 old
    assert _texts(r.read_new()) == ["[C] new"]  # 之後只吐真正的新行


def test_read_torn_middle_does_not_flood_old_lines():
    # 人多時撕裂讀取（全文中段壞掉一行）不可把整份舊訊息當成新訊息重翻（洪水）
    r = FakeWiz([
        _log(_say(1, "A", "a"), _say(2, "B", "b"), _say(3, "C", "c"), _say(4, "D", "d")),
        _log(_say(1, "A", "a"), _say(2, "B", "b"), _say(4, "D", "d")),  # 撕裂：缺 [C] c
        _log(_say(1, "A", "a"), _say(2, "B", "b"), _say(3, "C", "c"), _say(4, "D", "d"),
             _say(5, "E", "e")),                                        # 復原＋一行新訊息
    ])
    assert r.read_new() == []
    assert r.read_new() == []            # 撕裂輪：以尾段對回，不重吐舊行
    assert _texts(r.read_new()) == ["[E] e"]     # 復原後只吐真正的新行


def test_transient_empty_does_not_retranslate_on_refill():
    # 傳送/轉場時聊天暫態讀成空，之後又填回同樣歷史 → 不可重複翻譯
    hist = _log(_say(1, "A", "a"), _say(2, "B", "b"))
    r = FakeWiz([
        hist,                                              # sync 基準
        "",                                                # 轉場暫態：空讀
        hist,                                              # 填回同樣歷史
        _log(_say(1, "A", "a"), _say(2, "B", "b"), _say(3, "C", "c")),
    ])
    assert r.read_new() == []          # sync
    assert r.read_new() == []          # 空讀：忽略，保留基準
    assert r.read_new() == []          # 填回同樣歷史：不重譯
    assert _texts(r.read_new()) == ["[C] c"]   # 之後新訊息照常


def test_first_message_after_empty_chat_is_emitted():
    # 空聊天室（登入後）→ 第一句就要抓到，不能被當成初始歷史跳過
    r = FakeWiz([
        "",                                          # 空聊天室
        _log(_say(1, "A", "first")),                 # 第一句
        _log(_say(1, "A", "first"), _say(2, "B", "second")),
    ])
    assert r.read_new() == []                        # 首次：空基準
    assert _texts(r.read_new()) == ["[A] first"]
    assert _texts(r.read_new()) == ["[B] second"]


def test_message_after_chat_cleared_absorbs_first_then_resumes():
    # 有歷史 → 聊天被清空（relog）：清空後第一批吸收為新基準（見 reset 取捨），
    # 其後訊息照常輸出
    r = FakeWiz([
        _log(_say(1, "A", "old")),
        "",                                          # 清空（暫態：保留基準）
        _log(_say(1, "C", "fresh")),
        _log(_say(1, "C", "fresh"), _say(1, "D", "next")),
    ])
    assert r.read_new() == []                        # 基準=[A old]
    assert r.read_new() == []                        # 清空 → 忽略
    assert r.read_new() == []                        # 對不齊 → 吸收為新基準
    assert _texts(r.read_new()) == ["[D] next"]


def _burst_lines(n: int) -> list[str]:
    return [_say(2, "B", f"old{i}") for i in range(n)]


def test_implausible_burst_is_suppressed():
    # 實測情境：chatLog 在「約 110 行的短清單」與「上千行的完整歷史」之間反覆跳動，
    # 只要歷史開頭那行碰巧等於基準尾行（聊天充滿 lol/gg 等重複短行），align_append
    # 就會把整段舊訊息當成新訊息且不印 log。一輪 poll 只隔 poll_interval 秒，
    # 不可能新增這麼多行 → 一律不吐、靜默重建基準。
    from src.reader.mem_reader import MAX_NEW_LINES_PER_POLL
    tail = _say(9, "Z", "lol")
    old = _burst_lines(MAX_NEW_LINES_PER_POLL + 1)
    r = FakeWiz([
        _log(_say(1, "A", "a"), tail),
        _log(tail, *old),                      # 尾行對上歷史開頭 → 誤判整段為新訊息
        _log(tail, *old, _say(3, "C", "real")),
    ])
    assert r.read_new() == []
    assert r.read_new() == []                  # 洪水擋下
    assert _texts(r.read_new()) == ["[C] real"]        # 基準已重建，之後正常延續


def test_burst_log_reports_per_node_sizes(capsys):
    # 診斷 log 要看得出是哪個節點在灌入完整歷史（chatLog 有多個節點、內容量差一個數量級）
    from src.reader.mem_reader import MAX_NEW_LINES_PER_POLL
    base = [_say(1, "A", f"base{i}") for i in range(60)]
    old = _burst_lines(MAX_NEW_LINES_PER_POLL + 1)
    r = FakeWiz([
        [_log(*base), ""],
        [_log(*base, *old), ""],
    ])
    assert r.read_new() == []
    assert r.read_new() == []
    err = capsys.readouterr().err
    assert "implausible burst suppressed" in err
    assert f"sizes=[0, {60 + MAX_NEW_LINES_PER_POLL + 1}]" in err


def test_burst_within_limit_still_emitted():
    # 上限是防洪水，不是限流：基準已建立的聊天累積的正常大批次仍要全數吐出
    # （基準要夠長：極短基準冒出大批次會被 append 視圖切換防線吸收，屬預期）
    from src.reader.mem_reader import MAX_NEW_LINES_PER_POLL
    base = [_say(1, "A", f"base{i}") for i in range(60)]
    new_lines = _burst_lines(MAX_NEW_LINES_PER_POLL)
    r = FakeWiz([_log(*base), _log(*base, *new_lines)])
    assert r.read_new() == []
    assert len(r.read_new()) == MAX_NEW_LINES_PER_POLL


def test_read_failure_raises_game_not_running():
    r = FakeWiz([_log(_say(1, "A", "hi")), RuntimeError("process gone")])
    assert r.read_new() == []
    try:
        r.read_new()
        assert False, "應丟 GameNotRunning"
    except GameNotRunning:
        pass


def test_history_reappended_in_bulk_only_emits_unseen_lines():
    # 切伺服器轉場：chatLog 一輪內把整份歷史再接一次（實測 101 行→200 行）。
    # 這批絕大多數是看過的行，但只要混進一行沒讀過的，全有全無的「全部看過」
    # 判定就整批放行——大批次必須逐行過濾，只留真正沒見過的行。
    base = [_say(1, "Wolf", f"line {i}") for i in range(12)]
    doubled = base + base[:11] + [_say(1, "Wolf", "brand new line")]
    r = FakeWiz([_log(*base), _log(*doubled)])

    assert r.read_new() == []                  # 基準
    assert _texts(r.read_new()) == ["[Wolf] brand new line"]


def test_lines_keeps_quick_chat_word_balloon():
    # 快捷訊息（禁言帳號只能用選單發話）的圖示是 Art_Word_Balloon、link 末位為 0，
    # 其餘結構與一般發言相同；實機取樣，不得因圖示不在白名單被丟掉
    raw = ('<color;FFFFFF><image;Art/Art_Word_Balloon.dds;24;24;FFFFFFFF> '
           '<link;GID:196751008722541272,迈克尔,0>[迈克尔]</link> 不是</color>')
    assert _texts(lines_from_chatlog(raw)) == ["[迈克尔] 不是"]
