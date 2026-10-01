"""few-shot 範例集：請設定的模型把固定示範改寫成目標語言，並驗證三條路徑（收訊、系統訊息、區域）。"""
import hashlib
import re
from dataclasses import dataclass

from src.translation.postprocess import number_lines, unnumber_lines

EXAMPLE_REVISION = 1

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
