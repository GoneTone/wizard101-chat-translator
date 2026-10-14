"""chatLog 標記解析：clean、parse_line、頻道圖示、系統訊息、伺服器公告、顏色。"""

from src.reader.diff import player_out_with_idx
from src.reader.markup import clean, lines_from_nodes, parse_line
from tests.chatlog_helpers import (
    _broadcast,
    _log,
    _own,
    _say,
    _say_colored,
    _system,
    _system_colored,
    _texts,
    _texts_colors,
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


# --- parse_line：單行 markup 分類 ---


def _lines(raw):
    return [line for r in raw.split("\n") if (line := parse_line(r))]


def test_lines_extracts_player_say_with_sender():
    assert parse_line(_say(1, "Wolf", "hello world")).text == "[Wolf] hello world"


def test_lines_keeps_cjk_sender_and_spaces():
    log = _say(196751008724053815, "沃尔夫 亡灵骑兵", "wth")
    assert parse_line(log).text == "[沃尔夫 亡灵骑兵] wth"


def test_lines_flags_system_messages():
    # 系統訊息（掉寶/經驗/升等）照收，但標上 system 旗標走另一條差分軌
    log = "\n".join([_say(1, "Amy", "hi"), _system("你獲得了 51 金幣！"),
                     _system("你現在等級 28！")])
    lines = _lines(log)
    assert _texts(lines) == ["[Amy] hi", "你獲得了 51 金幣！", "你現在等級 28！"]
    assert [line.system for line in lines] == [False, True, True]


def test_lines_skips_debug_rows():
    # 除錯 chatLog 節點的行（[DBGL]/[STAT] 無 <link;GID>）不得誤入
    log = "\n".join(["[DBGL] HandleStatisticUpdate: new health 1783",
                     "[STAT] BuddyListManager::MSG_BuddyEntry Added",
                     _say(1, "Q", "back")])
    assert _texts(_lines(log)) == ["[Q] back"]


def test_lines_keeps_own_message():
    assert parse_line(_own("zztest123")).text == "[你] zztest123"
    assert parse_line(_own("測試 訊息 :)")).text == "[你] 測試 訊息 :)"


def test_lines_keeps_own_and_others_together():
    log = "\n".join([_say(1, "Amy", "hi"), _own("我回你"), _system("你獲得了 51 金幣！"),
                     "[STAT] noise", _say(2, "Bob", "yo")])
    assert _texts(_lines(log)) == ["[Amy] hi", "[你] 我回你",
                                               "你獲得了 51 金幣！", "[Bob] yo"]


def test_lines_preserves_order_and_repeats():
    log = "\n".join([_say(1, "A", "hi"), _say(2, "B", "yo"), _say(1, "A", "hi")])
    assert _texts(_lines(log)) == ["[A] hi", "[B] yo", "[A] hi"]


def test_lines_keeps_emoticon_line():
    log = ("<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> "
           "<link;GID:1,Lars,2>[Lars]</link> ty king "
           "<image;Emoticons/Emoticons_Heart.dds;24;24;FFFFFFFF> </color>")
    assert parse_line(log).text == "[Lars] ty king :heart:"


def test_lines_keeps_astral_emoji_text():
    assert parse_line(_say(1, "Amy", "nice 😂👀")).text == "[Amy] nice 😂👀"


def test_lines_empty_for_empty_input():
    assert parse_line("") is None


# --- 非 Art_Chat 圖示的玩家頻道（房間等）：chat_balloon_* 前綴 ---
def test_lines_keeps_house_channel_message_with_balloon_icon():
    # 房間頻道實測 markup：圖示為 chat_balloon_Owner（房主）、頻道色 FFFF00
    raw = ("<color;FFFF00><image;Art/chat_balloon_Owner.dds;24;24;FFFFFFFF> "
           "[你] Test1</color>")
    assert _texts_colors([parse_line(raw)]) == [("[你] Test1", "#ffff00")]


def test_lines_keeps_house_guest_variant():
    raw = ("<color;FFFF00><image;Art/chat_balloon_Guest.dds;24;24;FFFFFFFF> "
           "<link;GID:9,Amy,2>[Amy]</link> hi house</color>")
    assert _texts_colors([parse_line(raw)]) == [("[Amy] hi house", "#ffff00")]


def test_lines_skips_debug_rows_without_icon():
    # [DBGM]/[DBGL]/[STAT] 行無圖示：即使去標記後長得像聊天行也不得誤入
    log = "\n".join(["[DBGM] MSG_SendBlob 114 CurrentZone 114</color>",
                     "[DBGM] Loading housing blob: Type:Garden Objects:1</color>"])
    assert _lines(log) == []


# --- 系統訊息：照收但標上 system 旗標（走獨立差分軌，見 _diff_system_lines）---


def test_system_lines_are_emitted_with_the_system_flag():
    line = parse_line(_system_colored("00FF00", "你获得了 39 金币！"))
    assert line.text == "你获得了 39 金币！"
    assert line.system is True
    assert line.color == "#00ff00"


def test_player_lines_are_not_flagged_as_system():
    assert parse_line(_say_colored("FFFFFF", "Lars", "hi")).system is False


def test_system_lines_do_not_need_a_sender_prefix():
    # 玩家行必須通過 _VALID 的 [發送者] 規則，系統行沒有前綴、不適用
    assert parse_line(_system_colored("AA00AA", "你获得了 3 经验值！")).text == "你获得了 3 经验值！"


def test_system_lines_that_clean_to_nothing_are_dropped():
    assert parse_line(_system_colored("00FF00", "")) is None


def test_debug_lines_are_still_dropped():
    # 除錯行沒有任何頻道圖示，不會因為放行系統訊息而混進來
    assert parse_line("[DBGM] some debug noise") is None
    assert parse_line("[WARN] another one") is None


# --- 伺服器公告：只有 <color;..>、連圖示都沒有，同樣走系統軌 ---


def test_server_broadcast_without_any_icon_is_flagged_as_system():
    # 實機樣本：維修預告只有顏色標記，沒有任何 Art/ 圖示（app.log 也警告不到）
    line = parse_line(
        _broadcast("D9ABF8", "[Server Message] 服务器在2小时内关闭以进行维护"))
    assert line.text == "[Server Message] 服务器在2小时内关闭以进行维护"
    assert line.system is True
    assert line.color == "#d9abf8"


def test_server_broadcast_does_not_need_a_sender_prefix():
    line = parse_line(_broadcast("D9ABF8", "服务器即将关闭"))
    assert line.text == "服务器即将关闭"
    assert line.system is True


def test_server_broadcast_that_cleans_to_nothing_is_dropped():
    assert parse_line(_broadcast("D9ABF8", "")) is None


def test_iconless_lines_without_a_leading_color_tag_are_still_dropped():
    # 遊戲的除錯輸出同樣沒有圖示，但不以 <color;..> 起頭（實機樣本）
    assert parse_line("RECEIVED STATUS UPDATE for [191965934165243834]") is None
    assert parse_line("收到 [1688849928085385] 的伙伴邀请") is None


def test_lines_with_an_unknown_art_icon_are_not_taken_as_broadcasts():
    # 帶未知頻道圖示的行仍走 _warn_unknown_icon 丟棄，不會被誤收成伺服器公告
    raw = ("<color;FFFFFF><image;Art/Art_Unknown_Channel.dds;24;24;FFFFFFFF> "
           "[Lars] hi </color>")
    assert parse_line(raw) is None


def test_system_and_player_lines_keep_their_in_game_order():
    raw = _log(_system_colored("00FF00", "你获得了 39 金币！"),
               _say_colored("FFFFFF", "Lars", "hi"),
               _system_colored("AA00AA", "你获得了 3 经验值！"))
    lines = _lines(raw)
    assert [line.text for line in lines] == ["你获得了 39 金币！", "[Lars] hi", "你获得了 3 经验值！"]
    assert [line.system for line in lines] == [True, False, True]


def test_mirror_detection_still_only_considers_player_lines():
    # 主視圖有玩家行＋系統行，副節點只鏡射玩家行 → 仍判定為鏡射並剔除
    main = _log(_say_colored("FFFFFF", "Lars", "hi"), _system_colored("00FF00", "你获得了 39 金币！"))
    mirror = _say_colored("FFFFFF", "Lars", "hi")
    lines, mirrored = lines_from_nodes([main, mirror])
    assert mirrored == 1
    assert [line.text for line in lines] == ["[Lars] hi", "你获得了 39 金币！"]


def test_player_index_mapping_skips_lines_filtered_out_of_the_tail():
    # 慢路徑會從尾段「中間」剔除重浮歷史，剩下的不再是連續尾段：
    # 若只取同長度的尾段索引，吐出的會是 b、c 而不是 a、c
    cur_all = _lines(_log(_say_colored("FFFFFF", "P", "a"),
                                      _system_colored("00FF00", "掉寶"),
                                      _say_colored("FFFFFF", "P", "b"),
                                      _say_colored("FFFFFF", "P", "c")))
    player_idx = [0, 2, 3]
    cur_player = [cur_all[i] for i in player_idx]
    assert player_out_with_idx([cur_player[0], cur_player[2]], cur_player, player_idx) == [0, 3]
    assert player_out_with_idx([], cur_player, player_idx) == []


def test_player_index_mapping_tells_repeated_lines_apart():
    # 重複行（lol／gg）文字一模一樣，只能以物件識別對回索引
    cur_all = _lines(_log(_say_colored("FFFFFF", "P", "lol"),
                                      _system_colored("00FF00", "掉寶"),
                                      _say_colored("FFFFFF", "P", "lol")))
    player_idx = [0, 2]
    cur_player = [cur_all[i] for i in player_idx]
    assert player_out_with_idx([cur_player[1]], cur_player, player_idx) == [2]


# --- 行帶遊戲顏色：<color;RRGGBB> 解析成 ChatLine.color，供 overlay 對齊遊戲顯示色 ---
def test_lines_carry_game_color():
    line = parse_line(_say(1, "Wolf", "hello world"))
    assert line.text == "[Wolf] hello world"
    assert line.color == "#ffffff"
    assert line.own is False   # 帶玩家連結＝別人講的


def test_lines_color_normalized_lowercase_hex():
    raw = "<color;80FF00><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [你] hi </color>"
    assert _texts_colors([parse_line(raw)]) == [("[你] hi", "#80ff00")]


def test_lines_color_takes_last_six_of_eight_digit_hex():
    # 帶 alpha 的 AARRGGBB 形式：只取後 6 位當顯示色
    raw = "<color;FF80FF00><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [你] hi </color>"
    assert _texts_colors([parse_line(raw)]) == [("[你] hi", "#80ff00")]


def test_lines_color_missing_is_none():
    raw = "<image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [你] hi"
    assert _texts_colors([parse_line(raw)]) == [("[你] hi", None)]
