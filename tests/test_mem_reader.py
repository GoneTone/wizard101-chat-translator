import io

from src.reader.mem_reader import (
    RESET_WARMUP_POLLS, ChatLine, GameAccessDenied, GameNotRunning,
    GameVersionMismatch, WizChatReader, align_append, align_recover, clean,
    filter_resurfaced, is_version_mismatch, lines_from_chatlog, lines_from_nodes,
    player_out_with_idx,
)
from src.reader.message_log import MessageLog


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


def _texts_colors(lines):
    """只比對文字與遊戲顯示色。"""
    return [(l.text, l.color) for l in lines]


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


def test_lines_flags_system_messages():
    # 系統訊息（掉寶/經驗/升等）照收，但標上 system 旗標走另一條差分軌
    log = "\n".join([_say(1, "Amy", "hi"), _system("你獲得了 51 金幣！"),
                     _system("你現在等級 28！")])
    lines = lines_from_chatlog(log)
    assert _texts(lines) == ["[Amy] hi", "你獲得了 51 金幣！", "你現在等級 28！"]
    assert [l.system for l in lines] == [False, True, True]


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
    assert _texts(lines_from_chatlog(log)) == ["[Amy] hi", "[你] 我回你",
                                               "你獲得了 51 金幣！", "[Bob] yo"]


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


def test_lines_empty_for_empty_input():
    assert lines_from_chatlog("") == []


# --- 非 Art_Chat 圖示的玩家頻道（房間等）：chat_balloon_* 前綴 ---
def test_lines_keeps_house_channel_message_with_balloon_icon():
    # 房間頻道實測 markup：圖示為 chat_balloon_Owner（房主）、頻道色 FFFF00
    raw = ("<color;FFFF00><image;Art/chat_balloon_Owner.dds;24;24;FFFFFFFF> "
           "[你] Test1</color>")
    assert _texts_colors(lines_from_chatlog(raw)) == [("[你] Test1", "#ffff00")]


def test_lines_keeps_house_guest_variant():
    raw = ("<color;FFFF00><image;Art/chat_balloon_Guest.dds;24;24;FFFFFFFF> "
           "<link;GID:9,Amy,2>[Amy]</link> hi house</color>")
    assert _texts_colors(lines_from_chatlog(raw)) == [("[Amy] hi house", "#ffff00")]


def test_lines_skips_debug_rows_without_icon():
    # [DBGM]/[DBGL]/[STAT] 行無圖示：即使去標記後長得像聊天行也不得誤入
    log = "\n".join(["[DBGM] MSG_SendBlob 114 CurrentZone 114</color>",
                     "[DBGM] Loading housing blob: Type:Garden Objects:1</color>"])
    assert lines_from_chatlog(log) == []


# --- 系統訊息：照收但標上 system 旗標（走獨立差分軌，見 _diff_system_lines）---
def _system_colored(color: str, text: str) -> str:
    return (f"<color;{color}><image;Art/Art_Chat_System.dds;24;24;FFFFFFFF> "
            f"{text}</color>")


def test_system_lines_are_emitted_with_the_system_flag():
    lines = lines_from_chatlog(_system_colored("00FF00", "你获得了 39 金币！"))
    assert [l.text for l in lines] == ["你获得了 39 金币！"]
    assert lines[0].system is True
    assert lines[0].color == "#00ff00"


def test_player_lines_are_not_flagged_as_system():
    lines = lines_from_chatlog(_say_colored("FFFFFF", "Lars", "hi"))
    assert lines[0].system is False


def test_system_lines_do_not_need_a_sender_prefix():
    # 玩家行必須通過 _VALID 的 [發送者] 規則，系統行沒有前綴、不適用
    lines = lines_from_chatlog(_system_colored("AA00AA", "你获得了 3 经验值！"))
    assert [l.text for l in lines] == ["你获得了 3 经验值！"]


def test_system_lines_that_clean_to_nothing_are_dropped():
    assert lines_from_chatlog(_system_colored("00FF00", "")) == []


def test_debug_lines_are_still_dropped():
    # 除錯行沒有任何頻道圖示，不會因為放行系統訊息而混進來
    assert lines_from_chatlog("[DBGM] some debug noise") == []
    assert lines_from_chatlog("[WARN] another one") == []


# --- 伺服器公告：只有 <color;..>、連圖示都沒有，同樣走系統軌 ---
def _broadcast(color: str, text: str) -> str:
    return f"<color;{color}>{text}</color>"


def test_server_broadcast_without_any_icon_is_flagged_as_system():
    # 實機樣本：維修預告只有顏色標記，沒有任何 Art/ 圖示（app.log 也警告不到）
    lines = lines_from_chatlog(
        _broadcast("D9ABF8", "[Server Message] 服务器在2小时内关闭以进行维护"))
    assert [l.text for l in lines] == ["[Server Message] 服务器在2小时内关闭以进行维护"]
    assert lines[0].system is True
    assert lines[0].color == "#d9abf8"


def test_server_broadcast_does_not_need_a_sender_prefix():
    lines = lines_from_chatlog(_broadcast("D9ABF8", "服务器即将关闭"))
    assert [l.text for l in lines] == ["服务器即将关闭"]
    assert lines[0].system is True


def test_server_broadcast_that_cleans_to_nothing_is_dropped():
    assert lines_from_chatlog(_broadcast("D9ABF8", "")) == []


def test_iconless_lines_without_a_leading_color_tag_are_still_dropped():
    # 遊戲的除錯輸出同樣沒有圖示，但不以 <color;..> 起頭（實機樣本）
    assert lines_from_chatlog("RECEIVED STATUS UPDATE for [191965934165243834]") == []
    assert lines_from_chatlog("收到 [1688849928085385] 的伙伴邀请") == []


def test_lines_with_an_unknown_art_icon_are_not_taken_as_broadcasts():
    # 帶未知頻道圖示的行仍走 _warn_unknown_icon 丟棄，不會被誤收成伺服器公告
    raw = ("<color;FFFFFF><image;Art/Art_Unknown_Channel.dds;24;24;FFFFFFFF> "
           "[Lars] hi </color>")
    assert lines_from_chatlog(raw) == []


def test_system_and_player_lines_keep_their_in_game_order():
    raw = _log(_system_colored("00FF00", "你获得了 39 金币！"),
               _say_colored("FFFFFF", "Lars", "hi"),
               _system_colored("AA00AA", "你获得了 3 经验值！"))
    lines = lines_from_chatlog(raw)
    assert [l.text for l in lines] == ["你获得了 39 金币！", "[Lars] hi", "你获得了 3 经验值！"]
    assert [l.system for l in lines] == [True, False, True]


def test_mirror_detection_still_only_considers_player_lines():
    # 主視圖有玩家行＋系統行，副節點只鏡射玩家行 → 仍判定為鏡射並剔除
    main = _log(_say_colored("FFFFFF", "Lars", "hi"), _system_colored("00FF00", "你获得了 39 金币！"))
    mirror = _say_colored("FFFFFF", "Lars", "hi")
    lines, mirrored = lines_from_nodes([main, mirror])
    assert mirrored == 1
    assert [l.text for l in lines] == ["[Lars] hi", "你获得了 39 金币！"]


def test_player_index_mapping_skips_lines_filtered_out_of_the_tail():
    # 慢路徑會從尾段「中間」剔除重浮歷史，剩下的不再是連續尾段：
    # 若只取同長度的尾段索引，吐出的會是 b、c 而不是 a、c
    cur_all = lines_from_chatlog(_log(_say_colored("FFFFFF", "P", "a"),
                                      _system_colored("00FF00", "掉寶"),
                                      _say_colored("FFFFFF", "P", "b"),
                                      _say_colored("FFFFFF", "P", "c")))
    player_idx = [0, 2, 3]
    cur_player = [cur_all[i] for i in player_idx]
    assert player_out_with_idx([cur_player[0], cur_player[2]], cur_player, player_idx) == [0, 3]
    assert player_out_with_idx([], cur_player, player_idx) == []


def test_player_index_mapping_tells_repeated_lines_apart():
    # 重複行（lol／gg）文字一模一樣，只能以物件識別對回索引
    cur_all = lines_from_chatlog(_log(_say_colored("FFFFFF", "P", "lol"),
                                      _system_colored("00FF00", "掉寶"),
                                      _say_colored("FFFFFF", "P", "lol")))
    player_idx = [0, 2]
    cur_player = [cur_all[i] for i in player_idx]
    assert player_out_with_idx([cur_player[1]], cur_player, player_idx) == [2]


# --- 行帶遊戲顏色：<color;RRGGBB> 解析成 ChatLine.color，供 overlay 對齊遊戲顯示色 ---
def test_lines_carry_game_color():
    line, = lines_from_chatlog(_say(1, "Wolf", "hello world"))
    assert line.text == "[Wolf] hello world"
    assert line.color == "#ffffff"
    assert line.own is False   # 帶玩家連結＝別人講的


def test_lines_color_normalized_lowercase_hex():
    raw = "<color;80FF00><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [你] hi </color>"
    assert _texts_colors(lines_from_chatlog(raw)) == [("[你] hi", "#80ff00")]


def test_lines_color_takes_last_six_of_eight_digit_hex():
    # 帶 alpha 的 AARRGGBB 形式：只取後 6 位當顯示色
    raw = "<color;FF80FF00><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [你] hi </color>"
    assert _texts_colors(lines_from_chatlog(raw)) == [("[你] hi", "#80ff00")]


def test_lines_color_missing_is_none():
    raw = "<image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [你] hi"
    assert _texts_colors(lines_from_chatlog(raw)) == [("[你] hi", None)]


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

    def __init__(self, texts, inputs=None, message_log=None):
        super().__init__(message_log=message_log)
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
    assert _texts(filter_resurfaced(emitted, seen)) == ["[B] new"]


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


def test_single_line_view_refill_after_short_empty_is_not_retranslated():
    # 實機：聊天視圖裡只有一行玩家訊息，轉場清空後原樣填回——內容與基準一字不差，
    # 沒有新訊息可言。單行例外只適用於內容真的變了的情況，否則每次轉場都重譯尾行
    view = _log(_say(1, "A", "ill stay"))
    reads = [view] * 8 + ["", ""] + [view, view]
    r = FakeWiz(reads)
    for _ in range(len(reads)):
        assert r.read_new() == []


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


def test_single_line_view_after_transition_is_not_filtered_as_resurfaced():
    # 實機回報：聊天轉場空掉後收到私訊 "Test"，該行文字在啟動基準裡出現過，
    # 被 seen 過濾當成重浮歷史丟掉（第二次講同一句才走 append 而正常翻出）。
    # 歷史重浮一定是整份幾十行回來，視圖只有一行時必是新訊息，不得過濾。
    history = [_say(1, "Ann", "Test"), _say(2, "Bob", "hello"), _own("谢谢")]
    full = _log(*history)
    r = FakeWiz([full] * (1 + RESET_WARMUP_POLLS) + ["", "", _say(1, "Ann", "Test")])

    assert r.read_new() == []              # 基準（含舊的 Test）
    for _ in range(RESET_WARMUP_POLLS):    # 燒掉暖機輪數（實機早已過期）
        assert r.read_new() == []
    assert r.read_new() == []              # 轉場空讀
    assert r.read_new() == []              # 連續空讀 → 基準判定 stale
    assert _texts(r.read_new()) == ["[Ann] Test"]


def test_correlated_release_ignores_another_players_tail_line():
    # 轉場期間遊戲輸入框狀態會亂跳，關聯放行不能只看「輸入框剛開關過」：
    # 尾行是別人的發言（帶 <link;GID>）時放行等於重翻舊訊息（實機回報：
    # [摩根 灰烬行者] Have a good one Taylor 在轉場時重複出現）
    full = _log(_say(1, "A", "m1"), _own("gg"), _say(2, "Morgan", "Have a good one"))
    reads = [full] * 7 + [""] * 20 + [full]
    inputs = [False] * 26 + [True, False]
    r = FakeWiz(reads, inputs)
    for _ in range(28):
        assert r.read_new() == []


def test_lines_mark_own_message_without_player_link():
    # 「是不是自己講的」以有無玩家連結判斷：各語系的自稱用語不同，不比對名稱字串
    own, = lines_from_chatlog(_own("hello"))
    other, = lines_from_chatlog(_say(1, "Wolf", "hello"))
    assert own.own is True
    assert other.own is False


def test_new_message_survives_a_duplicated_chat_log_burst():
    # 實機回報：chatLog 短暫膨脹成重複版本（102 行歷史 ×8 ≈ 822 行），剛好在那一輪
    # 抵達的私訊被「暴增＝切到別的視圖」判定連同重複內容整批丟棄，對方講第二句才翻得出來。
    # 剔除看過的行之後只剩寥寥幾行時，那就是夾在重複內容裡的新訊息，必須照吐。
    history = [_say(1, "A", f"m{i}") for i in range(12)]
    duplicated = history * 4 + [_say(2, "B", "Test2")]
    r = FakeWiz([_log(*history), _log(*duplicated)])

    assert r.read_new() == []
    assert _texts(r.read_new()) == ["[B] Test2"]


# --- 鏡射節點：組隊／私訊浮動視窗把同一則訊息再渲染一份 ---
def test_lines_from_nodes_drops_a_mirrored_node():
    # 內容被主視圖完全涵蓋的節點＝鏡射視圖，不另計；主視圖沒有的行照收
    main = _log(_say(1, "A", "m1"), _own("hi"))
    lines, mirrored = lines_from_nodes([main, _log(_own("hi")), ""])
    assert _texts(lines) == ["[A] m1", "[你] hi"]
    assert mirrored == 1


def test_lines_from_nodes_keeps_a_node_with_its_own_content():
    # 私訊視窗有主視圖看不到的訊息：不是鏡射，兩個節點都要收
    main = _log(_say(1, "A", "m1"), _say(2, "B", "m2"))
    lines, mirrored = lines_from_nodes([main, _log(_own("whisper"))])
    assert _texts(lines) == ["[A] m1", "[B] m2", "[你] whisper"]
    assert mirrored == 0


def test_team_up_mirror_node_is_translated_once():
    # 實機 poll 43271：開組隊視窗後 chatLog 多一個節點（sizes=[1, 1, 0]），
    # 使用者發的同一句同時渲染在主視圖與組隊視窗，串接後一則訊息被翻兩次
    main = _log(_say(1, "A", "m1"), _say(2, "B", "m2"))
    sent = [_log(_own("hi")), _log(_own("hi")), ""]
    r = FakeWiz([[main, "", ""]] * (1 + RESET_WARMUP_POLLS) + [sent])
    for _ in range(1 + RESET_WARMUP_POLLS):
        assert r.read_new() == []
    assert _texts(r.read_new()) == ["[你] hi"]


def test_view_flap_beside_a_mirror_node_does_not_retranslate():
    # 實機 43280⇄43283：組隊視窗開著時主節點在「完整歷史」與「只剩剛送出那句」
    # 之間來回跳；鏡射多出來的那份會被 align_append 當成新增，每跳回來就重翻一次
    history = _log(_say(1, "A", "m1"), _say(2, "B", "m2"))
    small = [_log(_own("hi")), _log(_own("hi")), ""]
    full = [history, _log(_own("hi")), ""]
    r = FakeWiz([[history, "", ""]] * (1 + RESET_WARMUP_POLLS)
                + [small, full, small, full])
    for _ in range(1 + RESET_WARMUP_POLLS):
        assert r.read_new() == []
    assert _texts(r.read_new()) == ["[你] hi"]   # 剛送出：翻一次
    for _ in range(3):
        assert r.read_new() == []                # 視圖來回跳：不得重翻


# --- 系統訊息的差分軌：與玩家軌分離，由 emit_system 決定要不要輸出 ---
def test_system_and_player_lines_are_emitted_in_game_order():
    first = _say_colored("FFFFFF", "Lars", "hi")
    second = _log(first,
                  _system_colored("00FF00", "你获得了 39 金币！"),
                  _say_colored("FFFFFF", "Amy", "hey"),
                  _system_colored("AA00AA", "你获得了 3 经验值！"))
    r = FakeWiz([first, second])
    r.emit_system = True
    r.read_new()                      # 建立基準
    assert _texts(r.read_new()) == ["你获得了 39 金币！", "[Amy] hey", "你获得了 3 经验值！"]


def test_system_messages_do_not_disturb_the_player_baseline():
    # 一輪湧入大量系統訊息，夾在其中的玩家訊息仍須照吐
    base = _say_colored("FFFFFF", "Lars", "hi")
    flood = _log(base, *[_system_colored("00FF00", f"你获得了 {n} 金币！") for n in range(1, 15)],
                 _say_colored("FFFFFF", "Amy", "hey"))
    r = FakeWiz([base, flood])
    r.read_new()
    # 精確比對：emit_system 預設關閉，系統行一行都不該漏進玩家軌的輸出
    assert _texts(r.read_new()) == ["[Amy] hey"]


def test_a_poll_without_system_lines_does_not_stale_the_system_baseline():
    # 「這一輪沒有系統訊息」是日常狀態，不可拿它清掉系統軌基準：基準一沒了，
    # 下一輪就會被推去走 reset，而 reset 一律以看過集合過濾——再掉一次一字不差的
    # 同樣的寶（實機最常見的情形）就會被當成重浮歷史而整句吞掉。
    drop = _system_colored("00FF00", "你获得了 39 金币！")
    player = _say_colored("FFFFFF", "Lars", "hi")
    with_sys = _log(player, drop)
    no_sys = _log(player, _say_colored("FFFFFF", "Amy", "a"))
    dropped_again = _log(player, _say_colored("FFFFFF", "Amy", "a"), drop, drop)
    r = FakeWiz([with_sys, no_sys, no_sys, no_sys, dropped_again])
    r.emit_system = True
    r.read_new()                                    # 建立基準（含那一則掉寶）
    assert _texts(r.read_new()) == ["[Amy] a"]      # 這三輪一則系統訊息都沒有
    assert _texts(r.read_new()) == []
    assert _texts(r.read_new()) == []
    # 基準還在 → 只有「第二次」掉寶算新的：不吞掉，也不把第一次的重吐一遍
    assert _texts(r.read_new()) == ["你获得了 39 金币！"]


def test_system_lines_are_tracked_even_when_not_emitted():
    # 開關關閉時仍要跟蹤系統軌，之後打開才不會爆吐歷史（emit_system=False）
    first = _system_colored("00FF00", "你获得了 39 金币！")
    second = _log(first, _system_colored("00FF00", "你获得了 65 金币！"))
    r = FakeWiz([first, second, second])
    r.emit_system = False
    r.read_new()
    assert _texts(r.read_new()) == []      # 關閉時不吐
    r.emit_system = True
    assert _texts(r.read_new()) == []      # 打開後也不該把歷史倒出來


def test_system_track_keeps_emitting_after_being_re_enabled():
    first = _system_colored("00FF00", "你获得了 39 金币！")
    second = _log(first, _system_colored("00FF00", "你获得了 65 金币！"))
    third = _log(second, _system_colored("AA00AA", "你获得了 3 经验值！"))
    r = FakeWiz([first, second, third])
    r.emit_system = False
    r.read_new()
    r.read_new()
    r.emit_system = True
    assert _texts(r.read_new()) == ["你获得了 3 经验值！"]


def test_node_increase_does_not_re_emit_a_system_line_the_new_node_carries(capsys):
    # 實機：掉寶進行中開私訊／組隊視窗，chatLog 節點數 1->2。串接結構一變，對齊就會
    # 生出假的 append，而 append 路徑不過看過集合 → 剛掉過的那則寶再吐一次、再打一次
    # API。節點數是兩軌共同的事實：系統軌比照玩家軌走 reset 語意，交由看過集合擋下。
    drop = _system_colored("00FF00", "你获得了 39 金币！")
    main = _log(_say_colored("FFFFFF", "Lars", "hi"), drop)
    whisper = _log(_own("whisper 1"), drop)   # 新視窗把同一則掉寶也渲染了一份
    r = FakeWiz([[main]] * 7 + [[main, whisper]])
    r.emit_system = True
    for _ in range(7):
        assert r.read_new() == []             # 基準 ＋ 暖機期過完
    # 新節點裡的新玩家訊息照吐；已經看過的掉寶不得再吐一次
    assert _texts(r.read_new()) == ["[你] whisper 1"]
    assert ("system track handled as reset (chatLog node count increased)"
            in capsys.readouterr().err)


def test_merge_keeps_game_order_when_the_two_tracks_take_different_paths(capsys):
    # 雙軌設計的核心保證：兩軌各自走了哪條路徑都不影響相對順序（索引同源）。
    # 這裡讓玩家軌走 append、系統軌走 reset（頭部的舊掉寶被顯示上限修掉，
    # 與系統軌基準完全無重疊），輸出仍須與遊戲內的交錯順序一字不差。
    old_drop = _system_colored("00FF00", "你获得了 39 金币！")
    hi = _say_colored("FFFFFF", "Lars", "hi")
    gold = _system_colored("00FF00", "你获得了 65 金币！")
    hey = _say_colored("FFFFFF", "Amy", "hey")
    exp = _system_colored("AA00AA", "你获得了 88 经验值！")
    log = io.StringIO()
    r = FakeWiz([_log(old_drop, hi)] * 6 + [_log(hi, gold, hey, exp)],
                message_log=MessageLog(log))
    r.emit_system = True
    for _ in range(6):
        assert r.read_new() == []
    log.truncate(0)
    log.seek(0)
    emitted = r.read_new()
    assert "path=append" in log.getvalue()          # 玩家軌：hi 之後純附加
    assert ("system track handled as reset (no overlap with baseline)"
            in capsys.readouterr().err)             # 系統軌：與基準無重疊
    assert _texts(emitted) == ["你获得了 65 金币！", "[Amy] hey", "你获得了 88 经验值！"]


# --- messages.log：原始內容與判定結果落檔（見 src/reader/message_log.py）---
def test_message_log_records_raw_lines_and_what_was_emitted():
    buf = io.StringIO()
    system = "<color;FFFFFF><image;Art/Art_Chat_System.dds;24;24;FF> joined </color>"
    first = _say_colored("FF66CC", "Bob", "hi")
    second = _say_colored("FF66CC", "Amy", "yo")
    r = FakeWiz([_log(first, system), _log(first, system, second)],
                message_log=MessageLog(buf))
    r.read_new()   # 建立基準，不回吐
    assert _texts(r.read_new()) == ["[Amy] yo"]
    out = buf.getvalue()
    assert f"  RAW {first}" in out
    assert f"  RAW {system}" in out       # 系統訊息不過濾，照抄
    assert f"  RAW {second}" in out
    assert "path=baseline" in out
    assert "  OUT [Amy] yo" in out


def test_message_log_stays_quiet_while_the_chat_log_does_not_change():
    buf = io.StringIO()
    r = FakeWiz([_log(_say_colored("FF66CC", "Bob", "hi"))], message_log=MessageLog(buf))
    r.read_new()
    buf.seek(0), buf.truncate()
    r.read_new()
    assert buf.getvalue() == ""


def test_message_log_is_optional():
    r = FakeWiz([_log(_say_colored("FF66CC", "Bob", "hi"))])
    assert r.read_new() == []


# --- 連線失敗的分類（不需遊戲：以假 ClientHandler 注入例外）---
class _FailingHandler:
    """get_new_clients 一律丟出指定例外；close 供 _teardown 呼叫。"""

    def __init__(self, exc):
        self._exc = exc

    def get_new_clients(self):
        raise self._exc

    async def close(self):
        pass


def _reader_failing_to_open(monkeypatch, exc):
    """讓 _connect 走到 get_new_clients 就丟出 exc 的 reader。"""
    import wizwalker

    from src.reader import mem_reader
    monkeypatch.setattr(mem_reader, "detect_install_path", lambda: None)
    monkeypatch.setattr(wizwalker, "ClientHandler", lambda **kw: _FailingHandler(exc))
    return WizChatReader()


def test_open_process_denied_raises_access_denied(monkeypatch):
    # 遊戲以較高權限執行時 pymem 開不了 handle：要能與「找不到遊戲」分辨開來
    from pymem.exception import CouldNotOpenProcess
    r = _reader_failing_to_open(monkeypatch, CouldNotOpenProcess(4321))
    try:
        r._connect()
        assert False, "應丟 GameAccessDenied"
    except GameAccessDenied as exc:
        assert "4321" in str(exc)


def test_access_denied_is_a_game_not_running():
    # 繼承既有例外，上層的退避重連照舊生效，只在文案上分流
    assert issubclass(GameAccessDenied, GameNotRunning)


def test_other_connect_failure_stays_game_not_running(monkeypatch):
    r = _reader_failing_to_open(monkeypatch, RuntimeError("boom"))
    try:
        r._connect()
        assert False, "應丟 GameNotRunning"
    except GameAccessDenied:
        assert False, "非權限錯誤不得歸類為權限不足"
    except GameNotRunning:
        pass


def test_failed_connect_closes_event_loop(monkeypatch):
    # 連線失敗每 poll_interval 重試一輪，沒收掉 loop 會一輪洩漏一個
    r = _reader_failing_to_open(monkeypatch, RuntimeError("boom"))
    try:
        r._connect()
    except GameNotRunning:
        pass
    assert r._loop is None


# --- 掛入點與遊戲版本對不上的分類（純函式，不需遊戲）---
def test_pattern_failures_are_classified_as_version_mismatch():
    from wizwalker.errors import PatternFailed, PatternMultipleResults
    assert is_version_mismatch(PatternFailed(b"\x90\x48"))
    assert is_version_mismatch(PatternMultipleResults("got 2 results"))


def test_hook_ready_timeout_is_classified_as_version_mismatch():
    # pattern 掃得到、hook 也寫進去了，卻遲遲沒被觸發＝那段程式碼已不在執行路徑
    assert is_version_mismatch(TimeoutError("root window hook did not fire"))


def test_unrelated_connect_failures_are_not_version_mismatch():
    assert not is_version_mismatch(RuntimeError("boom"))
    assert not is_version_mismatch(OSError("nope"))


def test_version_mismatch_is_a_game_not_running():
    # 繼承既有例外，上層的退避重連照舊生效，只在文案上分流
    assert issubclass(GameVersionMismatch, GameNotRunning)


# --- 等待 hook 就緒：逾時而非無限等 ---
class _StubHookHandler:
    """read_current_root_window_base 依序回傳 values（例外物件即拋出，耗盡後一律回 0）。"""

    def __init__(self, values=(), activate_exc=None):
        self.values = list(values)
        self.activate_exc = activate_exc
        self.activate_kwargs = None
        self.process = type("P", (), {"base_address": 0x400000})()

    async def activate_root_window_hook(self, **kwargs):
        self.activate_kwargs = kwargs
        if self.activate_exc is not None:
            raise self.activate_exc

    async def read_current_root_window_base(self):
        value = self.values.pop(0) if self.values else 0
        if isinstance(value, Exception):
            raise value
        return value


class _StubClient:
    process_id = 4321

    def __init__(self, hook_handler):
        self.hook_handler = hook_handler
        self._pymem = type("M", (), {"base_address": 0x400000})()


def _waiting_reader(hook_handler):
    """已連上假 client、只差等待 hook 就緒的 reader。"""
    import asyncio
    r = WizChatReader()
    r._loop = asyncio.new_event_loop()
    r._client = _StubClient(hook_handler)
    return r


def test_wait_root_window_ready_returns_once_the_address_is_set(monkeypatch):
    from src.reader import mem_reader
    monkeypatch.setattr(mem_reader, "HOOK_READY_POLL", 0.0)
    r = _waiting_reader(_StubHookHandler([0, 0, 0x1234]))
    r._wait_root_window_ready()  # 不拋即通過


def test_wait_root_window_ready_ignores_transient_read_errors(monkeypatch):
    # hook 剛寫入時位址還沒有效，讀取失敗是常態，不該當成不相容
    from src.reader import mem_reader
    monkeypatch.setattr(mem_reader, "HOOK_READY_POLL", 0.0)
    r = _waiting_reader(_StubHookHandler([RuntimeError("read failed"), 0x1234]))
    r._wait_root_window_ready()


def test_wait_root_window_ready_times_out_instead_of_hanging_forever(monkeypatch):
    from src.reader import mem_reader
    monkeypatch.setattr(mem_reader, "HOOK_READY_TIMEOUT", 0.05)
    monkeypatch.setattr(mem_reader, "HOOK_READY_POLL", 0.0)
    r = _waiting_reader(_StubHookHandler())  # 位址永遠是 0
    try:
        r._wait_root_window_ready()
        assert False, "應丟 TimeoutError"
    except TimeoutError:
        pass


# --- 掛入路徑的接線 ---
class _StubHandler:
    """get_new_clients 回傳固定的假 client；close 供 _teardown 呼叫。"""

    def __init__(self, client):
        self._client = client

    def get_new_clients(self):
        return [self._client]

    async def close(self):
        pass


def _connecting_reader(monkeypatch, tmp_path, hook_handler):
    """讓 _connect 走完整條掛入路徑的 reader（狀態檔改寫進 tmp_path）。"""
    import wizwalker

    from src.reader import hook_state, mem_reader
    monkeypatch.setattr(mem_reader, "detect_install_path", lambda: None)
    monkeypatch.setattr(mem_reader, "HOOK_READY_POLL", 0.0)
    monkeypatch.setattr(hook_state, "STATE_DIR", tmp_path)
    client = _StubClient(hook_handler)
    monkeypatch.setattr(wizwalker, "ClientHandler", lambda **kw: _StubHandler(client))
    return WizChatReader()


def test_pattern_failure_while_attaching_raises_version_mismatch(monkeypatch, tmp_path):
    from wizwalker.errors import PatternFailed
    r = _connecting_reader(monkeypatch, tmp_path,
                           _StubHookHandler(activate_exc=PatternFailed(b"\x90")))
    try:
        r._connect()
        assert False, "應丟 GameVersionMismatch"
    except GameVersionMismatch:
        pass


def test_hook_never_firing_raises_version_mismatch(monkeypatch, tmp_path):
    from src.reader import mem_reader
    monkeypatch.setattr(mem_reader, "HOOK_READY_TIMEOUT", 0.05)
    r = _connecting_reader(monkeypatch, tmp_path, _StubHookHandler())
    try:
        r._connect()
        assert False, "應丟 GameVersionMismatch"
    except GameVersionMismatch:
        pass


def test_other_attach_failure_stays_game_not_running(monkeypatch, tmp_path):
    r = _connecting_reader(monkeypatch, tmp_path,
                           _StubHookHandler(activate_exc=RuntimeError("boom")))
    try:
        r._connect()
        assert False, "應丟 GameNotRunning"
    except GameVersionMismatch:
        assert False, "非 pattern／逾時的失敗不得歸類為版本不相容"
    except GameNotRunning:
        pass


def test_hook_state_is_saved_before_waiting_for_the_hook_to_fire(monkeypatch, tmp_path):
    """逾時被硬砍也要修得回來：狀態檔必須在等待就緒之前就寫好。"""
    from src.reader import hook_state

    saved_when_waiting = []

    class _Watching(_StubHookHandler):
        async def read_current_root_window_base(self):
            saved_when_waiting.append(hook_state._state_path(4321).exists())
            return 0x1234

    handler = _Watching()
    handler._autobot_address = 0x500000
    handler._original_autobot_bytes = b"\x90" * 8
    r = _connecting_reader(monkeypatch, tmp_path, handler)
    r._connect()
    assert saved_when_waiting and saved_when_waiting[0], \
        "等待 hook 就緒時，還原狀態檔必須已經存在"


def test_attach_does_not_use_wizwalkers_unbounded_wait(monkeypatch, tmp_path):
    # wizwalker 內建的 wait_for_ready 預設無限等且不可中斷，一律自己等
    handler = _StubHookHandler([0x1234])
    r = _connecting_reader(monkeypatch, tmp_path, handler)
    r._connect()
    assert handler.activate_kwargs == {"wait_for_ready": False}


def _tab_switch_script(mine: str, other: str) -> list[str]:
    """前 6 輪停在同一視圖（建立基準並耗掉 RESET_WARMUP_POLLS 的暖機吸收期），
    其後在兩個視圖之間來回切，模擬切聊天頁籤。"""
    return [mine] * 6 + [other, mine, other, mine]


def test_no_release_while_the_input_box_is_open():
    """輸入框開著時切頁籤，自己的舊發言不得被當成「剛送出」而重複顯示。

    實機回報：重開軟體後切聊天頁籤重複顯示同一句。app.log 顯示放行發生在
    「開啟輸入框後 60 毫秒」——那一刻使用者還在打字，不可能有剛送出的訊息，
    重新浮現的其實是切頁籤帶出來的舊訊息。送出的那一輪 input_open 為 False
    （訊息是在輸入框關掉之後才出現在 chatLog 裡），開著時放行純屬誤判。
    """
    mine = _own("Test321")
    other = _log(_say(1, "Amy", "123"), _say(2, "Bob", "456"))
    script = _tab_switch_script(mine, other)
    r = FakeWiz(script, inputs=[True] * len(script))   # 輸入框全程開著
    for _ in range(6):
        r.read_new()
    r.read_new()                            # 切到別人的視圖：首次見到，照吐
    assert _texts(r.read_new()) == [], "輸入框開著時不該放行重浮的舊發言"
    r.read_new()
    assert _texts(r.read_new()) == [], "再切回來同樣不該放行"


def test_release_fires_after_the_input_box_closes():
    """輸入框剛關閉＝剛送出：重打同一句被看過集合擋下時仍要放行（原始救援場景）。"""
    mine = _own("Test321")
    other = _log(_say(1, "Amy", "123"), _say(2, "Bob", "456"))
    script = _tab_switch_script(mine, other)
    # 前 6 輪開著（打字中），第 7 輪起關閉＝送出
    inputs = [True] * 6 + [False] * (len(script) - 6)
    r = FakeWiz(script, inputs=inputs)
    for _ in range(6):
        r.read_new()
    r.read_new()                            # 輸入框關閉的那一輪
    assert _texts(r.read_new()) == ["[你] Test321"], "剛送出的同字訊息要放行"


def test_release_fires_once_per_input_session():
    """關聯放行的額度以「一次輸入」為單位，窗內不得重複放行同一行。"""
    mine = _own("Test321")
    other = _log(_say(1, "Amy", "123"), _say(2, "Bob", "456"))
    script = _tab_switch_script(mine, other)
    inputs = [True] * 6 + [False] * (len(script) - 6)
    r = FakeWiz(script, inputs=inputs)
    for _ in range(6):
        r.read_new()
    r.read_new()
    assert _texts(r.read_new()) == ["[你] Test321"]
    r.read_new()
    assert _texts(r.read_new()) == [], "同一次輸入只放行一次"
