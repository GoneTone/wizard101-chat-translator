"""系統訊息的差分軌：與玩家軌分離、由 emit_system 決定要不要輸出。"""
import io

from src.reader.message_log import MessageLog
from tests.chatlog_helpers import (
    FakeWiz,
    _log,
    _own,
    _say_colored,
    _system_colored,
    _texts,
)


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
    # 「這一輪沒有系統訊息」是日常狀態，不可拿它清掉系統軌基準：基準一沒了下一輪就走 reset，
    # 而 reset 以看過集合過濾 —— 再掉一次一字不差的同樣的寶（實機最常見）就會被吞掉
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
    # 實機：掉寶進行中開私訊／組隊視窗，節點數 1->2。串接結構一變，對齊生出假的 append，
    # 而 append 不過看過集合 → 剛掉的寶再吐一次。系統軌比照玩家軌走 reset，交由看過集合擋下
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
    # 雙軌核心保證：兩軌各走哪條路徑都不影響相對順序（索引同源）。這裡讓玩家軌走 append、
    # 系統軌走 reset（頭部舊掉寶被顯示上限修掉、與基準無重疊），輸出仍須與遊戲內交錯順序一致
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
