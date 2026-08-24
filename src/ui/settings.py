"""一般設定視窗：分「基本／進階」分頁，儲存即套用（不需重啟）。
遊戲路徑例外：重掛 hook 需重啟，儲存後提示下次啟動生效。"""
import tkinter as tk
from tkinter import filedialog, messagebox, ttk

from src import __version__
from src.config import ADVANCED_LIMITS, APP_NAME, DEFAULT_CONFIG, clamp_advanced
from src.ui.fields import (AUTO_INPUT_LABEL, ApiFields, HotkeyField, LanguageField,
                           validate_api_form)


def parse_advanced_values(poll_var, fade_var, max_messages_var, type_delay_var,
                          alpha_var, parallel_var) -> tuple[dict | None, str | None]:
    """讀取並轉型進階數值 Tk 變數：使用者手動鍵入非數字時，Tk 變數的
    `.get()` 會拋 `TclError`，`int()`／`float()` 轉型也可能拋 `ValueError`——
    統一在此攔截並回傳 `（None， 錯誤訊息）`，讓呼叫端走既有表單錯誤提示、
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
        return None, "進階數值格式錯誤，請輸入數字"
    return values, None


class SettingsWindow:
    """設定視窗（單例）：open() 顯示或帶到前景；儲存時就地更新 cfg 並呼叫 on_save。"""

    def __init__(self, root: tk.Tk, cfg: dict, on_save, on_alpha_preview=None):
        self._root = root
        self._cfg = cfg
        self._on_save = on_save
        self._on_alpha_preview = on_alpha_preview  # 拖滑桿即時套用透明度（預覽）
        self._win: tk.Toplevel | None = None

    def open(self) -> None:
        if self._win is not None and self._win.winfo_exists():
            self._win.lift()
            self._win.focus_force()
            return
        cfg = self._cfg
        self._win = tk.Toplevel(self._root)
        self._win.title(f"{APP_NAME} — 設定")
        win_w, win_h = 640, 560
        x = (self._win.winfo_screenwidth() - win_w) // 2
        y = (self._win.winfo_screenheight() - win_h) // 2
        self._win.geometry(f"{win_w}x{win_h}+{x}+{y}")
        self._win.resizable(False, False)  # 版面按固定尺寸配置，縮放會切到文字
        self._win.attributes("-topmost", True)

        nb = ttk.Notebook(self._win)
        nb.pack(fill="both", expand=True, padx=8, pady=8)

        # --- 基本 ---
        basic = ttk.Frame(nb, padding=12)
        nb.add(basic, text="基本")
        self._api = ApiFields(basic, cfg["api"])
        self._api.pack(fill="x")
        self._api.set_target_language_fn(lambda: self._language.value())
        ttk.Label(basic, text="翻譯目標語言").pack(anchor="w", pady=(12, 0))
        self._language = LanguageField(basic, cfg["target_language"])
        self._language.pack(fill="x", pady=(2, 8))
        ttk.Label(basic, text="呼出輸入框的熱鍵").pack(anchor="w")
        self._hotkey = HotkeyField(basic, cfg["hotkey"])
        self._hotkey.pack(anchor="w", pady=(2, 0))
        self._auto_input = tk.BooleanVar(value=cfg["auto_show_input"])
        ttk.Checkbutton(basic, text=AUTO_INPUT_LABEL,
                        variable=self._auto_input).pack(anchor="w", pady=(10, 0))

        # --- 進階 ---
        adv = ttk.Frame(nb, padding=12)
        nb.add(adv, text="進階")
        self._poll = self._spin(adv, "輪詢間隔（秒）", cfg["poll_interval"],
                                "poll_interval", 0.1, "收訊掃描頻率，小＝更即時")
        self._fade = self._spin(adv, "訊息淡出（秒）", cfg["fade_seconds"],
                                "fade_seconds", 10, "0＝永不淡出，可滾動看歷史")
        self._max_msgs = self._spin(adv, "訊息保留上限", cfg["max_messages"],
                                    "max_messages", 10, "超過移除最舊")
        self._parallel = self._spin(adv, "同時翻譯則數", cfg["max_parallel_translations"],
                                    "max_parallel_translations", 1,
                                    "1＝逐則排隊；大於 1 時卡住的訊息不會擋住後續")
        self._type_delay = self._spin(adv, "鍵入延遲（秒）", cfg["type_delay"],
                                      "type_delay", 0.01, "遊戲漏字就調大")
        self._alpha_var = self._alpha_slider(adv, cfg["overlay_alpha"])
        path_row = ttk.Frame(adv)
        path_row.pack(fill="x", pady=(8, 0))
        ttk.Label(path_row, text="遊戲路徑", width=14).pack(side="left")
        self._game_path = tk.StringVar(value=cfg["game_path"] or "")
        ttk.Entry(path_row, textvariable=self._game_path).pack(
            side="left", fill="x", expand=True)
        ttk.Button(path_row, text="瀏覽…", width=7,
                   command=self._browse_game_path).pack(side="left", padx=(4, 0))
        ttk.Label(adv, text="留空＝自動偵測執行中的遊戲",
                  foreground="#888888").pack(anchor="w")

        btns = ttk.Frame(self._win, padding=(8, 0, 8, 8))
        btns.pack(side="bottom", fill="x")
        ttk.Label(btns, text=f"v{__version__}", foreground="#888888").pack(side="left")
        ttk.Button(btns, text="取消", command=self._cancel).pack(side="right")
        ttk.Button(btns, text="儲存", command=self._save).pack(side="right", padx=(0, 8))
        self._win.protocol("WM_DELETE_WINDOW", self._cancel)

    def _alpha_slider(self, parent, initial: float) -> tk.DoubleVar:
        """視窗不透明度滑桿：拖動即時預覽（套到 overlay 與泡泡），儲存才寫入設定。"""
        lo, hi = ADVANCED_LIMITS["overlay_alpha"]
        row = ttk.Frame(parent)
        row.pack(fill="x", pady=2)
        ttk.Label(row, text="視窗不透明度", width=14).pack(side="left")
        var = tk.DoubleVar(value=initial)
        value_label = ttk.Label(row, text=f"{initial:.2f}", width=5)

        def on_slide(raw: str) -> None:
            v = round(float(raw), 2)
            var.set(v)
            value_label.configure(text=f"{v:.2f}")
            if self._on_alpha_preview is not None:
                self._on_alpha_preview(v)

        ttk.Scale(row, from_=lo, to=hi, orient="horizontal", variable=var,
                  command=on_slide, length=160).pack(side="left")
        value_label.pack(side="left", padx=(6, 0))
        ttk.Label(row, text=f"即時預覽，小＝更透明（預設 {DEFAULT_CONFIG['overlay_alpha']}）",
                  foreground="#888888", wraplength=280,
                  justify="left").pack(side="left", padx=8)
        return var

    def _cancel(self) -> None:
        """取消／關窗：把預覽中的透明度還原為目前設定值。"""
        if self._on_alpha_preview is not None:
            self._on_alpha_preview(self._cfg["overlay_alpha"])
        self._win.destroy()

    def _spin(self, parent, label, initial, key, step, hint):
        lo, hi = ADVANCED_LIMITS[key]
        row = ttk.Frame(parent)
        row.pack(fill="x", pady=2)
        ttk.Label(row, text=label, width=14).pack(side="left")
        var = tk.DoubleVar(value=initial) if isinstance(initial, float) \
            else tk.IntVar(value=initial)
        ttk.Spinbox(row, textvariable=var, from_=lo, to=hi, increment=step,
                    width=8).pack(side="left")
        ttk.Label(row, text=f"{hint}（範圍 {lo}–{hi}，預設 {DEFAULT_CONFIG[key]}）",
                  foreground="#888888", wraplength=380,
                  justify="left").pack(side="left", padx=8)
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
        errors = validate_api_form(api)
        if not self._language.value():
            errors.append("請選擇或輸入翻譯目標語言")
        if advanced_error:
            errors.append(advanced_error)
        if errors:
            messagebox.showwarning("設定不完整", "\n".join(errors), parent=self._win)
            return
        cfg = self._cfg
        old_game_path = cfg["game_path"]
        cfg["api"] = api
        cfg["target_language"] = self._language.value()
        cfg["hotkey"] = self._hotkey.value()
        cfg["auto_show_input"] = self._auto_input.get()
        cfg.update(advanced)
        cfg["game_path"] = self._game_path.get().strip() or None
        game_path_changed = cfg["game_path"] != old_game_path
        self._on_save()
        if game_path_changed:
            messagebox.showinfo("提示", "遊戲路徑將於下次啟動生效", parent=self._win)
        self._win.destroy()
