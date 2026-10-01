"""few-shot 範例集：請設定的模型把固定示範改寫成目標語言，並驗證三條路徑（收訊、系統訊息、區域）。"""
import hashlib
import json
import os
import re
import tempfile
import threading
from dataclasses import dataclass
from pathlib import Path

from src.config import local_state_dir
from src.log import log
from src.translation.postprocess import number_lines, unnumber_lines

EXAMPLE_REVISION = 1
EXAMPLES_PATH = local_state_dir() / "translation-examples.json"
MAX_ENTRIES = 32

SOURCE_LINES = (
    "[Amy] idk, Kai and I got new armor and learned Fire Cat at Colossus Boulevard lol, "
    "brb my wand is trash",
    "Kai taught you Fire Cat! Gained {0} gold at Colossus Boulevard.",
    "Talk to the Fire Cat",
    "Go to Colossus Boulevard",
    "and then you must",
)

DEMO_LINES = (
    "[Amy] 不知道耶，我和 Kai 拿到新護甲，還在巨像大道（Colossus Boulevard）學會了火貓（Fire Cat），"
    "笑死，等我一下，我的法杖超爛",
    "Kai 教會了你火貓（Fire Cat）！在巨像大道（Colossus Boulevard）獲得了 {0} 金幣。",
    "和火貓（Fire Cat）談談",
    "前往巨像大道（Colossus Boulevard）",
    "然後你必須",
)

_Pair = tuple[str, str]

_GENERATION_SYSTEM = (
    "把使用者給的每一行翻成 {t}，逐行對應、保留行首編號，只輸出譯文。"
    "格式照下面的中文示範：遊戲專有名詞翻成 {t} 後緊接括號照抄英文原文；"
    "[Amy]、人名 Kai、{{0}} 照抄不翻；縮寫與一般名詞直接翻、不加括號；"
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
        """內容的 sha1 前 12 碼，供 log 比對範例有沒有變。"""
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
    region=("1. 和火猫谈谈\n2. 前往巨像大道\n3. 然后你必须",
            "1. Talk to the Fire Cat (火猫)\n2. Go to Colossus Boulevard (巨像大道)"
            "\n3. and then you must"),
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
    return re.search(rf"[（(]\s*{re.escape(name)}\s*[)）]", line) is not None


def _check_incoming(line: str) -> list[str]:
    ok = "[Amy]" in line and "Kai" in line and _in_parentheses(line, "Fire Cat") \
        and _in_parentheses(line, "Colossus Boulevard")
    return [] if ok else ["line 1: needs [Amy], Kai and both names in parentheses"]


def _check_system(line: str) -> list[str]:
    ok = "Kai" in line and line.count("{0}") == 1 and _in_parentheses(line, "Fire Cat") \
        and _in_parentheses(line, "Colossus Boulevard")
    return [] if ok else ["line 2: needs Kai, one {0} and both names in parentheses"]


def _check_region(lines: list[str]) -> list[str]:
    checks = ((3, _in_parentheses(lines[0], "Fire Cat")),
              (4, _in_parentheses(lines[1], "Colossus Boulevard")),
              (5, bool(lines[2].strip())))
    return [f"line {n}: failed region check" for n, ok in checks if not ok]


def parse_generated(output: str) -> tuple[ExampleSet, list[str]]:
    """拆開模型生成的五行譯文並逐路徑驗證，回傳（通過的範例, 失敗說明）。

    缺編號的行會被 unnumber_lines 補回原文，所以與原文相同的行視同缺行。
    """
    lines = unnumber_lines(output, list(SOURCE_LINES)).splitlines()
    if len(lines) != len(SOURCE_LINES) or any(
            line.strip() == source for line, source in zip(lines, SOURCE_LINES, strict=True)):
        return EMPTY, [f"line count {len(lines)} does not match {len(SOURCE_LINES)} "
                       "or a line was left untranslated"]
    lines = [line.strip() for line in lines]
    incoming, system, region = _check_incoming(lines[0]), _check_system(lines[1]), _check_region(lines[2:])
    _, region_source = number_lines("\n".join(SOURCE_LINES[2:]))
    _, region_output = number_lines("\n".join(lines[2:]))
    return ExampleSet(
        incoming=None if incoming else (SOURCE_LINES[0], lines[0]),
        system=None if system else (SOURCE_LINES[1], lines[1]),
        region=None if region else (region_source, region_output),
    ), incoming + system + region


_FIELDS = ("incoming", "system", "region")
_store_lock = threading.Lock()


def _pair_from_json(value) -> _Pair | None:
    if value is None:
        return None
    original, translated = value
    if not (isinstance(original, str) and isinstance(translated, str)):
        raise TypeError(f"pair holds {type(original).__name__}/{type(translated).__name__}")
    return original, translated


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
