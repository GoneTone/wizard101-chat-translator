"""messages.log 的記錄格式：逐筆 hook 記錄與附註（時戳由串流負責，此處不驗）。"""
import io

from src.reader.message_log import MessageLog, is_debug_line

SAY = "<color;FF66CC><image;Art/Art_Chat_Say.dds;24;24;FF><link;GID:1,Bob,2>[Bob]</link> hi &lt;3 </color>"
SYS = "<color;FFFFFF><image;Art/Art_Chat_System.dds;24;24;FF> A wizard has joined </color>"
WARN = "[WARN] Texture was not loaded correctly</color>"
DBGM = "[DBGM] ------ CloseInteraction</color>"


def _log(slot=None):
    buf = io.StringIO()
    return buf, MessageLog(buf, slot=slot)


def test_record_writes_seq_tab_verdict_and_raw_markup():
    buf, log = _log()
    log.record(7, 1, SAY, "player")
    assert buf.getvalue() == f"[seq=7 tab=1 player] {SAY}\n"


def test_note_writes_the_text_as_is():
    buf, log = _log()
    log.note("lost 3 chat lines (seq 4-9)")
    assert buf.getvalue() == "lost 3 chat lines (seq 4-9)\n"


def test_debug_lines_are_not_recorded():
    buf, log = _log()
    log.record(1, 0, WARN, "dropped")
    log.record(2, 0, SYS, "system-off")
    assert buf.getvalue() == f"[seq=2 tab=0 system-off] {SYS}\n"


def test_game_debug_tags_are_recognised():
    assert is_debug_line(WARN)
    assert is_debug_line(DBGM)
    assert is_debug_line("[ERRO] Texture::LoadTexture(FriendlyPlayer) FAILED!</color>")


def test_chat_lines_are_not_debug_lines():
    assert not is_debug_line(SAY)
    assert not is_debug_line(SYS)


def test_slot_prefix_marks_every_line():
    # 雙開時兩條 reader 共寫同一份 messages.log，沒前綴就分不出哪個帳號的記錄
    buf, log = _log(slot=2)
    log.record(1, 0, SAY, "player")
    log.note("lost 1 chat lines")
    lines = buf.getvalue().splitlines()
    assert len(lines) == 2 and all(line.startswith("[slot=2] ") for line in lines)
