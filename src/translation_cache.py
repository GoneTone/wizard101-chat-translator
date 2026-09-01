"""系統訊息譯文的持久化快取。

系統訊息不吃聊天上下文（見 translator.translate_system_message），因此「同一句原文
必然得到同一句譯文」——這正是它可以安全快取、而玩家對話不行的原因。

數字先正規化成佔位符再當 key：`你获得了 39 金币！` 與 `你获得了 65 金币！` 是同一個
句型，金額每次都不同，不正規化就永遠不會命中。
"""
import json
import os
import re
import sys
import tempfile
import threading
from collections import OrderedDict
from pathlib import Path

from src.config import local_state_dir

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
    """快取指紋：換服務商、換模型或換目標語言時，舊譯文必須整份作廢。
    **絕不含 API 金鑰**——這份指紋會被寫進磁碟。"""
    return f"{provider}|{model}|{target_language}"


class TranslationCache:
    """系統訊息譯文快取。key 是正規化後的樣板，value 是樣板的譯文。

    執行緒安全：reader 執行緒查詢、翻譯 worker 寫入，兩邊都持同一把鎖。
    """

    def __init__(self, fingerprint: str):
        self._fingerprint = fingerprint
        self._entries: "OrderedDict[str, str]" = OrderedDict()
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
        """存入一筆。`translated_template` 是**樣板的譯文**（仍帶佔位符）。
        佔位符與樣板對不上就不存並回傳 False——呼叫端須改用原文直翻。

        `fingerprint` 是這則譯文**產出當下**的指紋：翻譯還在飛行中時使用者可能在設定
        視窗換掉服務商／模型／目標語言（rebind），此時用舊設定翻好的譯文若照存，會被
        當成新設定的譯文寫進磁碟、跨重啟一直回吐錯誤語言。不符即丟棄並回傳 False。
        """
        template, _ = normalize(text)
        if not placeholders_match(template, translated_template):
            print(f"[cache] placeholder mismatch, not cached: "
                  f"template={template!r} translated={translated_template!r}",
                  file=sys.stderr)
            return False
        with self._lock:
            if fingerprint != self._fingerprint:
                print(f"[cache] fingerprint changed while translating, discarding "
                      f"stale translation: template={template!r} "
                      f"produced_under={fingerprint!r} current={self._fingerprint!r}",
                      file=sys.stderr)
                return False
            self._entries[template] = translated_template
            self._entries.move_to_end(template)
            while len(self._entries) > MAX_ENTRIES:
                dropped, _ = self._entries.popitem(last=False)
                print(f"[cache] evicted least recently used entry: {dropped!r}",
                      file=sys.stderr)
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
            print(f"[cache] fingerprint changed at runtime, clearing "
                  f"{len(self._entries)} entries", file=sys.stderr)
            self._entries.clear()
            self._fingerprint = fingerprint
            self._unflushed = 0

    def clear(self) -> int:
        """清空快取並刪掉磁碟檔案，回傳清掉的筆數（設定視窗要回報給使用者）。

        與 rebind() 語意不同：那是換指紋，會先把舊內容 flush 出去留著；這是使用者
        主動丟棄，不寫回任何東西。刪檔失敗只記 log 不拋——快取是最佳化路徑，
        清不掉檔案也不該讓設定視窗炸掉，下次 flush 會覆寫它。"""
        with self._lock:
            count = len(self._entries)
            self._entries.clear()
            self._unflushed = 0
        try:
            CACHE_PATH.unlink(missing_ok=True)
        except Exception as exc:
            print(f"[cache] could not delete {CACHE_PATH}: {exc}", file=sys.stderr)
        print(f"[cache] cleared {count} entries on user request", file=sys.stderr)
        return count

    def load(self) -> None:
        """從磁碟載入。指紋不符、檔案損壞或不存在一律當作空快取（不是錯誤）。"""
        if not CACHE_PATH.exists():
            print("[cache] no cache file yet, starting empty", file=sys.stderr)
            return
        try:
            data = json.loads(CACHE_PATH.read_text(encoding="utf-8"))
            stored = data.get("fingerprint")
            entries = data.get("entries") or {}
            if not isinstance(entries, dict):
                raise TypeError(f"entries is {type(entries).__name__}, expected dict")
            items = list(entries.items())[-MAX_ENTRIES:]
        except Exception as exc:
            print(f"[cache] unreadable cache file, starting empty: {exc}",
                  file=sys.stderr)
            return
        if stored != self._fingerprint:
            print(f"[cache] fingerprint changed, discarding {len(entries)} entries "
                  f"(stored={stored!r}, current={self._fingerprint!r})", file=sys.stderr)
            return
        with self._lock:
            self._entries = OrderedDict(items)
        print(f"[cache] loaded {len(items)} entries from {CACHE_PATH}",
              file=sys.stderr)

    def flush(self) -> None:
        """寫回磁碟：先寫進同目錄的獨立暫存檔，寫完再 os.replace() 原子性換上。

        worker 觸發的 auto-flush（見 put()）與關閉流程的最終 flush 可能同時觸發——
        兩邊各自寫進自己的暫存檔名，不會共用同一個寫入中的檔案；os.replace() 在
        Windows 與 POSIX 上都是原子操作，讀者（load()）不會看到寫一半的 JSON，
        兩次 flush 疊在一起也頂多是後者覆蓋前者、不會互相截斷成殘破檔案。
        寫檔失敗只記 log，不影響翻譯——快取是最佳化，不是必要路徑。"""
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
            print(f"[cache] flushed {count} entries to {CACHE_PATH}", file=sys.stderr)
        except Exception as exc:
            print(f"[cache] flush failed: {exc}", file=sys.stderr)
        finally:
            if tmp_path is not None:
                try:
                    tmp_path.unlink(missing_ok=True)
                except OSError:
                    pass


def translate_and_cache(translator, cache: TranslationCache, text: str) -> str:
    """翻一則系統訊息並存進快取。送去翻譯的是正規化後的樣板，存的也是樣板譯文；
    佔位符被模型弄壞、或翻譯期間指紋被換掉（見 put）時不快取，改用原文直翻一次
    ——後者這一次直翻走的已是新設定，拿到的譯文語言才對（正確性優先於命中率）。

    `translator` 只要求有 `translate_system_message(text) -> str`（見
    `src.translator.Translator`），這裡不直接依賴該型別以避免模組互相 import。
    """
    template, numbers = normalize(text)
    fingerprint = cache.fingerprint   # 翻譯期間使用者可能換設定，先記下產出當下的指紋
    translated = translator.translate_system_message(template)
    # put() 收的是**原文**、內部自己正規化。這裡不能傳 template——
    # 它含 `{0}`，再 normalize 一次會把裡面的 0 當成數字，變成 `{{0}}`。
    if cache.put(text, translated, fingerprint):
        return restore(translated, numbers)
    print(f"[cache] falling back to a direct translation: {text!r}", file=sys.stderr)
    return translator.translate_system_message(text)
