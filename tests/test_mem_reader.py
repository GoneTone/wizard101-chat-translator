from src.reader.mem_reader import (
    GameNotRunning, WizChatReader, align_append, clean, lines_from_chatlog,
)


# --- clean:去標記 / 還原實體 / 表情 ---
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


# --- lines_from_chatlog:從 chatLog 全文抽玩家發言 ---
def _say(gid: int, name: str, text: str) -> str:
    return (f"<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> "
            f"<link;GID:{gid},{name},2>[{name}]</link> {text} </color>")


def _system(text: str) -> str:
    return f"<color;00FF00><image;Art/Art_Chat_System.dds;24;24;FFFFFFFF> {text}</color>"


def _own(text: str) -> str:
    # 自己的發言:[你] 開頭,帶 Art_Chat 圖示但無 <link;GID>
    return f"<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [你] {text} </color>"


def test_lines_extracts_player_say_with_sender():
    assert lines_from_chatlog(_say(1, "Wolf", "hello world")) == ["[Wolf] hello world"]


def test_lines_keeps_cjk_sender_and_spaces():
    log = _say(196751008724053815, "沃尔夫 亡灵骑兵", "wth")
    assert lines_from_chatlog(log) == ["[沃尔夫 亡灵骑兵] wth"]


def test_lines_skips_system_messages():
    # 系統訊息(無 <link;GID>)不翻:掉寶/經驗/升等
    log = "\n".join([_say(1, "Amy", "hi"), _system("你獲得了 51 金幣！"),
                     _system("你現在等級 28！")])
    assert lines_from_chatlog(log) == ["[Amy] hi"]


def test_lines_skips_debug_rows():
    # 除錯 chatLog 節點的行([DBGL]/[STAT] 無 <link;GID>)不得誤入
    log = "\n".join(["[DBGL] HandleStatisticUpdate: new health 1783",
                     "[STAT] BuddyListManager::MSG_BuddyEntry Added",
                     _say(1, "Q", "back")])
    assert lines_from_chatlog(log) == ["[Q] back"]


def test_lines_keeps_own_message():
    # 自己的發言([你],無 link)也要收
    assert lines_from_chatlog(_own("zztest123")) == ["[你] zztest123"]
    assert lines_from_chatlog(_own("測試 訊息 :)")) == ["[你] 測試 訊息 :)"]


def test_lines_keeps_own_and_others_together():
    log = "\n".join([_say(1, "Amy", "hi"), _own("我回你"), _system("你獲得了 51 金幣！"),
                     "[STAT] noise", _say(2, "Bob", "yo")])
    assert lines_from_chatlog(log) == ["[Amy] hi", "[你] 我回你", "[Bob] yo"]


def test_lines_preserves_order_and_repeats():
    log = "\n".join([_say(1, "A", "hi"), _say(2, "B", "yo"), _say(1, "A", "hi")])
    assert lines_from_chatlog(log) == ["[A] hi", "[B] yo", "[A] hi"]


def test_lines_keeps_emoticon_line():
    log = (f"<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> "
           f"<link;GID:1,Lars,2>[Lars]</link> ty king "
           f"<image;Emoticons/Emoticons_Heart.dds;24;24;FFFFFFFF> </color>")
    assert lines_from_chatlog(log) == ["[Lars] ty king :heart:"]


def test_lines_keeps_astral_emoji_text():
    assert lines_from_chatlog(_say(1, "Amy", "nice 😂👀")) == ["[Amy] nice 😂👀"]


def test_lines_empty_when_no_player_chat():
    assert lines_from_chatlog(_system("你獲得了 14 金幣！")) == []
    assert lines_from_chatlog("") == []


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


# --- WizChatReader:以假 chatLog 文字驗證差分流程(不需遊戲) ---
class FakeWiz(WizChatReader):
    """以腳本化的 chatLog 全文序列取代 wizwalker I/O。"""

    def __init__(self, texts):
        super().__init__()
        self.texts = texts
        self.n = 0
        self._connected = True

    def _grab_text(self) -> str:
        i = min(self.n, len(self.texts) - 1)
        self.n += 1
        t = self.texts[i]
        if isinstance(t, Exception):
            raise t
        return t


def _log(*lines: str) -> str:
    return "\n".join(lines)


def test_first_read_skips_history():
    r = FakeWiz([_log(_say(1, "A", "old1"), _say(1, "A", "old2"))])
    assert r.read_new() == []          # 首次:記錄現況,不回吐既有歷史


def test_appended_lines_emitted():
    r = FakeWiz([
        _log(_say(1, "A", "one")),
        _log(_say(1, "A", "one"), _say(2, "B", "two"), _say(2, "B", "three")),
    ])
    assert r.read_new() == []
    assert r.read_new() == ["[B] two", "[B] three"]


def test_repeats_preserved_across_polls():
    r = FakeWiz([
        _log(_say(1, "A", "hi")),
        _log(_say(1, "A", "hi"), _say(1, "A", "hi")),
        _log(_say(1, "A", "hi"), _say(1, "A", "hi"), _say(1, "A", "hi")),
    ])
    assert r.read_new() == []
    assert r.read_new() == ["[A] hi"]
    assert r.read_new() == ["[A] hi"]


def test_head_trim_scroll_absorbed():
    r = FakeWiz([
        _log(_say(1, "A", "a"), _say(1, "B", "b"), _say(1, "C", "c")),
        _log(_say(1, "B", "b"), _say(1, "C", "c"), _say(1, "D", "d")),
    ])
    assert r.read_new() == []
    assert r.read_new() == ["[D] d"]   # 頭部修剪 + 尾端附加(捲動)只吐新行


def test_own_message_emitted():
    r = FakeWiz([
        _log(_say(1, "A", "hi")),
        _log(_say(1, "A", "hi"), _own("me too"), _say(2, "B", "yo")),
    ])
    assert r.read_new() == []
    assert r.read_new() == ["[你] me too", "[B] yo"]


def test_system_and_debug_never_emitted():
    r = FakeWiz([
        _log(_say(1, "A", "hi")),
        _log(_say(1, "A", "hi"), _system("你獲得了 51 金幣！"),
             "[DBGL] noise", _say(2, "B", "real")),
    ])
    assert r.read_new() == []
    assert r.read_new() == ["[B] real"]


def test_hard_reset_emits_new_content():
    # 聊天被重置成全新內容(如 relog)→ 新內容視為新訊息輸出,之後正常延續
    r = FakeWiz([
        _log(_say(1, "A", "one")),
        _log(_say(9, "Z", "fresh")),                       # 對不齊 → 全新內容
        _log(_say(9, "Z", "fresh"), _say(9, "Z", "next")),
    ])
    assert r.read_new() == []
    assert r.read_new() == ["[Z] fresh"]
    assert r.read_new() == ["[Z] next"]


def test_transient_empty_does_not_retranslate_on_refill():
    # 傳送/轉場時聊天暫態讀成空,之後又填回同樣歷史 → 不可重複翻譯
    hist = _log(_say(1, "A", "a"), _say(2, "B", "b"))
    r = FakeWiz([
        hist,                                              # sync 基準
        "",                                                # 轉場暫態:空讀
        hist,                                              # 填回同樣歷史
        _log(_say(1, "A", "a"), _say(2, "B", "b"), _say(3, "C", "c")),
    ])
    assert r.read_new() == []          # sync
    assert r.read_new() == []          # 空讀:忽略,保留基準
    assert r.read_new() == []          # 填回同樣歷史:不重譯
    assert r.read_new() == ["[C] c"]   # 之後新訊息照常


def test_first_message_after_empty_chat_is_emitted():
    # 空聊天室(登入後)→ 第一句就要抓到,不能被當成初始歷史跳過
    r = FakeWiz([
        "",                                          # 空聊天室
        _log(_say(1, "A", "first")),                 # 第一句
        _log(_say(1, "A", "first"), _say(2, "B", "second")),
    ])
    assert r.read_new() == []                        # 首次:空基準
    assert r.read_new() == ["[A] first"]
    assert r.read_new() == ["[B] second"]


def test_message_after_chat_cleared_is_emitted():
    # 有歷史 → 聊天被清空(relog) → 清空後第一句仍要抓到
    r = FakeWiz([
        _log(_say(1, "A", "old")),
        "",                                          # 清空
        _log(_say(1, "C", "fresh")),
    ])
    assert r.read_new() == []                        # 基準=[A old]
    assert r.read_new() == []                        # 清空 → 對不齊 → 重新同步(基準變空)
    assert r.read_new() == ["[C] fresh"]


def test_read_failure_raises_game_not_running():
    r = FakeWiz([_log(_say(1, "A", "hi")), RuntimeError("process gone")])
    assert r.read_new() == []
    try:
        r.read_new()
        assert False, "應丟 GameNotRunning"
    except GameNotRunning:
        pass
