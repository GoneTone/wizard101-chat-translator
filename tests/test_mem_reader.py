from collections import deque

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
    """以腳本化的 chatLog 全文序列取代 wizwalker I/O。"""

    def __init__(self, texts):
        super().__init__()
        self.texts = texts
        self.n = 0
        self._connected = True

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


def test_filter_resurfaced_keeps_repeats_beyond_retired_counts():
    # 多重集合語意：退役基準裡有幾份就最多剔幾份，真的又說了一樣的話要保留
    retired = deque([["[A] lol"]])
    emitted = [ChatLine("[A] lol", None), ChatLine("[A] lol", None),
               ChatLine("[B] new", None)]
    assert filter_resurfaced(emitted, retired) == [("[A] lol", None), ("[B] new", None)]


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


def test_hard_reset_emits_new_content():
    # 聊天被重置成全新內容（如 relog）→ 新內容視為新訊息輸出，之後正常延續
    r = FakeWiz([
        _log(_say(1, "A", "one")),
        _log(_say(9, "Z", "fresh")),                       # 對不齊 → 全新內容
        _log(_say(9, "Z", "fresh"), _say(9, "Z", "next")),
    ])
    assert r.read_new() == []
    assert _texts(r.read_new()) == ["[Z] fresh"]
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


def test_message_after_chat_cleared_is_emitted():
    # 有歷史 → 聊天被清空（relog） → 清空後第一句仍要抓到
    r = FakeWiz([
        _log(_say(1, "A", "old")),
        "",                                          # 清空
        _log(_say(1, "C", "fresh")),
    ])
    assert r.read_new() == []                        # 基準=[A old]
    assert r.read_new() == []                        # 清空 → 對不齊 → 重新同步（基準變空）
    assert _texts(r.read_new()) == ["[C] fresh"]


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
    tail = _say(9, "Z", "lol")
    old = _burst_lines(MAX_NEW_LINES_PER_POLL + 1)
    r = FakeWiz([
        [_log(_say(1, "A", "a"), tail), ""],
        [_log(tail, *old), ""],
    ])
    assert r.read_new() == []
    assert r.read_new() == []
    err = capsys.readouterr().err
    assert "implausible burst suppressed" in err
    assert f"sizes=[0, {MAX_NEW_LINES_PER_POLL + 2}]" in err


def test_burst_within_limit_still_emitted():
    # 上限是防洪水，不是限流：翻譯卡住時累積的正常批次仍要全數吐出
    from src.reader.mem_reader import MAX_NEW_LINES_PER_POLL
    old = _burst_lines(MAX_NEW_LINES_PER_POLL)
    r = FakeWiz([_log(_say(1, "A", "a")), _log(_say(1, "A", "a"), *old)])
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
