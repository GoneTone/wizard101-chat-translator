"""精靈與設定視窗共用的欄位群：熱鍵捕捉、翻譯目標語言與介面語言選擇。
小元件在 form.py，單筆服務的表單在 service_form.py。"""
import tkinter as tk
from tkinter import ttk

import keyboard

from src.i18n import (
    DEFAULT_LANGUAGE,
    available_languages,
    language_name,
    t,
)
from src.log import log
from src.ui.form import BackgroundButton, hint_label

# 翻譯目標語言的常用選項：各語言的 endonym，任何介面語言下都不翻譯。
COMMON_LANGUAGES = ["繁體中文（台灣）", "简体中文（中国）", "English", "日本語",
                    "한국어", "Español", "Português", "Deutsch", "Français"]


class HotkeyField(ttk.Frame):
    """熱鍵欄位：顯示目前值，點「更改」後按下組合鍵即設定（Esc 取消）。
    捕捉用 keyboard 套件的低階鍵盤鉤子而非 tk 事件：Ctrl+Space 等組合會先被
    輸入法（IME）或系統攔截、tk 收不到；低階鉤子在 IME 之前就能看到按鍵，
    且與實際註冊熱鍵走同一條路 —— 捕捉得到就保證註冊得到。"""

    def __init__(self, parent, initial: str):
        super().__init__(parent)
        self._value = initial
        self._label = ttk.Label(self, text=initial)
        self._label.pack(side="left")
        self._btn = ttk.Button(self, text=t("button.change"), width=14,
                               command=self._capture)
        self._btn.pack(side="left", padx=8)
        self._task = BackgroundButton(self._btn, "hotkey capture")

    def value(self) -> str:
        return self._value

    def set_value(self, s: str) -> None:
        self._value = s
        self._label.configure(text=s)

    def _capture(self) -> None:
        self._task.start(lambda: keyboard.read_hotkey(suppress=False), self._on_captured,
                         t("button.press_key"))

    def _on_captured(self, combo) -> None:
        if isinstance(combo, Exception):
            log(f"[settings] hotkey capture failed: {combo}")
            return
        if combo and combo != "esc":
            self.set_value(combo)


class LanguageField(ttk.Frame):
    """翻譯目標語言：常用語言下拉 + 可自行輸入。"""

    def __init__(self, parent, initial: str):
        super().__init__(parent)
        self._var = tk.StringVar(value=initial)
        combo = ttk.Combobox(self, textvariable=self._var, values=COMMON_LANGUAGES)
        combo.pack(fill="x")
        hint_label(self, t("hint.language")).pack(fill="x", pady=(2, 0))

    def value(self) -> str:
        return self._var.get().strip()

    def set_value(self, s: str) -> None:
        self._var.set(s)


class UiLanguageField(ttk.Frame):
    """介面語言：唯讀下拉，顯示 endonym、對外進出語言碼 —— 與 LanguageField
    （翻譯目標語言，可自由輸入）是不同用途的兩個欄位。"""

    def __init__(self, parent, initial: str, on_change=None):
        super().__init__(parent)
        self._on_change = on_change
        languages = available_languages()
        self._names = list(languages.values())
        self._codes = list(languages)
        self._var = tk.StringVar(value=language_name(
            initial if initial in languages else DEFAULT_LANGUAGE))
        combo = ttk.Combobox(self, textvariable=self._var, values=self._names,
                             state="readonly")
        combo.pack(fill="x")
        combo.bind("<<ComboboxSelected>>", lambda e: self._notify())

    def value(self) -> str:
        """目前選到的語言碼。"""
        name = self._var.get()
        return self._codes[self._names.index(name)] if name in self._names \
            else DEFAULT_LANGUAGE

    def set_value(self, code: str) -> None:
        self._var.set(language_name(
            code if code in available_languages() else DEFAULT_LANGUAGE))

    def _notify(self) -> None:
        if self._on_change is not None:
            self._on_change(self.value())
