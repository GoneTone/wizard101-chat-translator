"""messages.log 的純邏輯測試：原始行差分、每輪記錄格式（時戳由串流負責，此處不驗）。"""
import io

from src.reader.message_log import MessageLog, is_debug_line, new_raw_lines

SAY = "<color;FF66CC><image;Art/Art_Chat_Say.dds;24;24;FF><link;GID:1,Bob,2>[Bob]</link> hi &lt;3 </color>"
SYS = "<color;FFFFFF><image;Art/Art_Chat_System.dds;24;24;FF> A wizard has joined </color>"
WARN = "[WARN] Texture was not loaded correctly</color>"
DBGM = "[DBGM] ------ CloseInteraction</color>"


def _log():
    buf = io.StringIO()
    return buf, MessageLog(buf)


# --- new_raw_lines：以出現次數差分，重複發言不會被吃掉 ---
def test_all_lines_new_against_empty_baseline():
    assert new_raw_lines([], ["a", "b"]) == ["a", "b"]


def test_lines_already_in_baseline_are_not_new():
    assert new_raw_lines(["a", "b"], ["a", "b", "c"]) == ["c"]


def test_extra_occurrence_of_a_repeated_line_counts_as_new():
    assert new_raw_lines(["lol"], ["lol", "lol"]) == ["lol"]


def test_shrunken_snapshot_has_no_new_lines():
    assert new_raw_lines(["a", "b", "c"], ["a"]) == []


# --- snapshot ---
def test_first_snapshot_logs_every_raw_line_verbatim():
    buf, log = _log()
    log.snapshot([SAY, SYS], nodes=1, sizes=[212], input_open=False)
    assert buf.getvalue() == (
        "[poll=1 nodes=1 sizes=[212] lines=2 input_open=False new=2]\n"
        f"  RAW {SAY}\n"
        f"  RAW {SYS}\n")


def test_unchanged_snapshot_writes_nothing():
    buf, log = _log()
    log.snapshot([SAY], nodes=1, sizes=[106], input_open=False)
    buf.seek(0), buf.truncate()
    log.snapshot([SAY], nodes=1, sizes=[106], input_open=False)
    assert buf.getvalue() == ""


def test_only_lines_new_since_previous_poll_are_logged():
    buf, log = _log()
    log.snapshot([SAY], nodes=1, sizes=[106], input_open=False)
    buf.seek(0), buf.truncate()
    log.snapshot([SAY, SYS], nodes=1, sizes=[212], input_open=True)
    assert buf.getvalue() == (
        "[poll=2 nodes=1 sizes=[212] lines=2 input_open=True new=1]\n"
        f"  RAW {SYS}\n")


def test_changed_snapshot_without_new_lines_logs_header_only():
    buf, log = _log()
    log.snapshot([SAY, SYS], nodes=1, sizes=[212], input_open=False)
    buf.seek(0), buf.truncate()
    log.snapshot([SAY], nodes=1, sizes=[106], input_open=False)
    assert buf.getvalue() == "[poll=2 nodes=1 sizes=[106] lines=1 input_open=False new=0]\n"


# --- decision ---
def test_decision_lists_the_lines_handed_to_translation():
    buf, log = _log()
    log.snapshot([SAY], nodes=1, sizes=[106], input_open=False)
    buf.seek(0), buf.truncate()
    log.decision("append", appended=1, emitted=["[Bob] hi <3"])
    assert buf.getvalue() == ("[poll=1 path=append appended=1 emitted=1]\n"
                              "  OUT [Bob] hi <3\n")


def test_decision_without_appended_count_omits_the_field():
    buf, log = _log()
    log.snapshot([SAY], nodes=1, sizes=[106], input_open=False)
    buf.seek(0), buf.truncate()
    log.decision("baseline", appended=None, emitted=[])
    assert buf.getvalue() == "[poll=1 path=baseline emitted=0]\n"


def test_decision_silent_when_the_snapshot_did_not_change():
    buf, log = _log()
    log.snapshot([SAY], nodes=1, sizes=[106], input_open=False)
    log.decision("append", appended=1, emitted=["[Bob] hi <3"])
    buf.seek(0), buf.truncate()
    log.snapshot([SAY], nodes=1, sizes=[106], input_open=False)
    log.decision("append", appended=0, emitted=[])
    assert buf.getvalue() == ""


def test_decision_logged_when_lines_are_emitted_from_an_unchanged_snapshot():
    buf, log = _log()
    log.snapshot([SAY], nodes=1, sizes=[106], input_open=False)
    buf.seek(0), buf.truncate()
    log.snapshot([SAY], nodes=1, sizes=[106], input_open=False)
    log.decision("reset", appended=1, emitted=["[Bob] hi <3"])
    assert buf.getvalue() == ("[poll=2 path=reset appended=1 emitted=1]\n"
                              "  OUT [Bob] hi <3\n")


# --- 遊戲自己的除錯行：不是聊天，排除在外 ---
def test_game_debug_tags_are_recognised():
    assert is_debug_line(WARN)
    assert is_debug_line(DBGM)
    assert is_debug_line("[ERRO] Texture::LoadTexture(FriendlyPlayer) FAILED!</color>")


def test_chat_lines_are_not_debug_lines():
    assert not is_debug_line(SAY)
    assert not is_debug_line(SYS)


def test_debug_lines_are_left_out_of_the_log():
    buf, log = _log()
    log.snapshot([SAY, WARN, SYS], nodes=1, sizes=[318], input_open=False)
    assert buf.getvalue() == (
        "[poll=1 nodes=1 sizes=[318] lines=2 input_open=False new=2]\n"
        f"  RAW {SAY}\n"
        f"  RAW {SYS}\n")


def test_poll_whose_only_change_is_debug_noise_writes_nothing():
    buf, log = _log()
    log.snapshot([SAY], nodes=1, sizes=[106], input_open=False)
    buf.seek(0), buf.truncate()
    log.snapshot([SAY, WARN, DBGM], nodes=1, sizes=[212], input_open=False)
    assert buf.getvalue() == ""
