"""few-shot 範例集：請設定的模型把固定示範改寫成目標語言，並驗證三條路徑（收訊、系統訊息、區域）。"""
import hashlib
import json
import os
import re
import tempfile
import threading
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from src.config import local_state_dir
from src.log import log
from src.translation.postprocess import (
    number_lines,
    numbered_entries,
    tidy_parentheses,
    unnumber_lines,
)

# 範例內容（含 GAME_LANGUAGE_EXAMPLES）或生成提示詞有變時要遞增，讓舊快取作廢。
EXAMPLE_REVISION = 4
EXAMPLES_PATH = local_state_dir() / "translation-examples.json"
MAX_ENTRIES = 32

_INTERFACE_LABEL = "OPTIONS"

SOURCE_LINES = (
    "[Amy] idk, Kai and I got new armor and learned Fire Cat at Colossus Boulevard lol, "
    "brb my wand is trash",
    "Kai taught you Fire Cat! Gained {0} gold at Colossus Boulevard.",
    "Talk to the Fire Cat",
    "Go to Colossus Boulevard",
    _INTERFACE_LABEL,
    "and then you must",
)

DEMO_LINES = (
    "[Amy] 不知道耶，我和 Kai 拿到新護甲，還在巨像大道（Colossus Boulevard）學會了火貓（Fire Cat），"
    "笑死，等我一下，我的法杖超爛",
    "Kai 教會了你火貓（Fire Cat）！在巨像大道（Colossus Boulevard）獲得了 {0} 金幣。",
    "和火貓（Fire Cat）談談",
    "前往巨像大道（Colossus Boulevard）",
    "選項",
    "然後你必須",
)

_Pair = tuple[str, str]

_GENERATION_SYSTEM = (
    "把使用者給的每一行翻成 {t}，逐行對應、保留行首編號，只輸出譯文。"
    "格式照下面的中文示範：遊戲專有名詞翻成 {t} 後緊接括號照抄英文原文；"
    "[Amy]、人名 Kai、{{0}} 照抄不翻；縮寫、一般名詞與介面按鈕文字直接翻、不加括號；"
    "最後一行是被截斷的句子，譯文也停在同一處。\n"
    "中文示範：\n{demo}"
)


@dataclass(frozen=True)
class ExampleSet:
    """三條路徑的 few-shot 範例（原文, 譯文）；通不過驗證的路徑為 None。"""

    incoming: _Pair | None
    system: _Pair | None
    region: _Pair | None

    def merge(self, other: "ExampleSet") -> "ExampleSet":
        """自己已有的路徑保留，缺的用 other 補。"""
        return ExampleSet(self.incoming or other.incoming,
                          self.system or other.system,
                          self.region or other.region)

    @property
    def complete(self) -> bool:
        return None not in (self.incoming, self.system, self.region)

    @property
    def empty(self) -> bool:
        return self.incoming is None and self.system is None and self.region is None

    def digest(self) -> str:
        """內容的 sha1 前 12 碼，是系統訊息譯文快取指紋的一部分（見 main.incoming_fingerprint）。

        改動這裡或 ExampleSet 的 repr 會讓使用者既有的譯文快取全部作廢。"""
        return hashlib.sha1(repr(self).encode("utf-8")).hexdigest()[:12]

    def summary(self) -> str:
        return " ".join(f"{name}={'ok' if value else 'failed'}" for name, value in
                        (("incoming", self.incoming), ("system", self.system),
                         ("region", self.region)))


EMPTY = ExampleSet(None, None, None)

GAME_LANGUAGE_EXAMPLES = ExampleSet(
    incoming=("[小明] 不知道耶，我和 Kai 拿到新護甲，還在巨像大道學會了火貓，笑死，"
              "等我一下，我的法杖超爛",
              "[小明] idk, Kai and I got new armor and learned Fire Cat (火貓) "
              "at Colossus Boulevard (巨像大道) lol, brb my wand is trash"),
    system=("Kai 教会了你火猫！在巨像大道获得了 {0} 金币。",
            "Kai taught you Fire Cat (火猫)! Gained {0} gold at Colossus Boulevard (巨像大道)."),
    region=("和火猫谈谈\n前往巨像大道\n选项\n然后你必须",
            "Talk to the Fire Cat (火猫)\nGo to Colossus Boulevard (巨像大道)"
            "\nOptions\nand then you must"),
)


def examples_fingerprint(api: dict, target_language: str) -> str:
    """範例快取指紋：換服務商、端點、模型、目標語言或範例版次就作廢；絕不含 API 金鑰。"""
    return (f"{api.get('provider', '')}|{api.get('base_url', '')}|{api.get('model', '')}"
            f"|{target_language.strip()}|e{EXAMPLE_REVISION}")


def generation_request(target_language: str) -> tuple[str, list[dict]]:
    """組出請模型把示範改寫成 target_language 的（system 提示詞, 對話輪）。"""
    _, demo = number_lines("\n".join(DEMO_LINES))
    _, source = number_lines("\n".join(SOURCE_LINES))
    system = _GENERATION_SYSTEM.format(t=target_language, demo=demo)
    return system, [{"role": "user", "content": source}]


def _in_parentheses(line: str, name: str) -> bool:
    """名稱譯過且括號附原文。「Fire Cat (Fire Cat)」這種只是照抄英文再重複一次的不算：
    實測它教模型專有名詞不翻，括號又會被 tidy_parentheses 當成重複刪掉。"""
    kept = tidy_parentheses(name, line)
    return re.search(rf"[（(]\s*{re.escape(name)}\s*[)）]", kept) is not None


def _check_incoming(line: str) -> list[str]:
    ok = "[Amy]" in line and "Kai" in line and _in_parentheses(line, "Fire Cat") \
        and _in_parentheses(line, "Colossus Boulevard")
    return [] if ok else [
        "line 1: needs [Amy], Kai and both names translated with the original in parentheses"]


def _check_system(line: str) -> list[str]:
    ok = "Kai" in line and line.count("{0}") == 1 and _in_parentheses(line, "Fire Cat") \
        and _in_parentheses(line, "Colossus Boulevard")
    return [] if ok else [
        "line 2: needs Kai, one {0} and both names translated with the original in parentheses"]


def _check_region(lines: list[str]) -> list[str]:
    # 介面標籤那行示範「不加括號」：少了它，小模型會替整排短標籤（選單、設定項目）都附原文
    checks = ((3, _in_parentheses(lines[0], "Fire Cat"),
               "needs Fire Cat translated with the original in parentheses"),
              (4, _in_parentheses(lines[1], "Colossus Boulevard"),
               "needs Colossus Boulevard translated with the original in parentheses"),
              (5, re.search(r"[（()）]", lines[2]) is None,
               "interface label must not have parentheses"),
              (6, bool(lines[3]), "must not be empty"))
    return [f"line {n}: {failure}" for n, ok, failure in checks if not ok]


def parse_generated(output: str) -> tuple[ExampleSet, list[str]]:
    """拆開模型生成的各行譯文並逐路徑驗證，回傳（通過的範例, 失敗說明）。

    缺編號的行會被 unnumber_lines 補回原文，所以與原文相同的行視同缺行。
    """
    lines = unnumber_lines(output, list(SOURCE_LINES)).splitlines()
    if len(lines) != len(SOURCE_LINES):
        return EMPTY, [f"line count {len(lines)} != {len(SOURCE_LINES)}"]
    lines = [line.strip() for line in lines]
    # 介面標籤在部分語言拼法與英文相同（法文 OPTIONS），模型真的寫了這行時照抄也算譯文；
    # 漏掉而被補回原文的不算，否則範例會示範「標籤不翻」
    label_n = SOURCE_LINES.index(_INTERFACE_LABEL) + 1
    label_answered = bool(numbered_entries(output).get(label_n))
    untranslated = [f"line {n} untranslated" for n, (line, source)
                    in enumerate(zip(lines, SOURCE_LINES, strict=True), 1)
                    if line == source and not (n == label_n and label_answered)]
    if untranslated:
        return EMPTY, untranslated
    incoming = _check_incoming(lines[0])
    system = _check_system(lines[1])
    region = _check_region(lines[2:])
    return ExampleSet(
        incoming=None if incoming else (SOURCE_LINES[0], lines[0]),
        system=None if system else (SOURCE_LINES[1], lines[1]),
        region=None if region else ("\n".join(SOURCE_LINES[2:]), "\n".join(lines[2:])),
    ), incoming + system + region


_FIELDS = ("incoming", "system", "region")
_store_lock = threading.Lock()


def _pair_from_json(value) -> _Pair | None:
    if value is None:
        return None
    if not (isinstance(value, list) and len(value) == 2
            and all(isinstance(item, str) for item in value)):
        raise TypeError(f"pair is not a list of two strings: {value!r:.80}")
    return value[0], value[1]


class ExampleStore:
    """範例集的磁碟快取，以指紋為 key；不在記憶體保存內容，每次都直接讀檔。"""

    def __init__(self, path: Path = EXAMPLES_PATH):
        self._path = path

    def get(self, fingerprint: str) -> ExampleSet | None:
        """查指紋對應的範例集，沒有（或檔案壞掉）回傳 None。"""
        entry = self._read().get(fingerprint)
        return None if entry is None else ExampleSet(**entry)

    def put(self, fingerprint: str, examples: ExampleSet) -> None:
        """存入並把該指紋移到最後，超過 MAX_ENTRIES 捨棄最舊的；全空的範例集不寫入。"""
        if examples.empty:
            return
        with _store_lock:
            entries = self._read()
            entries.pop(fingerprint, None)
            entries[fingerprint] = {name: getattr(examples, name) for name in _FIELDS}
            for dropped in list(entries)[:-MAX_ENTRIES]:
                del entries[dropped]
            self._write(entries)

    def _read(self) -> dict[str, dict]:
        if not self._path.exists():
            return {}
        try:
            raw = json.loads(self._path.read_text(encoding="utf-8"))["entries"]
            return {fp: {name: _pair_from_json(entry[name]) for name in _FIELDS}
                    for fp, entry in raw.items()}
        except Exception as exc:
            log(f"[translate] example cache unreadable ({self._path}: "
                f"{type(exc).__name__}: {exc}), treating as empty")
            return {}

    def _write(self, entries: dict[str, dict]) -> None:
        tmp_path: Path | None = None
        try:
            self._path.parent.mkdir(parents=True, exist_ok=True)
            fd, tmp_name = tempfile.mkstemp(dir=self._path.parent, prefix=f"{self._path.name}.")
            tmp_path = Path(tmp_name)
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                json.dump({"entries": entries}, f, ensure_ascii=False)
            os.replace(tmp_path, self._path)
            tmp_path = None
            log(f"[translate] example cache saved: {len(entries)} entries to {self._path}")
        except Exception as exc:
            log(f"[translate] example cache write failed ({self._path}: "
                f"{type(exc).__name__}: {exc})")
        finally:
            if tmp_path is not None:
                tmp_path.unlink(missing_ok=True)


class ExampleCoordinator:
    """決定範例何時走快取、何時背景生成，並把結果交給符合指紋的 translator。

    ensure 只能在 UI 執行緒呼叫；生成中與失敗集合只在 UI 執行緒讀寫，背景工作一律經 post 回來。
    """

    def __init__(self, store: ExampleStore, post: Callable[[Callable[[], None]], None],
                 spawn: Callable[[Callable[[], None]], None] | None = None,
                 on_applied: Callable[[object], None] | None = None):
        self._store = store
        self._post = post
        self._spawn = spawn or self._spawn_thread
        self._on_applied = on_applied
        self._translators: list = []
        self._pending: set[str] = set()
        self._failed: set[str] = set()

    @staticmethod
    def _spawn_thread(job: Callable[[], None]) -> None:
        threading.Thread(target=job, name="example-generation", daemon=True).start()

    def ensure(self, translator) -> None:
        """讓 translator 取得目前指紋的範例：命中快取就套用，否則背景生成（同指紋只生成一次）。"""
        if not any(known is translator for known in self._translators):
            self._translators.append(translator)
        fingerprint = translator.examples_fingerprint
        cached = self._store.get(fingerprint)
        if cached is not None:
            self._apply(translator, cached, fingerprint)
            log(f"[translate] examples cache hit ({translator.describe()}, fingerprint={fingerprint}, "
                f"{cached.summary()})")
            return
        if fingerprint in self._pending or fingerprint in self._failed:
            state = "pending" if fingerprint in self._pending else "failed"
            log(f"[translate] examples unavailable, translating without them "
                f"({translator.describe()}, fingerprint={fingerprint}, {state})")
            return
        self._pending.add(fingerprint)
        log(f"[translate] generating examples in background ({translator.describe()}, "
            f"fingerprint={fingerprint})")
        self._spawn(lambda: self._generate(translator, fingerprint))

    def _apply(self, translator, examples: ExampleSet, fingerprint: str) -> bool:
        if not translator.set_examples(examples, fingerprint):
            return False
        if self._on_applied is not None:
            self._on_applied(translator)
        return True

    def _generate(self, translator, requested: str) -> None:
        try:
            fingerprint, examples = translator.generate_examples()
        except Exception as exc:
            log(f"[translate] example generation failed ({translator.describe()}, "
                f"fingerprint={requested}): {type(exc).__name__}: {exc}")
            self._post(lambda: self._finish(requested, None, None))
            return
        if examples.empty:
            log(f"[translate] example generation failed validation on every path "
                f"({translator.describe()}, fingerprint={requested})")
            self._post(lambda: self._finish(requested, None, None))
            return
        self._store.put(fingerprint, examples)
        self._post(lambda: self._finish(requested, fingerprint, examples))

    def _finish(self, requested: str, fingerprint: str | None, examples: ExampleSet | None) -> None:
        self._pending.discard(requested)
        if examples is None:
            if any(t.examples_fingerprint == requested for t in self._translators):
                self._failed.add(requested)
            else:
                log(f"[translate] example generation abandoned, settings changed "
                    f"(fingerprint={requested})")
            return
        applied = [t for t in self._translators
                   if t.examples_fingerprint == fingerprint and self._apply(t, examples, fingerprint)]
        if applied:
            log(f"[translate] examples generated and applied to {len(applied)} translator(s) "
                f"({examples.summary()})")
        else:
            log(f"[translate] examples stored but not applied, no translator matches "
                f"(fingerprint={fingerprint})")
