"""一般設定視窗：分「基本／進階」分頁，儲存即套用（不需重啟）。
遊戲路徑例外：重掛 hook 需重啟，儲存後提示下次啟動生效。
介面語言是唯一「改了就先看到」的欄位：換語言即時預覽（視窗以新語言重建、
常駐介面 relabel），但仍要按下儲存才寫進設定，取消則還原成開窗時的語言。"""
import copy
import os
import tkinter as tk
import webbrowser
from tkinter import filedialog, messagebox, ttk

from src import __version__
from src.config import ADVANCED_LIMITS, DEFAULT_CONFIG, app_dir, app_name, clamp_advanced
from src.i18n import current_language, set_language, t, translators
from src.log import log
from src.services import find, validate_service
from src.ui.fields import HotkeyField, LanguageField, UiLanguageField
from src.ui.form import (
    HINT_COLOR,
    BackgroundButton,
    help_translate_link,
    hint_label,
    link_label,
    linked_text,
    show_outcome,
    translators_row,
)
from src.ui.geometry import centered_position
from src.ui.responsive import HINT_TRAILING, bind_wrap
from src.ui.richtext import LINK_COLOR
from src.ui.scrollable import ScrollableFrame
from src.ui.service_list import ServicePane
from src.updater import AUTHOR_URL, CROWDIN_URL, ISSUES_URL, PROJECT_URL, check_for_update

MIN_WIDTH = 640   # 視窗寬度下限：再窄欄位與說明會橫向擠壓，捲動救不了
MIN_HEIGHT = 360  # 視窗高度下限：內容可捲動，只需容得下分頁標籤、幾行欄位與按鈕列
# 「關於」分頁裡唯讀資訊與維護項目之間的間距：拉開才不會被看成同一串條目
_GROUP_GAP = 32


def parse_advanced_values(poll_var, fade_var, max_messages_var, type_delay_var,
                          alpha_var, parallel_var) -> tuple[dict | None, str | None]:
    """讀取並轉型進階數值的 Tk 變數。手動鍵入非數字時 `.get()` 拋 `TclError`、
    轉型拋 `ValueError`，統一攔下回傳 `(None, 錯誤文案 key)` 讓呼叫端走表單錯誤提示，
    不讓 cfg 寫到一半；成功回傳 `(clamp_advanced(...), None)`。"""
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
                 on_language_preview=None, on_update_found=None,
                 check_update=check_for_update, cache=None):
        self._root = root
        self._cfg = cfg
        self._on_save = on_save
        self._on_alpha_preview = on_alpha_preview  # 拖滑桿即時套用透明度（預覽）
        self._on_language_preview = on_language_preview  # 讓常駐視窗跟上預覽中的語言
        self._on_update_found = on_update_found  # 手動檢查查到新版時通知（overlay 顯示橫幅）
        self._cache = cache   # 譯文快取；None＝關於分頁不畫「清除快取」那一列
        self._check_update = check_update   # 可注入是為了測試，正式路徑用預設
        self._win: tk.Toplevel | None = None
        # 未儲存的編輯暫存：欄位初始值讀這裡，換語言重建視窗才不會丟掉填到一半的內容；
        # None＝沒有開著的編輯階段，下次 open() 重新從 cfg 取一份。
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
        # 開窗尺寸：「基本」分頁在英文下（三種語言中最高）要 512px 內容高，加上分頁標籤
        # 與按鈕列 620 放得下，640 再留給測試連線的結果訊息；寬 720 讓說明少換一兩行
        # （640 時「進階」分頁的說明多出約 40px）。
        win_w, win_h = 720, 640
        if self._restore_geometry is not None:
            self._win.geometry(self._restore_geometry)   # 換語言重建：不要跳回螢幕中央
        else:
            x, y = centered_position(self._win.winfo_screenwidth(),
                                     self._win.winfo_screenheight(), win_w, win_h)
            self._win.geometry(f"{win_w}x{win_h}+{x}+{y}")
        self._win.resizable(True, True)
        # 下限比開窗尺寸小：內容可捲動，使用者要縮就讓他縮
        self._win.minsize(MIN_WIDTH, MIN_HEIGHT)
        # 抬到最前面一次就放掉：設定是從 topmost 的 overlay（⚙）叫出來的，不抬會被它
        # 蓋住、看起來像沒反應；但設定視窗只是一般視窗，不該一直壓在其他程式之上。
        self._win.attributes("-topmost", True)
        self._win.after_idle(self._release_topmost, self._win)

        # 按鈕列先 pack：後宣告會在視窗變矮時被 expand=True 的內容區擠掉
        btns = ttk.Frame(self._win, padding=(8, 0, 8, 8))
        btns.pack(side="bottom", fill="x")
        ttk.Label(btns, text=f"v{__version__}", foreground=HINT_COLOR).pack(side="left")
        ttk.Button(btns, text=t("button.cancel"), command=self._cancel).pack(side="right")
        ttk.Button(btns, text=t("button.save"), command=self._save).pack(side="right",
                                                                        padx=(0, 8))

        nb = ttk.Notebook(self._win)
        nb.pack(fill="both", expand=True, padx=8, pady=8)
        self._nb = nb

        self._build_basic(nb, cfg)
        self._build_services(nb, cfg)
        self._build_advanced(nb, cfg)
        self._build_about(nb)

        if self._restore_tab is not None:
            nb.select(self._restore_tab)
        self._restore_geometry = self._restore_tab = None
        self._win.protocol("WM_DELETE_WINDOW", self._cancel)

    def _release_topmost(self, win: tk.Toplevel) -> None:
        """放掉開窗時暫時抬起用的 topmost（見 open()）。
        排進 idle 佇列後才跑，期間視窗可能已被關掉或因換語言重建，故先確認還在。"""
        if win.winfo_exists():
            win.attributes("-topmost", False)

    def _build_basic(self, nb, cfg: dict) -> None:
        """基本分頁：介面語言、翻譯目標語言、熱鍵、自動呼出輸入框。"""
        basic_scroll = ScrollableFrame(nb, padding=12)
        basic = basic_scroll.body
        nb.add(basic_scroll, text=t("settings.tab.basic"))
        # 介面語言與翻譯目標語言最容易被搞混，相鄰擺放才看得出各管什麼
        ttk.Label(basic, text=t("field.ui_language")).pack(anchor="w")
        self._ui_language = UiLanguageField(basic, current_language(),
                                            on_change=self._on_language_change)
        # 掛名屬於選到的這個語言，緊貼在下拉之下才看得出對應關係；
        # 邀請協助翻譯的連結接在掛名之下，沒有掛名也照樣顯示 —— 沒人翻的語言更需要
        self._translators_row = translators_row(basic)
        self._ui_language.pack(fill="x", pady=(2, 4))
        if self._translators_row is not None:
            self._translators_row.pack(anchor="w", pady=(0, 2))
        self._help_translate_link = help_translate_link(basic)
        self._help_translate_link.pack(anchor="w", pady=(0, 10))
        ttk.Label(basic, text=t("settings.target_language")).pack(anchor="w")
        self._language = LanguageField(basic, cfg["target_language"])
        self._language.pack(fill="x", pady=(2, 10))
        ttk.Label(basic, text=t("settings.hotkey")).pack(anchor="w", pady=(12, 0))
        self._hotkey = HotkeyField(basic, cfg["hotkey"])
        self._hotkey.pack(anchor="w", pady=(2, 0))
        # 每把熱鍵底下緊接著它自己的選項，看得出哪個開關管哪個功能
        self._auto_input = tk.BooleanVar(value=cfg["auto_show_input"])
        ttk.Checkbutton(basic, text=t("field.auto_input"),
                        variable=self._auto_input).pack(anchor="w", pady=(6, 0))
        ttk.Label(basic, text=t("settings.region_hotkey")).pack(anchor="w", pady=(10, 0))
        self._region_hotkey = HotkeyField(basic, cfg["region_hotkey"])
        self._region_hotkey.pack(anchor="w", pady=(2, 0))
        self._paste_hotkey = tk.BooleanVar(value=cfg["paste_hotkey"])
        ttk.Checkbutton(basic, text=t("field.paste_hotkey"),
                        variable=self._paste_hotkey).pack(anchor="w", pady=(4, 0))

    def _build_services(self, nb, cfg: dict) -> None:
        """翻譯服務分頁：預設服務、用途分派與服務清單。"""
        scroll = ScrollableFrame(nb, padding=12)
        nb.add(scroll, text=t("settings.tab.services"))
        self._services = ServicePane(scroll.body, cfg,
                                     target_language_fn=lambda: self._language.value())
        self._services.pack(fill="x")

    def _build_advanced(self, nb, cfg: dict) -> None:
        """進階分頁：數值參數、不透明度、系統訊息開關、遊戲路徑。"""
        adv_scroll = ScrollableFrame(nb, padding=12)
        adv = adv_scroll.body
        nb.add(adv_scroll, text=t("settings.tab.advanced"))
        # grid 而非 pack：標籤欄寬度由最長的一條決定；固定字元寬（width=14）中文塞得下、
        # 英文（"Message fade-out (s)"）會被裁掉。
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
        # 路徑列比 Spinbox 寬得多：跨欄放進自己的 Frame，才不會把每一列的數值欄都撐開
        path_row = ttk.Frame(adv)
        path_row.grid(row=7, column=1, columnspan=2, sticky="ew", padx=(8, 0),
                      pady=(10, 2))
        self._game_path_row = path_row   # 版面順序測試取得這一列的入口
        self._game_path = tk.StringVar(value=cfg["game_path"] or "")
        # 「瀏覽…」先 pack：後宣告會在視窗變窄時被 expand=True 的輸入框擠掉
        ttk.Button(path_row, text=t("button.browse"), width=7,
                   command=self._browse_game_path).pack(side="right", padx=(4, 0))
        ttk.Entry(path_row, textvariable=self._game_path).pack(
            side="left", fill="x", expand=True)
        hint_label(adv, t("settings.game_path_hint"), trailing=HINT_TRAILING).grid(
            row=8, column=0, columnspan=3, sticky="ew")

    def _build_about(self, nb) -> None:
        """關於分頁：版本與手動檢查更新、專案與開發者連結、譯者、協助翻譯、紀錄檔位置。

        列號用遞增的 row 而非寫死的數字：譯者與快取都是可有可無的列，
        寫死的話每插一列就要把後面全部重編。"""
        about_scroll = ScrollableFrame(nb, padding=12)
        about = about_scroll.body
        nb.add(about_scroll, text=t("settings.tab.about"))
        about.columnconfigure(1, weight=1)
        row = 0

        ttk.Label(about, text=t("about.version")).grid(row=row, column=0, sticky="w",
                                                       pady=2)
        version_row = ttk.Frame(about)
        version_row.grid(row=row, column=1, sticky="ew", padx=(8, 0), pady=2)
        self._version_label = ttk.Label(version_row, text=f"v{__version__}")
        self._version_label.pack(side="left")
        self._update_btn = ttk.Button(version_row, text=t("button.check_update"),
                                      command=self._start_update_check)
        self._update_btn.pack(side="left", padx=(8, 0))
        # 每次重建視窗都是新的一輪：舊視窗還沒回來的結果留在舊按鈕那一輪，不會落到這裡
        self._update_task = BackgroundButton(self._update_btn, "update check")
        self._update_result = ttk.Label(version_row, text="")
        # 不 fill／expand：「有新版」時整個標籤是連結，撐滿整列會讓文字後的空白也可點；
        # 換行寬度仍由 bind_wrap 依這一列的寬度算，長訊息照樣折行。
        self._update_result.pack(side="left", padx=8)
        bind_wrap(self._update_result)

        row += 1
        ttk.Label(about, text=t("about.project")).grid(row=row, column=0, sticky="w",
                                                       pady=2)
        self._project_link = link_label(about, PROJECT_URL, PROJECT_URL)
        self._project_link.grid(row=row, column=1, sticky="w", padx=(8, 0), pady=2)

        row += 1
        ttk.Label(about, text=t("about.author")).grid(row=row, column=0, sticky="w",
                                                      pady=2)
        # 開發者名稱是識別碼不是文案，不進語言檔（與服務商品牌名同理）
        self._author_link = link_label(about, "GoneTone", AUTHOR_URL)
        self._author_link.grid(row=row, column=1, sticky="w", padx=(8, 0), pady=2)

        credit = translators(current_language())
        self._about_translators = None
        if credit:   # 空白＝這份譯文沒有譯者掛名（來源語言就是如此），整列不畫
            row += 1
            ttk.Label(about, text=t("credit.translators")).grid(row=row, column=0,
                                                                sticky="w", pady=2)
            self._about_translators = linked_text(about, credit)
            self._about_translators.grid(row=row, column=1, sticky="w", padx=(8, 0),
                                         pady=2)

        row += 1
        ttk.Label(about, text=t("about.help_translate")).grid(
            row=row, column=0, sticky="w", pady=2)
        self._contribute_link = link_label(about, CROWDIN_URL, CROWDIN_URL)
        self._contribute_link.grid(row=row, column=1, sticky="w", padx=(8, 0), pady=2)

        row += 1
        ttk.Label(about, text=t("about.issues")).grid(row=row, column=0, sticky="w",
                                                      pady=2)
        self._issues_link = link_label(about, ISSUES_URL, ISSUES_URL)
        self._issues_link.grid(row=row, column=1, sticky="w", padx=(8, 0), pady=2)

        row += 1
        ttk.Label(about, text=t("about.logs")).grid(row=row, column=0, sticky="w",
                                                    pady=(_GROUP_GAP, 2))
        logs_row = ttk.Frame(about)
        logs_row.grid(row=row, column=1, sticky="ew", padx=(8, 0), pady=(_GROUP_GAP, 2))
        # 按鈕先 pack：後宣告會被 expand=True 的路徑標籤擠掉
        ttk.Button(logs_row, text=t("button.open_folder"),
                   command=self._open_log_folder).pack(side="right", padx=(4, 0))
        self._logs_label = ttk.Label(logs_row, text=str(app_dir()))
        self._logs_label.pack(side="left", fill="x", expand=True)
        row += 1
        hint_label(about, t("about.logs_hint"), trailing=HINT_TRAILING).grid(
            row=row, column=0, columnspan=2, sticky="ew")

        if self._cache is not None:   # None＝呼叫端沒有快取（測試與早期啟動路徑）
            row += 1
            ttk.Label(about, text=t("about.cache")).grid(row=row, column=0, sticky="w",
                                                         pady=(10, 2))
            cache_row = ttk.Frame(about)
            cache_row.grid(row=row, column=1, sticky="ew", padx=(8, 0), pady=(10, 2))
            ttk.Button(cache_row, text=t("button.clear_cache"),
                       command=self._clear_cache).pack(side="left")
            self._cache_result = ttk.Label(cache_row, text="")
            self._cache_result.pack(side="left", padx=(8, 0))
            hint_label(about, t("about.cache_hint"), trailing=HINT_TRAILING).grid(
                row=row + 1, column=0, columnspan=2, sticky="ew")

    def _clear_cache(self) -> None:
        """清掉譯文快取並回報筆數。不加確認對話框：快取會自動重建，
        誤按的代價只是下一則同樣的系統訊息重翻一次。"""
        count = self._cache.clear()
        self._cache_result.configure(text=t("about.cache_cleared", count=count))
        log(f"[settings] translation cache cleared by user ({count} entries)")

    def _start_update_check(self) -> None:
        """手動檢查更新：背景查詢，結果回主執行緒顯示（見 form.BackgroundButton）。"""
        self._update_result.configure(text="")
        self._update_task.start(self._check_update, self._on_update_checked,
                                t("button.checking"))

    def _on_update_checked(self, result) -> None:
        """檢查更新的結果：Release（有新版）、None（已是最新）或拋出的例外。"""
        self._update_result.unbind("<Button-1>")
        self._update_result.configure(cursor="")
        if isinstance(result, Exception):
            log(f"[update] manual check failed: {result}")
            show_outcome(self._update_result, False, t("update.failed", error=result))
            return
        if result is None:
            log("[update] manual check: already up to date")
            show_outcome(self._update_result, True, t("update.latest"))
            return
        release = result
        log(f"[update] manual check: {release.version} available")
        # 有新版：整個標籤是可點的連結，用連結藍、不加 ✓／✗ 前綴
        self._update_result.configure(text=t("update.available", version=release.version),
                                      foreground=LINK_COLOR, cursor="hand2")
        self._update_result.bind("<Button-1>", lambda e: webbrowser.open(release.url))
        if self._on_update_found is not None:
            self._on_update_found(release)

    def _open_log_folder(self) -> None:
        path = app_dir()
        try:
            os.startfile(path)
        except OSError as exc:
            log(f"[ui] open log folder failed: path={path} error={exc}")

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
        hint_label(holder, t("settings.alpha_hint", default=DEFAULT_CONFIG["overlay_alpha"])
                   ).pack(side="left", fill="x", expand=True, padx=8)
        return var

    def _on_language_change(self, code: str) -> None:
        """介面語言換了：立刻以新語言預覽，不寫 cfg（按儲存才算數）。
        整個視窗重建而非逐一 relabel：欄位元件各自帶著舊語言的文字，逐一刷新容易漏
        （精靈同理）；代價是 API 測試結果被清掉 —— 那句譯文本來就綁著當時的語言。"""
        if code == current_language():
            return
        self._collect_into_draft()
        self._restore_geometry = self._win.geometry()
        self._restore_tab = self._nb.index("current")
        log(f"[ui] settings previewing language {code}")
        set_language(code)
        # after_idle：此處在 <<ComboboxSelected>> 事件內，ttk 類別 binding 還在處理同一事件，
        # 立即 destroy() 會讓它收尾時碰到已死的 widget（TclError: invalid command name）
        self._win.after_idle(self._rebuild)

    def _preview_language(self, code: str) -> None:
        """套用預覽語言；常駐的 overlay 也要跟著換（由呼叫端提供）。"""
        set_language(code)
        self._relabel_overlay()

    def _relabel_overlay(self) -> None:
        if self._on_language_preview is not None:
            self._on_language_preview()

    def _rebuild(self) -> None:
        """以新語言重建視窗。overlay 先 relabel、用 update() 把重繪跑完，才拆舊窗建新窗。

        新視窗映射時會湧出大量繪圖事件，overlay 標籤縮短後騰出區域的重繪（Tk 排在 idle）
        會被排到那之後，舊語言多出的那截字殘留約 0.3 秒（實機截圖與逐幀擷取確認）。
        update_idletasks() 不夠：騰出區域要先收到 Windows 的 WM_PAINT 才會重繪，得跑一輪
        完整事件迴圈。此處在 after_idle 內、不在事件 binding 裡，update() 的重入風險低；
        期間視窗仍可能被使用者關掉，故先確認還在。"""
        self._relabel_overlay()
        self._root.update()
        if self._win is not None and self._win.winfo_exists():
            self._win.destroy()
        self._win = None
        self.open()

    def _form_values(self) -> tuple[dict, str | None]:
        """讀取表單目前的值（不含介面語言，儲存路徑另外處理），回傳（欄位值，進階數值的
        錯誤文案 key 或 None）；進階數值解析失敗時就不含那幾個鍵。"""
        values = {
            **self._services.values(),
            "target_language": self._language.value(),
            "hotkey": self._hotkey.value(),
            "region_hotkey": self._region_hotkey.value(),
            "auto_show_input": self._auto_input.get(),
            "paste_hotkey": self._paste_hotkey.get(),
            "translate_system_messages": self._translate_system.get(),
            "game_path": self._game_path.get().strip() or None,
        }
        advanced, error = parse_advanced_values(
            self._poll, self._fade, self._max_msgs, self._type_delay, self._alpha_var,
            self._parallel)
        if advanced is not None:
            values.update(advanced)
        return values, error

    def _collect_into_draft(self) -> None:
        """把欄位目前的值寫回 draft，供重建視窗時復原。進階數值是非數字、目標語言清空時
        保留 draft 原值：重建不是儲存，不該在這裡擋使用者。"""
        values, _error = self._form_values()
        if not values["target_language"]:
            del values["target_language"]
        self._draft.update(values)

    def _cancel(self) -> None:
        """取消／關窗：把預覽中的透明度與介面語言都還原為目前設定值。"""
        if self._win is None or not self._win.winfo_exists():
            return   # update() 期間再按一次取消／關窗會重入到這裡
        if self._on_alpha_preview is not None:
            self._on_alpha_preview(self._cfg["overlay_alpha"])
        self._draft = None
        if (self._language_at_open is not None
                and current_language() != self._language_at_open):
            log(f"[ui] settings language preview reverted to "
                f"{self._language_at_open}")
            self._preview_language(self._language_at_open)
            # 同 _rebuild：先把 overlay 的重繪跑完再拆窗，拆窗的事件才不會把它往後推
            # （逐幀擷取實測：先拆再 relabel、或拆完排 after_idle 都仍殘留約 0.1 秒）
            self._root.update()
        if self._win is not None and self._win.winfo_exists():
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
        hint_label(parent, t("settings.range_hint", hint=t(hint_key), lo=lo, hi=hi,
                             default=DEFAULT_CONFIG[key]),
                   trailing=HINT_TRAILING).grid(row=grid_row, column=2, sticky="ew",
                                                padx=8, pady=2)
        return var

    def _browse_game_path(self) -> None:
        chosen = filedialog.askdirectory(parent=self._win)
        if chosen:
            self._game_path.set(chosen)

    def _save(self) -> None:
        # 先整批解析再驗證：格式錯誤也要走表單錯誤提示，不能讓 cfg 寫到一半。
        values, advanced_error = self._form_values()
        # 預設服務要存在且填得完整：手改壞掉的設定不該被原樣存回去
        default = find(values, values["default_service"])
        errors = ["error.need_service"] if default is None or validate_service(default) else []
        if values["hotkey"] == values["region_hotkey"]:
            errors.append("error.hotkeys_same")
        if not values["target_language"]:
            errors.append("error.need_target_language")
        if advanced_error:
            errors.append(advanced_error)
        if errors:
            messagebox.showwarning(t("dialog.incomplete_title"),
                                   "\n".join(t(e) for e in errors), parent=self._win)
            return
        cfg = self._cfg
        game_path_changed = values["game_path"] != cfg["game_path"]
        # 語言要在 on_save 之前套用：apply_settings 會依新語言重繪 overlay。
        cfg["ui_language"] = self._ui_language.value()
        set_language(cfg["ui_language"])
        cfg.update(values)
        self._on_save()
        if game_path_changed:
            messagebox.showinfo(t("dialog.notice_title"), t("dialog.game_path_restart"),
                                parent=self._win)
        self._draft = None
        self._win.destroy()
