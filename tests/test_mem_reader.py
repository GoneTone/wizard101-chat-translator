from src.reader.mem_reader import ChatReader, clean, extract_lines


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


def test_extract_rejects_binary_garbage_copy():
    # 破損副本:開頭合法但中間夾二進位(替換字元 U+FFFD、指標痕跡碼位)
    assert extract_lines(_wrap("[Luc] for all th�Ƞ耀 and drama")) == []
    assert extract_lines(_wrap("[Jin] can gift me a paٲᅨ ls")) == []


def test_extract_rejects_merged_two_messages():
    # 兩則被併在一起(跨越 </color>):含第二個發送者括號
    merged = "[伊莎贝拉] can anyone gift me a potion [达科塔 暗影血统] 不是"
    assert extract_lines(_wrap(merged)) == []


def test_extract_rejects_leftover_markup_fragment():
    assert extract_lines(_wrap("[Han] yeah i did to color> foo")) == []
    assert extract_lines(_wrap("[Han] i hear you &gt; and more")) == []


def test_extract_keeps_clean_cjk_sender():
    assert extract_lines(_wrap("[本杰明] how do u do the eyes")) == ["[本杰明] how do u do the eyes"]


def test_extract_keeps_clean_english():
    assert extract_lines(_wrap("[Luc BelleMer] i cant even blame you")) == ["[Luc BelleMer] i cant even blame you"]


def test_extract_skips_when_no_close_tag():
    open_only = "<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [Wolf] dangling"
    assert extract_lines(u16(open_only)) == []


def test_extract_dedups_within_blob():
    assert extract_lines(u16(SAY + " " + SAY)) == ["[Wolf] hello world"]


def test_extract_multiple_distinct_lines_in_order():
    a = "<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [A] first </color>"
    b = "<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [B] second </color>"
    assert extract_lines(u16(a + b)) == ["[A] first", "[B] second"]


class _FakeReader(ChatReader):
    """以假掃描替換真實記憶體存取,驗證全掃/熱掃的排程。"""

    def __init__(self, **kw):
        super().__init__(**kw)
        self.full_calls = 0
        self.hot_calls = 0

    def _open(self):
        return 1

    def _close(self, handle):
        pass

    def _full_scan(self, handle):
        self.full_calls += 1
        return [f"full{self.full_calls}"], [0x1000, 0x2000]  # 假熱區

    def _hot_scan(self, handle):
        self.hot_calls += 1
        return [f"hot{self.hot_calls}"]


def test_first_read_is_full_scan_and_caches_hot_regions():
    r = _FakeReader(full_scan_every=3)
    assert r.read() == ["full1"]
    assert r.full_calls == 1 and r.hot_calls == 0
    assert r._hot == [0x1000, 0x2000]


def test_hot_scan_used_between_full_scans():
    r = _FakeReader(full_scan_every=3)
    modes = []
    for _ in range(7):
        before_full = r.full_calls
        r.read()
        modes.append("full" if r.full_calls > before_full else "hot")
    # 第1次全掃(熱區空)→ 熱,熱 → 第4次到期全掃 → 熱,熱 → 第7次全掃
    assert modes == ["full", "hot", "hot", "full", "hot", "hot", "full"]
    assert r.full_calls == 3 and r.hot_calls == 4


def test_empty_hot_regions_forces_full_scan():
    r = _FakeReader(full_scan_every=100)
    r._full_scan = lambda h: ([], [])  # 全掃找不到聊天 → 熱區保持空
    r.full_calls = 0
    # 熱區一直空,即使未到 full_scan_every 也應每次都全掃(而非熱掃)
    for _ in range(3):
        r.read()
    assert r.hot_calls == 0


def test_most_common_window_picks_most_duplicated():
    from src.reader.mem_reader import most_common_window
    groups = [("a", "b"), ("a", "b", "c"), ("a", "b", "c"), ("a", "b", "c"), ("x",)]
    # 出現最多份的小群內容 = 當前可視視窗
    assert most_common_window(groups) == ["a", "b", "c"]


def test_most_common_window_empty():
    from src.reader.mem_reader import most_common_window
    assert most_common_window([]) == []


def test_extract_lines_no_dedup_keeps_repeats():
    a = "<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [A] hi </color>"
    blob = u16(a + a)
    assert extract_lines(blob, dedup=False) == ["[A] hi", "[A] hi"]
    assert extract_lines(blob, dedup=True) == ["[A] hi"]
