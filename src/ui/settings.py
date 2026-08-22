"""一般設定視窗：分「基本／進階」分頁，儲存即套用（不需重啟）。
遊戲路徑例外：重掛 hook 需重啟，儲存後提示下次啟動生效。"""
import tkinter as tk
from tkinter import filedialog, messagebox, ttk

from src.ui.fields import ApiFields, HotkeyField, LanguageField, validate_api_form

_ADVANCED_LIMITS = {
    "poll_interval": (0.1, 5.0),
    "fade_seconds": (0, 3600),
    "max_messages": (10, 1000),
    "type_delay": (0.0, 0.5),
}


def clamp_advanced(values: dict) -> dict:
    """把進階數值夾在合理範圍，避免填出爆炸值。"""
    for key, (lo, hi) in _ADVANCED_LIMITS.items():
        values[key] = min(hi, max(lo, values[key]))
    return values


class SettingsWindow:
    """設定視窗（單例）：open() 顯示或帶到前景；儲存時就地更新 cfg 並呼叫 on_save。"""

    def __init__(self, root: tk.Tk, cfg: dict, on_save):
        self._root = root
        self._cfg = cfg
        self._on_save = on_save
        self._win: tk.Toplevel | None = None

    def open(self) -> None:
        if self._win is not None and self._win.winfo_exists():
            self._win.lift()
            self._win.focus_force()
            return
        cfg = self._cfg
        self._win = tk.Toplevel(self._root)
        self._win.title("Wizard101 聊天翻譯助手 — 設定")
        self._win.geometry("560x520")
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

        # --- 進階 ---
        adv = ttk.Frame(nb, padding=12)
        nb.add(adv, text="進階")
        self._poll = self._spin(adv, "輪詢間隔（秒）", cfg["poll_interval"],
                                0.1, 5.0, 0.1, "收訊掃描頻率，小＝更即時")
        self._fade = self._spin(adv, "訊息淡出（秒）", cfg["fade_seconds"],
                                0, 3600, 10, "0＝永不淡出，可滾動看歷史")
        self._max_msgs = self._spin(adv, "訊息保留上限", cfg["max_messages"],
                                    10, 1000, 10, "超過移除最舊")
        self._type_delay = self._spin(adv, "鍵入延遲（秒）", cfg["type_delay"],
                                      0.0, 0.5, 0.01, "遊戲漏字就調大")
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
        ttk.Button(btns, text="取消", command=self._win.destroy).pack(side="right")
        ttk.Button(btns, text="儲存", command=self._save).pack(side="right", padx=(0, 8))

    def _spin(self, parent, label, initial, lo, hi, step, hint):
        row = ttk.Frame(parent)
        row.pack(fill="x", pady=2)
        ttk.Label(row, text=label, width=14).pack(side="left")
        var = tk.DoubleVar(value=initial) if isinstance(initial, float) \
            else tk.IntVar(value=initial)
        ttk.Spinbox(row, textvariable=var, from_=lo, to=hi, increment=step,
                    width=8).pack(side="left")
        ttk.Label(row, text=hint, foreground="#888888").pack(side="left", padx=8)
        return var

    def _browse_game_path(self) -> None:
        chosen = filedialog.askdirectory(parent=self._win)
        if chosen:
            self._game_path.set(chosen)

    def _save(self) -> None:
        api = self._api.get_values()
        errors = validate_api_form(api)
        if not self._language.value():
            errors.append("請選擇或輸入翻譯目標語言")
        if errors:
            messagebox.showwarning("設定不完整", "\n".join(errors), parent=self._win)
            return
        cfg = self._cfg
        old_game_path = cfg["game_path"]
        cfg["api"] = api
        cfg["target_language"] = self._language.value()
        cfg["hotkey"] = self._hotkey.value()
        advanced = clamp_advanced({
            "poll_interval": float(self._poll.get()),
            "fade_seconds": int(self._fade.get()),
            "max_messages": int(self._max_msgs.get()),
            "type_delay": float(self._type_delay.get()),
        })
        cfg.update(advanced)
        cfg["game_path"] = self._game_path.get().strip() or None
        game_path_changed = cfg["game_path"] != old_game_path
        self._on_save(game_path_changed)
        if game_path_changed:
            messagebox.showinfo("提示", "遊戲路徑將於下次啟動生效", parent=self._win)
        self._win.destroy()
