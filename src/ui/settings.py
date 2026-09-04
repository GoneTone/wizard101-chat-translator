"""一般設定視窗：分「基本／進階」分頁，儲存即套用（不需重啟）。
遊戲路徑例外：重掛 hook 需重啟，儲存後提示下次啟動生效。
介面語言是唯一「改了就先看到」的欄位：換語言即時預覽（視窗以新語言重建、
常駐介面 relabel），但仍要按下儲存才寫進設定，取消則還原成開窗時的語言。"""
import copy
import os
import queue
import sys
import threading
import tkinter as tk
import webbrowser
from tkinter import filedialog, messagebox, ttk

from src import __version__
from src.config import (ADVANCED_LIMITS, DEFAULT_CONFIG, app_dir, app_name,
                        clamp_advanced)
from src.i18n import current_language, set_language, t
from src.ui.fields import (LINK_COLOR, ApiFields, HotkeyField, LanguageField,
                           UiLanguageField, link_label, poll_queue,
                           validate_api_form)
from src.ui.responsive import bind_wrap
from src.ui.scrollable import ScrollableFrame
from src.updater import AUTHOR_URL, PROJECT_URL, check_for_update

MIN_WIDTH = 640   # 視窗寬度下限：再窄欄位與說明會橫向擠壓，捲動救不了
MIN_HEIGHT = 360  # 視窗高度下限：內容可捲動，只需容得下分頁標籤、幾行欄位與按鈕列
# 說明文字換行時的右側預留：欄位自己的 grid padx（8）＋分頁內距（12）＋一點餘裕。
# 少扣了就會把說明的最後一兩個字切在視窗右緣外。
_HINT_TRAILING = 24
# 檢查更新結果的字色：沿用測試連線那組（成功綠、失敗紅），有新版用連結藍。
_UPDATE_COLORS = {"latest": "#2e8b57", "available": LINK_COLOR, "failed": "#cc3333"}
# 「關於」分頁的分組間距：版本／專案／開發者是唯讀資訊，紀錄檔與譯文快取是會動手的
# 維護項目，兩區之間拉開才不會被看成同一串條目。
_GROUP_GAP = 32


def parse_advanced_values(poll_var, fade_var, max_messages_var, type_delay_var,
                          alpha_var, parallel_var) -> tuple[dict | None, str | None]:
    """讀取並轉型進階數值 Tk 變數：使用者手動鍵入非數字時，Tk 變數的
    `.get()` 會拋 `TclError`，`int()`／`float()` 轉型也可能拋 `ValueError`——
    統一在此攔截並回傳 `（None， 錯誤文案 key）`，讓呼叫端走既有表單錯誤提示、
    不讓 cfg 被寫到一半。成功則回傳 `(clamp_advanced(...), None)`。"""
    try:
        values = clamp_advanced({
            "poll_interval": float(poll_var.get()),
            "fade_seconds": int(fade_var.get()),
            "max_messages": int(max_messages_var.get()),
            "type_delay": float(type_delay_var.get()),
            "overlay_alpha": float(alpha_var.get()),
            "max_parallel_translations": int(parallel_var.get()),
        })
    except (tk.TclError, ValueError):
        return None, "error.advanced_not_number"
    return values, None


class SettingsWindow:
    """設定視窗（單例）：open() 顯示或帶到前景；儲存時就地更新 cfg 並呼叫 on_save。"""

    def __init__(self, root: tk.Tk, cfg: dict, on_save, on_alpha_preview=None,
                 on_language_preview=None, check_update=check_for_update, cache=None):
        self._root = root
        self._cfg = cfg
        self._on_save = on_save
        self._on_alpha_preview = on_alpha_preview  # 拖滑桿即時套用透明度（預覽）
        self._on_language_preview = on_language_preview  # 讓常駐視窗跟上預覽中的語言
        self._cache = cache   # 譯文快取；None＝關於分頁不畫「清除快取」那一列
        self._check_update = check_update   # 可注入是為了測試，正式路徑用預設
        self._update_queue: queue.Queue = queue.Queue()
        self._win: tk.Toplevel | None = None
        # 未儲存的編輯暫存：欄位初始值都讀這裡，換語言重建視窗才不會弄丟填到一半的內容。
        # None＝目前沒有開著的編輯階段，下次 open() 重新從 cfg 取一份。
        self._draft: dict | None = None
        self._language_at_open: str | None = None
        self._restore_geometry: str | None = None   # 重建時沿用的視窗位置與大小
        self._restore_tab: int | None = None        # 重建時沿用的分頁

    def open(self) -> None:
        if self._win is not None and self._win.winfo_exists():
            self._win.lift()
            self._win.focus_force()
            return
        if self._draft is None:
            self._draft = copy.deepcopy(self._cfg)
            self._language_at_open = current_language()
        cfg = self._draft
        self._win = tk.Toplevel(self._root)
        self._win.title(t("settings.title", app=app_name()))
        # 開窗尺寸：量過「基本」分頁在英文下需要 512px 內容高（三種語言中最高），
        # 加上分頁標籤與按鈕列後 620 就放得下，取 640 再留一點給測試連線的結果訊息。
        # 寬度 720 讓各欄說明少換一兩行（640 時「進階」分頁的說明會多出約 40px）。
        win_w, win_h = 720, 640
        if self._restore_geometry is not None:
            self._win.geometry(self._restore_geometry)   # 換語言重建：不要跳回螢幕中央
        else:
            x = (self._win.winfo_screenwidth() - win_w) // 2
            y = (self._win.winfo_screenheight() - win_h) // 2
            self._win.geometry(f"{win_w}x{win_h}+{x}+{y}")
        self._win.resizable(True, True)
        # 下限比開窗尺寸小：使用者要縮小就讓他縮，內容捲動即可
        self._win.minsize(MIN_WIDTH, MIN_HEIGHT)
        self._win.attributes("-topmost", True)

        # 底部按鈕列先 pack：pack 依宣告順序分配空間，expand=True 的內容區若先宣告，
        # 會吃光剩餘高度，這條固定高度的按鈕列就會在視窗變矮時被擠扁甚至消失。
        btns = ttk.Frame(self._win, padding=(8, 0, 8, 8))
        btns.pack(side="bottom", fill="x")
        ttk.Label(btns, text=f"v{__version__}", foreground="#888888").pack(side="left")
        ttk.Button(btns, text=t("button.cancel"), command=self._cancel).pack(side="right")
        ttk.Button(btns, text=t("button.save"), command=self._save).pack(side="right",
                                                                        padx=(0, 8))

        nb = ttk.Notebook(self._win)
        nb.pack(fill="both", expand=True, padx=8, pady=8)
        self._nb = nb

        # --- 基本 ---
        basic_scroll = ScrollableFrame(nb, padding=12)
        basic = basic_scroll.body
        nb.add(basic_scroll, text=t("settings.tab.basic"))
        # 兩個語言設定放在一起：介面語言與翻譯目標語言是最容易被搞混的一對，
        # 相鄰擺放才看得出「這個管介面、那個管收到的訊息」。
        ttk.Label(basic, text=t("field.ui_language")).pack(anchor="w")
        self._ui_language = UiLanguageField(basic, current_language(),
                                            on_change=self._on_language_change)
        self._ui_language.pack(fill="x", pady=(2, 10))
        ttk.Label(basic, text=t("settings.target_language")).pack(anchor="w")
        self._language = LanguageField(
            basic, cfg["target_language"],
            on_change=lambda: self._api.clear_test_result())
        self._language.pack(fill="x", pady=(2, 10))
        self._api = ApiFields(basic, cfg["api"])
        self._api.pack(fill="x")
        self._api.set_target_language_fn(lambda: self._language.value())
        ttk.Label(basic, text=t("settings.hotkey")).pack(anchor="w", pady=(12, 0))
        self._hotkey = HotkeyField(basic, cfg["hotkey"])
        self._hotkey.pack(anchor="w", pady=(2, 0))
        self._auto_input = tk.BooleanVar(value=cfg["auto_show_input"])
        ttk.Checkbutton(basic, text=t("field.auto_input"),
                        variable=self._auto_input).pack(anchor="w", pady=(10, 0))

        # --- 進階 ---
        adv_scroll = ScrollableFrame(nb, padding=12)
        adv = adv_scroll.body
        nb.add(adv_scroll, text=t("settings.tab.advanced"))
        # grid 而非 pack：標籤欄的寬度由最長的那一條決定，各列自然對齊。
        # 原本用固定字元寬（width=14）對齊，中文塞得下、英文會被裁掉
        # （"Message fade-out (s)"、"Parallel translations"）。
        adv.columnconfigure(2, weight=1)   # 說明欄吃掉剩餘寬度
        self._poll = self._spin(adv, 0, "settings.poll_interval", cfg["poll_interval"],
                                "poll_interval", 0.1, "settings.poll_interval_hint")
        self._fade = self._spin(adv, 1, "settings.fade", cfg["fade_seconds"],
                                "fade_seconds", 10, "settings.fade_hint")
        self._max_msgs = self._spin(adv, 2, "settings.max_messages",
                                    cfg["max_messages"], "max_messages", 10,
                                    "settings.max_messages_hint")
        self._parallel = self._spin(adv, 3, "settings.parallel",
                                    cfg["max_parallel_translations"],
                                    "max_parallel_translations", 1,
                                    "settings.parallel_hint")
        self._type_delay = self._spin(adv, 4, "settings.type_delay", cfg["type_delay"],
                                      "type_delay", 0.01, "settings.type_delay_hint")
        self._alpha_var = self._alpha_slider(adv, 5, cfg["overlay_alpha"])

        self._translate_system = tk.BooleanVar(value=cfg["translate_system_messages"])
        ttk.Checkbutton(adv, text=t("field.translate_system"),
                        variable=self._translate_system).grid(
            row=6, column=0, columnspan=3, sticky="w", pady=(10, 2))

        ttk.Label(adv, text=t("settings.game_path")).grid(row=7, column=0, sticky="w",
                                                          pady=(10, 2))
        # 滑桿與路徑列跨欄放進自己的 Frame：它們比 Spinbox 寬得多，
        # 讓它們獨占 column 1 會把每一列的數值欄都撐開、右邊拉出一大片空白。
        path_row = ttk.Frame(adv)
        path_row.grid(row=7, column=1, columnspan=2, sticky="ew", padx=(8, 0),
                      pady=(10, 2))
        self._game_path_row = path_row   # 版面順序測試取得這一列的入口
        self._game_path = tk.StringVar(value=cfg["game_path"] or "")
        # 「瀏覽…」先 pack：expand=True 的輸入框若先宣告會吃光整列寬度，
        # 這顆固定寬度的按鈕就會在視窗變窄時被擠掉。
        ttk.Button(path_row, text=t("button.browse"), width=7,
                   command=self._browse_game_path).pack(side="right", padx=(4, 0))
        ttk.Entry(path_row, textvariable=self._game_path).pack(
            side="left", fill="x", expand=True)
        hint = ttk.Label(adv, text=t("settings.game_path_hint"), foreground="#888888",
                         justify="left")
        hint.grid(row=8, column=0, columnspan=3, sticky="ew")
        bind_wrap(hint, trailing=_HINT_TRAILING)

        self._build_about(nb)

        if self._restore_tab is not None:
            nb.select(self._restore_tab)
        self._restore_geometry = self._restore_tab = None
        self._win.protocol("WM_DELETE_WINDOW", self._cancel)

    def _build_about(self, nb) -> None:
        """關於分頁：版本與手動檢查更新、專案與開發者連結、紀錄檔位置。"""
        about_scroll = ScrollableFrame(nb, padding=12)
        about = about_scroll.body
        nb.add(about_scroll, text=t("settings.tab.about"))
        about.columnconfigure(1, weight=1)

        ttk.Label(about, text=t("about.version")).grid(row=0, column=0, sticky="w",
                                                       pady=2)
        version_row = ttk.Frame(about)
        version_row.grid(row=0, column=1, sticky="ew", padx=(8, 0), pady=2)
        self._version_label = ttk.Label(version_row, text=f"v{__version__}")
        self._version_label.pack(side="left")
        self._update_btn = ttk.Button(version_row, text=t("button.check_update"),
                                      command=self._start_update_check)
        self._update_btn.pack(side="left", padx=(8, 0))
        self._update_result = ttk.Label(version_row, text="")
        # 不 fill／expand：「有新版」時整個標籤是可點的連結，撐滿整列會讓文字
        # 後面那段空白也跟著可點、游標也變成手指。換行寬度仍由 bind_wrap 依
        # 這一列的寬度算，長訊息（檢查失敗帶例外訊息）照樣折行。
        self._update_result.pack(side="left", padx=8)
        bind_wrap(self._update_result)

        ttk.Label(about, text=t("about.project")).grid(row=1, column=0, sticky="w",
                                                       pady=2)
        self._project_link = link_label(about, PROJECT_URL, PROJECT_URL)
        self._project_link.grid(row=1, column=1, sticky="w", padx=(8, 0), pady=2)

        ttk.Label(about, text=t("about.author")).grid(row=2, column=0, sticky="w",
                                                      pady=2)
        # 開發者名稱是識別碼不是文案，不進語言檔（與服務商品牌名同理）
        self._author_link = link_label(about, "GoneTone", AUTHOR_URL)
        self._author_link.grid(row=2, column=1, sticky="w", padx=(8, 0), pady=2)

        # 維護區的起點：與上面的唯讀資訊拉開（見 _GROUP_GAP）
        ttk.Label(about, text=t("about.logs")).grid(row=3, column=0, sticky="w",
                                                    pady=(_GROUP_GAP, 2))
        logs_row = ttk.Frame(about)
        logs_row.grid(row=3, column=1, sticky="ew", padx=(8, 0), pady=(_GROUP_GAP, 2))
        # 按鈕先 pack：expand=True 的路徑標籤若先宣告會吃光整列，把按鈕擠掉
        ttk.Button(logs_row, text=t("button.open_folder"),
                   command=self._open_log_folder).pack(side="right", padx=(4, 0))
        self._logs_label = ttk.Label(logs_row, text=str(app_dir()))
        self._logs_label.pack(side="left", fill="x", expand=True)
        logs_hint = ttk.Label(about, text=t("about.logs_hint"), foreground="#888888",
                              justify="left")
        logs_hint.grid(row=4, column=0, columnspan=2, sticky="ew")
        bind_wrap(logs_hint, trailing=_HINT_TRAILING)

        if self._cache is not None:   # None＝呼叫端沒有快取（測試與早期啟動路徑）
            ttk.Label(about, text=t("about.cache")).grid(row=5, column=0, sticky="w",
                                                         pady=(10, 2))
            cache_row = ttk.Frame(about)
            cache_row.grid(row=5, column=1, sticky="ew", padx=(8, 0), pady=(10, 2))
            ttk.Button(cache_row, text=t("button.clear_cache"),
                       command=self._clear_cache).pack(side="left")
            self._cache_result = ttk.Label(cache_row, text="")
            self._cache_result.pack(side="left", padx=(8, 0))
            cache_hint = ttk.Label(about, text=t("about.cache_hint"),
                                   foreground="#888888", justify="left")
            cache_hint.grid(row=6, column=0, columnspan=2, sticky="ew")
            bind_wrap(cache_hint, trailing=_HINT_TRAILING)

    def _clear_cache(self) -> None:
        """清掉譯文快取並回報筆數。不加確認對話框：快取會自動重建，
        誤按的代價只是下一則同樣的系統訊息重翻一次。"""
        count = self._cache.clear()
        self._cache_result.configure(text=t("about.cache_cleared", count=count))
        print(f"[settings] translation cache cleared by user ({count} entries)",
              file=sys.stderr)

    def _start_update_check(self) -> None:
        """手動檢查更新：背景查詢，結果經 queue 交回主執行緒顯示（見 poll_queue）。

        每次按下都重建 queue：SettingsWindow 整個 app 生命週期只有一個實例，
        `self._update_queue` 若只在 __init__ 建一次，視窗在結果送回前被關掉
        （或換語言 `_rebuild`）就會讓 poll_queue 停止輪詢，結果留在舊 queue 裡；
        下次檢查沿用同一個 queue，會先撈到那筆過期結果，而不是這一輪的。

        queue 同時以參數交給 worker，而不是讓它回頭讀 `self._update_queue`：
        前一輪沒回來的 worker 一旦查到的是新那一輪的 queue，過期結果照樣會被
        撈走顯示——重建 queue 只擋掉視窗重開這條路徑，擋不掉兩輪重疊。"""
        result_queue: queue.Queue = queue.Queue()
        self._update_queue = result_queue   # 這一輪的通道（poll_queue 與測試取用）
        self._update_btn.configure(state="disabled", text=t("button.checking"))
        self._update_result.configure(text="")
        threading.Thread(target=self._update_check_worker, args=(result_queue,),
                         daemon=True).start()
        poll_queue(self._win, result_queue, self._on_update_checked)

    def _update_check_worker(self, result_queue: queue.Queue) -> None:
        try:
            release = self._check_update()
        except Exception as exc:
            print(f"[update] manual check failed: {exc}", file=sys.stderr)
            result_queue.put(("failed", t("update.failed", error=exc), None))
            return
        if release is None:
            print("[update] manual check: already up to date", file=sys.stderr)
            result_queue.put(("latest", t("update.latest"), None))
            return
        print(f"[update] manual check: {release.version} available", file=sys.stderr)
        result_queue.put(("available",
                          t("update.available", version=release.version),
                          release.url))

    def _on_update_checked(self, result) -> None:
        state, message, url = result
        self._update_btn.configure(state="normal", text=t("button.check_update"))
        prefix = {"latest": "✓ ", "failed": "✗ ", "available": ""}[state]
        self._update_result.configure(text=prefix + message,
                                      foreground=_UPDATE_COLORS[state],
                                      cursor="hand2" if url else "")
        self._update_result.unbind("<Button-1>")
        if url:
            self._update_result.bind("<Button-1>", lambda e: webbrowser.open(url))

    def _open_log_folder(self) -> None:
        """開啟 app.log／messages.log 所在的資料夾（Windows 檔案總管）。"""
        path = app_dir()
        try:
            os.startfile(path)
        except OSError as exc:
            print(f"[ui] open log folder failed: path={path} error={exc}",
                  file=sys.stderr)

    def _alpha_slider(self, parent, grid_row: int, initial: float) -> tk.DoubleVar:
        """視窗不透明度滑桿：拖動即時預覽（套到 overlay 與泡泡），儲存才寫入設定。"""
        lo, hi = ADVANCED_LIMITS["overlay_alpha"]
        ttk.Label(parent, text=t("settings.alpha")).grid(row=grid_row, column=0,
                                                        sticky="w", pady=2)
        # 滑桿比 Spinbox 寬得多：跨欄放進自己的 Frame，才不會把每一列的數值欄都撐開
        holder = ttk.Frame(parent)
        holder.grid(row=grid_row, column=1, columnspan=2, sticky="ew", padx=(8, 0),
                    pady=2)
        var = tk.DoubleVar(value=initial)
        value_label = ttk.Label(holder, text=f"{initial:.2f}", width=5)

        def on_slide(raw: str) -> None:
            v = round(float(raw), 2)
            var.set(v)
            value_label.configure(text=f"{v:.2f}")
            if self._on_alpha_preview is not None:
                self._on_alpha_preview(v)

        ttk.Scale(holder, from_=lo, to=hi, orient="horizontal", variable=var,
                  command=on_slide, length=160).pack(side="left")
        value_label.pack(side="left", padx=(6, 0))
        note = ttk.Label(holder, text=t("settings.alpha_hint",
                                        default=DEFAULT_CONFIG["overlay_alpha"]),
                         foreground="#888888", justify="left")
        note.pack(side="left", fill="x", expand=True, padx=8)
        bind_wrap(note)
        return var

    def _on_language_change(self, code: str) -> None:
        """介面語言換了：立刻以新語言預覽，但不寫 cfg（按儲存才算數）。

        整個視窗重建、而非逐一 reconfigure 標籤：欄位元件各自帶著舊語言的文字，
        逐一刷新容易漏掉（精靈也是這麼做的）。代價是 API 測試結果會被清掉——
        那句譯文本來就綁著當時的語言。"""
        if code == current_language():
            return
        self._collect_into_draft()
        self._restore_geometry = self._win.geometry()
        self._restore_tab = self._nb.index("current")
        print(f"[ui] settings previewing language {code}", file=sys.stderr)
        self._preview_language(code)
        # after_idle：這裡是從 <<ComboboxSelected>> 事件內呼叫，ttk 的類別 binding
        # 還在處理同一個事件，立即 destroy() 會讓它收尾時對已死的 widget 操作，
        # 冒出 TclError: invalid command name。延到事件處理完才重建視窗。
        self._win.after_idle(self._rebuild)

    def _preview_language(self, code: str) -> None:
        """套用預覽語言：設定視窗之外，常駐的 overlay 也要跟著換（由呼叫端提供）。"""
        set_language(code)
        if self._on_language_preview is not None:
            self._on_language_preview()

    def _rebuild(self) -> None:
        self._win.destroy()
        self._win = None
        self.open()

    def _collect_into_draft(self) -> None:
        """把目前填在欄位裡的值寫回 draft，供重建視窗時復原。

        進階數值鍵到一半是非數字時就保留 draft 原值：重建不是儲存，
        不該在這裡把使用者擋在表單錯誤提示前面。"""
        draft = self._draft
        draft["api"] = self._api.get_values()
        if self._language.value():
            draft["target_language"] = self._language.value()
        draft["hotkey"] = self._hotkey.value()
        draft["auto_show_input"] = self._auto_input.get()
        draft["translate_system_messages"] = self._translate_system.get()
        draft["game_path"] = self._game_path.get().strip() or None
        advanced, _error = parse_advanced_values(
            self._poll, self._fade, self._max_msgs, self._type_delay, self._alpha_var,
            self._parallel)
        if advanced is not None:
            draft.update(advanced)

    def _cancel(self) -> None:
        """取消／關窗：把預覽中的透明度與介面語言都還原為目前設定值。"""
        if self._on_alpha_preview is not None:
            self._on_alpha_preview(self._cfg["overlay_alpha"])
        if (self._language_at_open is not None
                and current_language() != self._language_at_open):
            print(f"[ui] settings language preview reverted to "
                  f"{self._language_at_open}", file=sys.stderr)
            self._preview_language(self._language_at_open)
        self._draft = None
        self._win.destroy()

    def _spin(self, parent, grid_row, label_key, initial, key, step, hint_key):
        """進階數值的一列：標籤、Spinbox、範圍說明各佔 grid 的一欄。"""
        lo, hi = ADVANCED_LIMITS[key]
        ttk.Label(parent, text=t(label_key)).grid(row=grid_row, column=0, sticky="w",
                                                  pady=2)
        var = tk.DoubleVar(value=initial) if isinstance(initial, float) \
            else tk.IntVar(value=initial)
        ttk.Spinbox(parent, textvariable=var, from_=lo, to=hi, increment=step,
                    width=8).grid(row=grid_row, column=1, sticky="w", padx=(8, 0),
                                  pady=2)
        note = ttk.Label(parent, text=t("settings.range_hint", hint=t(hint_key), lo=lo,
                                        hi=hi, default=DEFAULT_CONFIG[key]),
                         foreground="#888888", justify="left")
        note.grid(row=grid_row, column=2, sticky="ew", padx=8, pady=2)
        bind_wrap(note, trailing=_HINT_TRAILING)
        return var

    def _browse_game_path(self) -> None:
        chosen = filedialog.askdirectory(parent=self._win)
        if chosen:
            self._game_path.set(chosen)

    def _save(self) -> None:
        # 進階數值先解析：格式錯誤也要走表單錯誤提示，不能讓 cfg 寫到一半。
        advanced, advanced_error = parse_advanced_values(
            self._poll, self._fade, self._max_msgs, self._type_delay, self._alpha_var,
            self._parallel)
        api = self._api.get_values()
        errors = validate_api_form(self._api.active_values())
        if not self._language.value():
            errors.append("error.need_target_language")
        if advanced_error:
            errors.append(advanced_error)
        if errors:
            messagebox.showwarning(t("dialog.incomplete_title"),
                                   "\n".join(t(e) for e in errors), parent=self._win)
            return
        cfg = self._cfg
        old_game_path = cfg["game_path"]
        cfg["api"] = api
        # 語言要在 on_save 之前套用：apply_settings 會依新語言重繪 overlay。
        cfg["ui_language"] = self._ui_language.value()
        set_language(cfg["ui_language"])
        cfg["target_language"] = self._language.value()
        cfg["hotkey"] = self._hotkey.value()
        cfg["auto_show_input"] = self._auto_input.get()
        cfg["translate_system_messages"] = self._translate_system.get()
        cfg.update(advanced)
        cfg["game_path"] = self._game_path.get().strip() or None
        game_path_changed = cfg["game_path"] != old_game_path
        self._on_save()
        if game_path_changed:
            messagebox.showinfo(t("dialog.notice_title"), t("dialog.game_path_restart"),
                                parent=self._win)
        self._draft = None
        self._win.destroy()
