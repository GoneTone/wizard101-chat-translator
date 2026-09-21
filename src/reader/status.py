"""收訊狀態的彙整：多個客戶端各自回報，這裡算出 overlay 唯一一條狀態列與橫幅該顯示什麼。

`banner_for`／`translation_banner` 決定橫幅文案（純函式）；`StatusBoard` 收各編號的回報、
彙整後只在結果改變時排進 ui_queue。
"""
import queue
import threading
from collections.abc import Iterable
from typing import TYPE_CHECKING, NamedTuple

if TYPE_CHECKING:   # 只當型別用：reader 不該在執行期依賴 ui
    from src.translation.pool import TranslationPool
    from src.ui.overlay import OverlayWindow

_PLAIN_NOTICE = {"config": "notice.config_error", "offline": "notice.offline"}
# 狀態字的優先序：只要有一個客戶端在忙就顯示忙；要使用者動手的永久性失敗排在「連線中」
# 之後，只在沒有任何客戶端正常時才浮上來
_STATE_PRIORITY = ("translating", "listening", "locating", "access_denied", "version_mismatch")
_PERMANENT_ISSUES = ("notice.access_denied", "notice.version_mismatch")
_ATTACHED_STATES = ("listening", "translating")


def banner_for(game_issue: str | None, error_state: str | None,
               error_detail: tuple[int | None, str] | None = None) -> tuple[str, dict] | None:
    """決定該顯示哪一條錯誤橫幅：（文案 key，format 變數）或 None＝不顯示。
    game_issue（遊戲端問題的文案 key）優先於翻譯錯誤：連不上遊戲時翻譯狀態已無意義。
    error_detail 是 pool 記下的（HTTP 狀態碼，API 說明）：有就照實顯示，
    沒有才退回只靠狀態猜的固定文案。"""
    if game_issue:
        return game_issue, {}
    if error_state not in _PLAIN_NOTICE:
        return None
    if not error_detail:
        return _PLAIN_NOTICE[error_state], {}
    status, message = error_detail
    if error_state == "config":
        return "notice.config_error_detail", {"status": status, "message": message}
    if status is not None:
        return "notice.offline_http", {"status": status, "message": message}
    return "notice.offline_detail", {"message": message}


def translation_banner(game_issue: str | None, pool: "TranslationPool",
                       system_pool: "TranslationPool | None") -> tuple[str, dict] | None:
    """兩條翻譯佇列任一有錯就顯示：玩家對話優先（它才是主要用途）。"""
    failing = pool if pool.error_state else system_pool
    if failing is None:
        return banner_for(game_issue, None)
    return banner_for(game_issue, failing.error_state, failing.error_detail)


class SlotReport(NamedTuple):
    """一個客戶端最近一次回報：狀態字 key 與遊戲端問題的文案 key（None＝遊戲正常）。"""
    state: str
    game_issue: str | None


def aggregate_state(states: Iterable[str]) -> str:
    """多個客戶端的狀態字取一個給狀態列；沒有客戶端或全部一般性失敗＝等待遊戲。"""
    states = set(states)
    for state in _STATE_PRIORITY:
        if state in states:
            return state
    return "waiting_game"


def aggregate_game_issue(reports: Iterable[SlotReport]) -> str | None:
    """橫幅要顯示的遊戲端問題。永久性問題（權限、版本）任一客戶端有就顯示；
    否則只要有一個客戶端掛入成功就不顯示（另一個還在啟動中很正常）；
    沒有任何客戶端、或全部失敗，才顯示找不到遊戲；全部仍在掛入中則不顯示。"""
    reports = list(reports)
    for r in reports:
        if r.game_issue in _PERMANENT_ISSUES:
            return r.game_issue
    if any(r.state in _ATTACHED_STATES for r in reports):
        return None
    if not reports or any(r.game_issue for r in reports):
        return "notice.game_missing"
    return None


class StatusBoard:
    """各客戶端的收訊執行緒只回報自己；這裡彙整成 overlay 的狀態字與橫幅，
    只在結果改變時排進 ui_queue（否則每輪 poll 都會塞一個沒意義的重繪）。"""

    def __init__(self, overlay: "OverlayWindow", ui_queue: queue.Queue,
                 pool: "TranslationPool", system_pool: "TranslationPool | None" = None):
        self._overlay = overlay
        self._queue = ui_queue
        self._pool = pool
        self._system_pool = system_pool
        self._lock = threading.Lock()
        self._slots: dict[int, SlotReport] = {}
        self._status: str | None = None
        self._banner: tuple[str, dict] | None = None

    def report(self, slot: int, state: str, game_issue: str | None) -> None:
        """某編號的最新狀態。"""
        with self._lock:
            self._slots[slot] = SlotReport(state, game_issue)
            self._push()

    def drop(self, slot: int) -> None:
        """某編號的執行緒結束了。"""
        with self._lock:
            self._slots.pop(slot, None)
            self._push()

    def refresh(self) -> None:
        """回報沒變但翻譯池錯誤狀態可能變了（或還沒有任何客戶端）時重算一次。"""
        with self._lock:
            self._push()

    def _push(self) -> None:
        reports = list(self._slots.values())
        status = aggregate_state(r.state for r in reports)
        banner = translation_banner(aggregate_game_issue(reports), self._pool, self._system_pool)
        if status != self._status:
            self._status = status
            self._queue.put(lambda: self._overlay.set_status(status))
        if banner != self._banner:
            self._banner = banner
            if banner is None:
                self._queue.put(self._overlay.clear_error)
            else:
                key, kwargs = banner
                self._queue.put(lambda: self._overlay.set_error(key, **kwargs))
