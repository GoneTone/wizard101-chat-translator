"""reader_loop 行為：WizChatReader.read_new() → 推進 context → overlay 佔位 → 提交 pool。
翻譯本身與其重試改由 TranslationPool 負責（見 test_pool.py）。"""
import queue
import threading
import time

import src.main as main_module
from src.translation.context import ChatContext
from src.i18n import t
from src.main import banner_for, reader_loop
from src.reader.mem_reader import (
    ChatLine, GameAccessDenied, GameNotRunning, GameVersionMismatch,
)


class FakePool:
    """記錄提交內容；error_state／in_flight 由測試直接設定。"""

    def __init__(self):
        self.submitted: list[tuple[str, list[str], int]] = []
        self.error_state: str | None = None
        self.in_flight = 0

    def submit(self, line, context, msg_id):
        self.submitted.append((line, list(context), msg_id))


class FakeOverlay:
    def __init__(self):
        self.messages: list[tuple[str, str]] = []
        self.pending_flags: list[bool] = []
        self.colors: list[str | None] = []
        self.errors: list[str] = []
        self.clears = 0
        self.statuses: list[str] = []

    def add_message(self, original, translated, now=None, msg_id=None, pending=False,
                    color=None):
        self.messages.append((original, translated))
        self.pending_flags.append(pending)
        self.colors.append(color)

    def update_message(self, msg_id, translated):
        pass

    def set_error(self, key):
        self.errors.append(key)

    def clear_error(self):
        self.clears += 1

    def set_status(self, state):
        self.statuses.append(state)


class FakeReader:
    """依序回傳 reads[i]（每輪新增的行清單）；跑完設 stop。GameNotRunning 以例外物件表示。"""

    def __init__(self, reads, stop, anchored=True):
        self.reads = reads
        self.stop = stop
        self.anchored = anchored
        self.n = 0
        self.emit_system = False

    def read_new(self):
        i = self.n
        self.n += 1
        if self.n >= len(self.reads):
            self.stop.set()
        r = self.reads[i]
        if isinstance(r, Exception):
            raise r
        # 真實 reader 回傳 ChatLine；腳本可寫純字串（無色）省事
        return [l if isinstance(l, ChatLine) else ChatLine(l, None) for l in r]

    def close(self):
        pass


def run_scripted(cfg, overlay, reads, monkeypatch, pool=None, context=None,
                 system_pool=None, cache=None, readers=None):
    """readers（若提供）會被塞進建立出來的 FakeReader，供呼叫端檢查
    reader.emit_system 等屬性是否真的被 reader_loop 同步過。"""
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    pool = pool or FakePool()
    context = context or ChatContext()

    def make_reader(**kw):
        reader = FakeReader(reads, stop)
        if readers is not None:
            readers.append(reader)
        return reader

    monkeypatch.setattr(main_module, "WizChatReader", make_reader)
    reader_loop(cfg, overlay, ui_queue, stop, context, pool,
                system_pool=system_pool, cache=cache)
    _drain(ui_queue)
    return pool, context


def _drain(ui_queue):
    while True:
        try:
            ui_queue.get_nowait()()
        except queue.Empty:
            break


class FakeCache:
    """記錄查詢；hits 指定哪些原文要命中，以及命中時回傳的譯文。"""

    def __init__(self, hits=None):
        self.hits = dict(hits or {})
        self.queried: list[str] = []

    def get(self, text):
        self.queried.append(text)
        return self.hits.get(text)


def _drop(text: str) -> ChatLine:
    """一行系統訊息（帶遊戲顯示色）。"""
    return ChatLine(text, "#00ff00", False, True)


def test_system_lines_skip_the_chat_context(monkeypatch):
    # 系統訊息不得進入上下文：8 行的窗會被掉寶洗光，玩家對話就失去語境
    cfg = {"poll_interval": 0.01, "translate_system_messages": True}
    ov = FakeOverlay()
    reads = [[_drop("你获得了 39 金币！"), ChatLine("[Lars] hi", None)], []]
    sys_pool = FakePool()
    pool, context = run_scripted(cfg, ov, reads, monkeypatch,
                                 system_pool=sys_pool, cache=FakeCache())
    assert context.snapshot() == ["[Lars] hi"]
    assert [line for line, _, _ in pool.submitted] == ["[Lars] hi"]
    assert [line for line, _, _ in sys_pool.submitted] == ["你获得了 39 金币！"]
    # 系統訊息不吃上下文，提交時 context 必為空
    assert [ctx for _, ctx, _ in sys_pool.submitted] == [[]]


def test_cached_system_line_is_added_already_translated(monkeypatch):
    cfg = {"poll_interval": 0.01, "translate_system_messages": True}
    ov = FakeOverlay()
    reads = [[_drop("你获得了 39 金币！")], []]
    sys_pool = FakePool()
    cache = FakeCache({"你获得了 39 金币！": "你獲得了 39 金幣！"})
    run_scripted(cfg, ov, reads, monkeypatch, system_pool=sys_pool, cache=cache)
    assert ov.messages == [("你获得了 39 金币！", "你獲得了 39 金幣！")]
    assert ov.pending_flags == [False]   # 命中快取＝完成態，不經過佔位
    assert ov.colors == ["#00ff00"]      # 遊戲顯示色照樣帶到 overlay
    assert sys_pool.submitted == []      # 命中就不進 pool
    assert cache.queried == ["你获得了 39 金币！"]   # 確實查過快取，不是巧合命中


def test_uncached_system_line_goes_to_the_system_pool(monkeypatch):
    cfg = {"poll_interval": 0.01, "translate_system_messages": True}
    ov = FakeOverlay()
    reads = [[_drop("你获得了 39 金币！")], []]
    sys_pool = FakePool()
    pool, _ = run_scripted(cfg, ov, reads, monkeypatch,
                           system_pool=sys_pool, cache=FakeCache())
    assert ov.messages == [("你获得了 39 金币！", t("notice.pending"))]
    assert ov.pending_flags == [True]
    assert [line for line, _, _ in sys_pool.submitted] == ["你获得了 39 金币！"]
    assert pool.submitted == []          # 不得混進玩家對話那條佇列


def test_banner_shows_a_system_pool_error_when_the_player_pool_is_clean(monkeypatch):
    # API 掛掉時若剛好只有系統訊息在跑，玩家 pool 的 error_state 還是 None，
    # 橫幅仍然必須出現（見 main.reader_loop 的 or 合併）
    cfg = {"poll_interval": 0.01, "translate_system_messages": True}
    ov = FakeOverlay()
    sys_pool = FakePool()
    sys_pool.error_state = "offline"
    run_scripted(cfg, ov, [[], []], monkeypatch, system_pool=sys_pool,
                 cache=FakeCache())
    assert ov.errors == ["notice.offline"]


def test_system_lines_are_ignored_when_the_setting_is_off(monkeypatch):
    cfg = {"poll_interval": 0.01, "translate_system_messages": False}
    ov = FakeOverlay()
    sys_pool = FakePool()
    # reader.emit_system 為 False 時真實 reader 根本不會吐系統行，
    # 這裡驗證的是 reader_loop 有把設定傳下去（而不是只因為腳本裡沒有系統行）
    reads = [[ChatLine("[Lars] hi", None)], []]
    readers: list = []
    pool, _ = run_scripted(cfg, ov, reads, monkeypatch, system_pool=sys_pool,
                           cache=FakeCache(), readers=readers)
    assert sys_pool.submitted == []
    assert readers[0].emit_system is False


class TogglingEmitSystemReader(FakeReader):
    """在第一輪讀取後把 cfg 的開關切成 True，藉此驗證 reader.emit_system 是否
    在每輪迴圈開頭重新同步（而不是只在建立 reader 當下設一次就固定住）。
    emit_system_log 記錄每輪呼叫 read_new() 時，reader.emit_system 當下的值。"""

    def __init__(self, reads, stop, cfg):
        super().__init__(reads, stop)
        self.cfg = cfg
        self.emit_system_log: list[bool] = []

    def read_new(self):
        self.emit_system_log.append(self.emit_system)
        if self.n == 0:
            self.cfg["translate_system_messages"] = True
        return super().read_new()


def test_reader_emit_system_is_resynced_every_loop_iteration(monkeypatch):
    # 設定可能在執行中被使用者從設定視窗切換；若只在建立 reader 當下同步一次，
    # 中途切換就要等下次重開程式才生效。這裡在第一輪讀取後把 cfg 切成 True，
    # 驗證第二輪迴圈開頭已經跟上（見 main.reader_loop 主迴圈開頭那行同步）。
    cfg = {"poll_interval": 0.01, "translate_system_messages": False}
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    reads = [[], [], []]
    created: list[TogglingEmitSystemReader] = []

    def make_reader(**kw):
        reader = TogglingEmitSystemReader(reads, stop, cfg)
        created.append(reader)
        return reader

    monkeypatch.setattr(main_module, "WizChatReader", make_reader)
    reader_loop(cfg, FakeOverlay(), ui_queue, stop, ChatContext(), FakePool(),
                system_pool=FakePool(), cache=FakeCache())
    _drain(ui_queue)
    assert created[0].emit_system_log == [False, True, True]


def test_new_lines_are_placeheld_and_submitted_in_order(monkeypatch):
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    reads = [[], ["[A] a"], ["[B] hi", "[B] hi"], []]
    pool, _ = run_scripted(cfg, ov, reads, monkeypatch)
    # overlay 依讀取順序先佔位（譯文位置為「翻譯中…」），順序不由翻譯完成先後決定
    assert ov.messages == [("[A] a", t("notice.pending")), ("[B] hi", t("notice.pending")),
                           ("[B] hi", t("notice.pending"))]
    assert [line for line, _, _ in pool.submitted] == ["[A] a", "[B] hi", "[B] hi"]
    assert [msg_id for _, _, msg_id in pool.submitted] == [1, 2, 3]
    assert ov.pending_flags == [True, True, True]  # 佔位要標記，overlay 才會用較暗的顏色


def test_game_color_flows_to_overlay_text_to_context_and_pool(monkeypatch):
    # ChatLine 的顏色只進 overlay（對齊遊戲配色）；context 與翻譯只吃純文字
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    reads = [[ChatLine("[A] a", "#80ff00")], []]
    cache = FakeCache()
    pool, context = run_scripted(cfg, ov, reads, monkeypatch, cache=cache)
    assert ov.messages == [("[A] a", t("notice.pending"))]
    assert ov.colors == ["#80ff00"]
    assert pool.submitted == [("[A] a", [], 1)]
    assert context.snapshot() == ["[A] a"]
    assert cache.queried == []   # 玩家對話不查快取——快取只服務系統訊息


def test_context_advances_by_read_order_not_by_completion(monkeypatch):
    # 每則帶到的 context 是它「之前」的行——平行翻譯時同批訊息仍看得到彼此
    cfg = {"poll_interval": 0.01}
    reads = [["[A] one", "[B] two", "[C] three"], []]
    pool, context = run_scripted(cfg, FakeOverlay(), reads, monkeypatch)
    assert [ctx for _, ctx, _ in pool.submitted] == [
        [], ["[A] one"], ["[A] one", "[B] two"]]
    assert context.snapshot() == ["[A] one", "[B] two", "[C] three"]


def test_banner_follows_pool_error_state(monkeypatch):
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    pool = FakePool()
    pool.error_state = "offline"
    run_scripted(cfg, ov, [[], []], monkeypatch, pool=pool)
    assert ov.errors == ["notice.offline"]


def test_banner_prefers_game_issue_over_translation_error():
    # 連不上遊戲時翻譯狀態已無意義，橫幅顯示遊戲端的問題
    assert banner_for("notice.game_missing", "offline") == "notice.game_missing"
    assert banner_for("notice.access_denied", "offline") == "notice.access_denied"
    assert banner_for(None, "config") == "notice.config_error"
    assert banner_for(None, "offline") == "notice.offline"
    assert banner_for(None, None) is None


def test_status_shows_translating_while_pool_busy(monkeypatch):
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    pool = FakePool()
    pool.in_flight = 2
    run_scripted(cfg, ov, [[], []], monkeypatch, pool=pool)
    assert "translating" in ov.statuses


class StatusFakeReader(FakeReader):
    """依 poll 次數翻轉 pool.in_flight：模擬「送出翻譯後 pool 忙碌一輪、
    隨後轉回閒置」，藉此驗證狀態指示會從翻譯中降回監聽中（而不是卡住）。
    FakePool.in_flight 是純屬性，直接由這裡代替真正的 pool 翻轉。"""

    def __init__(self, reads, stop, pool, busy_at):
        super().__init__(reads, stop)
        self._pool = pool
        self._busy_at = busy_at

    def read_new(self):
        self._pool.in_flight = 1 if self.n == self._busy_at else 0
        return super().read_new()


def test_status_transitions(monkeypatch):
    # 監聽 →（有新訊息、pool 忙碌）翻譯中 → 監聽：完整三段都要出現，
    # 不能只停在「翻譯中」——回歸測試（重寫 reader_loop 時遺失的舊測試）。
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    pool = FakePool()
    reads = [[], ["[A] a"], []]
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    monkeypatch.setattr(main_module, "WizChatReader",
                        lambda **kw: StatusFakeReader(reads, stop, pool, busy_at=1))
    reader_loop(cfg, ov, ui_queue, stop, ChatContext(), pool)
    _drain(ui_queue)
    assert ov.statuses == ["listening", "translating", "listening"]


def test_status_locating_when_not_anchored(monkeypatch):
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    monkeypatch.setattr(main_module, "WizChatReader",
                        lambda **kw: FakeReader([[], []], stop, anchored=False))
    reader_loop(cfg, ov, ui_queue, stop, ChatContext(), FakePool())
    _drain(ui_queue)
    assert ov.statuses == ["locating"]  # 狀態未變不重複發


class InputFakeReader(FakeReader):
    """加上腳本化的遊戲輸入框開關狀態（每輪一個值）。"""

    def __init__(self, reads, stop, input_states):
        super().__init__(reads, stop)
        self.input_states = input_states

    def input_open(self):
        # read_new 已把 self.n 遞增，本輪狀態用 n-1 對應
        return self.input_states[min(self.n - 1, len(self.input_states) - 1)]


def _run_with_input(cfg, reads, input_states, monkeypatch):
    events = []
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    monkeypatch.setattr(main_module, "WizChatReader",
                        lambda **kw: InputFakeReader(reads, stop, input_states))
    reader_loop(cfg, FakeOverlay(), ui_queue, stop, ChatContext(), FakePool(),
                on_input_open=lambda: events.append("open"),
                on_input_close=lambda: events.append("close"))
    _drain(ui_queue)
    return events


def test_game_input_edge_triggers_open_and_close(monkeypatch):
    # 只在「關→開」與「開→關」的邊緣各觸發一次，持續開著不重複觸發
    cfg = {"poll_interval": 0.01, "auto_show_input": True}
    events = _run_with_input(cfg, [[], [], [], [], []],
                             [False, True, True, False, False], monkeypatch)
    assert events == ["open", "close"]


def test_game_input_detection_disabled_by_config(monkeypatch):
    cfg = {"poll_interval": 0.01, "auto_show_input": False}
    events = _run_with_input(cfg, [[], [], []],
                             [False, True, False], monkeypatch)
    assert events == []


def test_game_not_running_shows_banner_once(monkeypatch):
    monkeypatch.setattr(main_module, "GAME_MISSING_INTERVAL", 0.01)
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    reads = [GameNotRunning("no game"), GameNotRunning("no game")]
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    monkeypatch.setattr(main_module, "WizChatReader",
                        lambda **kw: FakeReader(reads, stop))
    reader_loop(cfg, ov, ui_queue, stop, ChatContext(), FakePool())
    _drain(ui_queue)
    assert ov.errors == ["notice.game_missing"]
    assert ov.messages == []
    assert "waiting_game" in ov.statuses


def test_access_denied_shows_its_own_banner_and_status(monkeypatch):
    # 權限不足與「找不到遊戲」是兩回事：橫幅要直接說出該以系統管理員身分重開
    monkeypatch.setattr(main_module, "GAME_MISSING_INTERVAL", 0.01)
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    reads = [GameAccessDenied("denied"), GameAccessDenied("denied")]
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    monkeypatch.setattr(main_module, "WizChatReader",
                        lambda **kw: FakeReader(reads, stop))
    reader_loop(cfg, ov, ui_queue, stop, ChatContext(), FakePool())
    _drain(ui_queue)
    assert ov.errors == ["notice.access_denied"]
    assert "access_denied" in ov.statuses


def test_version_mismatch_shows_its_own_banner_and_status(monkeypatch):
    # 掛入點與遊戲版本對不上，跟「遊戲沒開」長得一樣但解法完全不同：
    # 橫幅要說出該更新遊戲與本程式，而不是叫使用者繼續等遊戲
    monkeypatch.setattr(main_module, "GAME_MISSING_INTERVAL", 0.01)
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    reads = [GameVersionMismatch("pattern failed"), GameVersionMismatch("pattern failed")]
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    monkeypatch.setattr(main_module, "WizChatReader",
                        lambda **kw: FakeReader(reads, stop))
    reader_loop(cfg, ov, ui_queue, stop, ChatContext(), FakePool())
    _drain(ui_queue)
    assert ov.errors == ["notice.version_mismatch"]
    assert "version_mismatch" in ov.statuses


class DelayedInputReader(FakeReader):
    """輸入框在指定秒數後才變成開啟：模擬使用者在一輪的等待中途打開遊戲聊天欄。"""

    def __init__(self, reads, stop, opens_after):
        super().__init__(reads, stop)
        self.opens_after = opens_after
        self.started = time.monotonic()

    def input_open(self):
        return time.monotonic() - self.started >= self.opens_after


def test_game_input_detected_during_the_wait_between_polls(monkeypatch):
    # 聊天欄在兩輪讀取之間被打開：翻譯輸入框不該等到下一輪才彈出
    cfg = {"poll_interval": 0.5, "auto_show_input": True}
    opened_at = []
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    monkeypatch.setattr(main_module, "WizChatReader",
                        lambda **kw: DelayedInputReader([[], []], stop, 0.1))
    start = time.monotonic()
    reader_loop(cfg, FakeOverlay(), ui_queue, stop, ChatContext(), FakePool(),
                on_input_open=lambda: opened_at.append(time.monotonic() - start))
    _drain(ui_queue)
    assert opened_at, "沒有偵測到輸入框開啟"
    assert opened_at[0] < 0.25, f"延遲 {opened_at[0]:.3f}s，等到了下一輪讀取"
