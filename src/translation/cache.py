"""系統訊息譯文的持久化快取。

系統訊息不吃聊天上下文（見 translator.translate_system_message），因此「同一句原文
必然得到同一句譯文」——這正是它可以安全快取、而玩家對話不行的原因。

數字先正規化成佔位符再當 key：`你获得了 39 金币！` 與 `你获得了 65 金币！` 是同一個
句型，金額每次都不同，不正規化就永遠不會命中。
"""
import json
import os
import re
import tempfile
import threading
from collections import OrderedDict
from pathlib import Path

from src.config import local_state_dir
from src.log import log
from src.translation.translator import PROMPT_REVISION, has_stray_latin

_NUMBER = re.compile(r"\d+(?:[.,]\d+)*")
_PLACEHOLDER = re.compile(r"\{(\d+)\}")


def normalize(text: str) -> tuple[str, list[str]]:
    """把文字中的數字換成依序編號的佔位符，回傳（樣板, 依序取出的數字）。"""
    numbers: list[str] = []

    def take(m: re.Match) -> str:
        numbers.append(m.group(0))
        return f"{{{len(numbers) - 1}}}"

    return _NUMBER.sub(take, text), numbers


def restore(template: str, numbers: list[str]) -> str:
    """把數字填回樣板。依佔位符的**編號**取值，不依出現位置——
    譯文的語序可能與原文不同（`{1} gold and {0} XP`），照位置填會把數字對調。"""
    def put(m: re.Match) -> str:
        idx = int(m.group(1))
        return numbers[idx] if idx < len(numbers) else m.group(0)

    return _PLACEHOLDER.sub(put, template)


def placeholders_match(template: str, translated: str) -> bool:
    """譯文的佔位符是否與樣板完全一致（含重複次數）。

    模型可能吃掉、改寫或多生出佔位符；不一致就不能回填，該筆一律不存快取、
    改用原文直翻一次（正確性優先於命中率）。"""
    return sorted(_PLACEHOLDER.findall(template)) == sorted(_PLACEHOLDER.findall(translated))


CACHE_PATH = local_state_dir() / "system-message-cache.json"
MAX_ENTRIES = 2000    # 系統訊息的句型與材料名是有限集合，2000 筆足以涵蓋
FLUSH_EVERY = 20      # 累積這麼多筆新增才落盤一次（不逐筆寫）


def fingerprint_of(provider: str, model: str, target_language: str) -> str:
    """快取指紋：換服務商、模型、目標語言或提示詞版次時舊譯文整份作廢（不含版次的話，
    舊提示詞翻壞的譯名會跨程式更新留在磁碟上）。絕不含 API 金鑰——指紋會寫進磁碟。"""
    return f"{provider}|{model}|{target_language}|p{PROMPT_REVISION}"


class TranslationCache:
    """系統訊息譯文快取。key 是正規化後的樣板，value 是樣板的譯文。

    執行緒安全：reader 執行緒查詢、翻譯 worker 寫入，兩邊都持同一把鎖。
    """

    def __init__(self, fingerprint: str):
        self._fingerprint = fingerprint
        self._entries: OrderedDict[str, str] = OrderedDict()
        self._unflushed = 0
        self._lock = threading.Lock()

    @property
    def fingerprint(self) -> str:
        """目前生效的指紋。翻譯開始前先取一份，寫回時交給 put() 比對（見 put）。"""
        with self._lock:
            return self._fingerprint

    def get(self, text: str) -> str | None:
        """查快取。命中回傳**已回填數字**的譯文，未命中回傳 None。"""
        template, numbers = normalize(text)
        with self._lock:
            translated = self._entries.get(template)
            if translated is None:
                return None
            self._entries.move_to_end(template)
        return restore(translated, numbers)

    def put(self, text: str, translated_template: str, fingerprint: str) -> bool:
        """存入一筆；`translated_template` 是樣板的譯文（仍帶佔位符）。
        佔位符對不上就不存並回傳 False——呼叫端須改用原文直翻。
        `fingerprint` 是譯文產出當下的指紋：翻譯飛行中使用者可能 rebind 換掉服務商／模型／
        目標語言，舊設定翻好的譯文若照存會被當成新設定的寫進磁碟、跨重啟回吐錯誤語言。"""
        template, _ = normalize(text)
        if not placeholders_match(template, translated_template):
            log(f"[cache] placeholder mismatch, not cached: "
                f"template={template!r} translated={translated_template!r}")
            return False
        with self._lock:
            if fingerprint != self._fingerprint:
                log(f"[cache] fingerprint changed while translating, discarding "
                    f"stale translation: template={template!r} "
                    f"produced_under={fingerprint!r} current={self._fingerprint!r}")
                return False
            self._entries[template] = translated_template
            self._entries.move_to_end(template)
            while len(self._entries) > MAX_ENTRIES:
                dropped, _ = self._entries.popitem(last=False)
                log(f"[cache] evicted least recently used entry: {dropped!r}")
            self._unflushed += 1
            due = self._unflushed >= FLUSH_EVERY
        if due:
            self.flush()
        return True

    def rebind(self, fingerprint: str) -> None:
        """指紋變更（換服務商／模型／目標語言）：先落盤舊的，再清空重來。"""
        if fingerprint == self._fingerprint:
            return
        self.flush()
        with self._lock:
            log(f"[cache] fingerprint changed at runtime, clearing "
                f"{len(self._entries)} entries")
            self._entries.clear()
            self._fingerprint = fingerprint
            self._unflushed = 0

    def clear(self) -> int:
        """清空快取並刪掉磁碟檔案，回傳清掉的筆數。與 rebind() 不同：那會先 flush 舊內容，
        這是使用者主動丟棄、不寫回。刪檔失敗只記 log 不拋——快取是最佳化路徑，
        下次 flush 會覆寫它。"""
        with self._lock:
            count = len(self._entries)
            self._entries.clear()
            self._unflushed = 0
        try:
            CACHE_PATH.unlink(missing_ok=True)
        except Exception as exc:
            log(f"[cache] could not delete {CACHE_PATH}: {exc}")
        log(f"[cache] cleared {count} entries on user request")
        return count

    def load(self) -> None:
        """從磁碟載入。指紋不符、檔案損壞或不存在一律當作空快取（不是錯誤）。"""
        if not CACHE_PATH.exists():
            log("[cache] no cache file yet, starting empty")
            return
        try:
            data = json.loads(CACHE_PATH.read_text(encoding="utf-8"))
            stored = data.get("fingerprint")
            entries = data.get("entries") or {}
            if not isinstance(entries, dict):
                raise TypeError(f"entries is {type(entries).__name__}, expected dict")
            items = list(entries.items())[-MAX_ENTRIES:]
        except Exception as exc:
            log(f"[cache] unreadable cache file, starting empty: {exc}")
            return
        if stored != self._fingerprint:
            log(f"[cache] fingerprint changed, discarding {len(entries)} entries "
                f"(stored={stored!r}, current={self._fingerprint!r})")
            return
        with self._lock:
            self._entries = OrderedDict(items)
        log(f"[cache] loaded {len(items)} entries from {CACHE_PATH}")

    def flush(self) -> None:
        """寫回磁碟：先寫同目錄的獨立暫存檔，再 os.replace() 原子換上。
        worker 的 auto-flush 與關閉時的最終 flush 可能同時觸發，各自寫自己的暫存檔，
        讀者不會看到寫一半的 JSON，疊在一起頂多後者覆蓋前者。寫檔失敗只記 log。"""
        with self._lock:
            payload = {"fingerprint": self._fingerprint,
                       "entries": dict(self._entries)}
            count = len(self._entries)
            self._unflushed = 0
        tmp_path: Path | None = None
        try:
            CACHE_PATH.parent.mkdir(parents=True, exist_ok=True)
            fd, tmp_name = tempfile.mkstemp(dir=CACHE_PATH.parent,
                                            prefix=f"{CACHE_PATH.name}.")
            tmp_path = Path(tmp_name)
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                json.dump(payload, f, ensure_ascii=False)
            os.replace(tmp_path, CACHE_PATH)
            tmp_path = None   # 已被換到目的地，不必再清
            log(f"[cache] flushed {count} entries to {CACHE_PATH}")
        except Exception as exc:
            log(f"[cache] flush failed: {exc}")
        finally:
            if tmp_path is not None:
                try:
                    tmp_path.unlink(missing_ok=True)
                except OSError:
                    pass


def translate_and_cache(translator, cache: TranslationCache, text: str) -> str:
    """翻一則系統訊息並存進快取。送翻與存入的都是正規化後的樣板；佔位符被模型弄壞或
    翻譯期間指紋被換掉（見 put）時不快取，改用原文直翻一次——那一次走的已是新設定。
    落回英文的譯文（見 translator.has_stray_latin）一律不落盤：翻譯器已重譯過一次，
    救不回的照樣顯示，但存進快取等於把錯誤固化、每次命中都吐同一個英文名。

    `translator` 只要求有 `translate_system_message(text) -> str` 與 `target_language`，
    不直接依賴 Translator 型別以避免模組互相 import。"""
    template, numbers = normalize(text)
    fingerprint = cache.fingerprint   # 先記下產出當下的指紋（翻譯期間可能換設定）
    translated = translator.translate_system_message(template)
    if has_stray_latin(template, translated, translator.target_language):
        log(f"[cache] translation is not in the target language, not cached: "
            f"template={template!r} translated={translated!r}")
        return restore(translated, numbers)
    # put() 收原文、內部自己正規化；傳 template 會把 `{0}` 裡的 0 再當成數字、變成 `{{0}}`。
    if cache.put(text, translated, fingerprint):
        return restore(translated, numbers)
    log(f"[cache] falling back to a direct translation: {text!r}")
    return translator.translate_system_message(text)
