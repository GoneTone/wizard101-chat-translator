"""多客戶端狀態彙整：優先序、橫幅規則（純函式），以及 StatusBoard 只在變化時推 UI。"""
import queue

from src.reader.status import (
    SlotReport,
    StatusBoard,
    aggregate_game_issue,
    aggregate_state,
)


class FakePool:
    def __init__(self):
        self.error_state = None
        self.error_detail = None
        self.in_flight = 0


class FakeOverlay:
    def __init__(self):
        self.statuses: list[str] = []
        self.errors: list[tuple[str, dict]] = []
        self.clears = 0

    def set_status(self, state):
        self.statuses.append(state)

    def set_error(self, key, **kwargs):
        self.errors.append((key, kwargs))

    def clear_error(self):
        self.clears += 1


def _drain(q):
    while True:
        try:
            q.get_nowait()()
        except queue.Empty:
            break


def test_state_priority_translating_beats_everything():
    assert aggregate_state(["listening", "translating", "locating"]) == "translating"


def test_state_priority_listening_beats_locating_and_failures():
    assert aggregate_state(["locating", "listening", "access_denied"]) == "listening"


def test_permanent_failure_shows_when_nothing_is_attached():
    assert aggregate_state(["waiting_game", "version_mismatch"]) == "version_mismatch"


def test_no_slots_means_waiting_for_the_game():
    assert aggregate_state([]) == "waiting_game"
    assert aggregate_state(["waiting_game", "waiting_game"]) == "waiting_game"


def test_permanent_issue_is_reported_even_if_another_slot_works():
    # 權限不足要使用者動手才解得掉，不能被另一個正常的客戶端蓋掉
    reports = [SlotReport("listening", None), SlotReport("access_denied", "notice.access_denied")]
    assert aggregate_game_issue(reports) == "notice.access_denied"


def test_transient_failure_on_one_slot_is_hidden_while_another_listens():
    # 第二個客戶端還在更新器畫面：第一個正常收訊時不跳「找不到遊戲」
    reports = [SlotReport("listening", None), SlotReport("waiting_game", "notice.game_missing")]
    assert aggregate_game_issue(reports) is None


def test_all_slots_failing_reports_game_missing():
    reports = [SlotReport("waiting_game", "notice.game_missing")]
    assert aggregate_game_issue(reports) == "notice.game_missing"


def test_no_slots_reports_game_missing():
    assert aggregate_game_issue([]) == "notice.game_missing"


def test_all_slots_still_locating_shows_no_banner():
    # 剛啟動、第一輪掛入還沒完成：現況也不跳橫幅
    assert aggregate_game_issue([SlotReport("locating", None)]) is None


def test_board_pushes_only_on_change():
    ov, q = FakeOverlay(), queue.Queue()
    board = StatusBoard(ov, q, FakePool())
    board.report(1, "listening", None)
    board.report(1, "listening", None)
    board.report(2, "locating", None)
    _drain(q)
    assert ov.statuses == ["listening"]


def test_board_drop_recomputes():
    ov, q = FakeOverlay(), queue.Queue()
    board = StatusBoard(ov, q, FakePool())
    board.report(1, "listening", None)
    board.report(2, "locating", None)
    board.drop(1)
    _drain(q)
    assert ov.statuses == ["listening", "locating"]


def test_board_banner_follows_game_issue_and_pool_errors():
    ov, q, pool = FakeOverlay(), queue.Queue(), FakePool()
    board = StatusBoard(ov, q, pool)
    board.report(1, "waiting_game", "notice.game_missing")
    board.report(1, "listening", None)
    pool.error_state = "offline"
    board.refresh()
    _drain(q)
    assert ov.errors == [("notice.game_missing", {}), ("notice.offline", {})]
    assert ov.clears == 1


def test_board_refresh_with_no_slots_reports_waiting_game():
    ov, q = FakeOverlay(), queue.Queue()
    board = StatusBoard(ov, q, FakePool())
    board.refresh()
    _drain(q)
    assert ov.statuses == ["waiting_game"]
    assert ov.errors == [("notice.game_missing", {})]
