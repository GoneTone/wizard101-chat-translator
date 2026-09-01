"""系統訊息譯文的持久化快取。

系統訊息不吃聊天上下文（見 translator.translate_system_message），因此「同一句原文
必然得到同一句譯文」——這正是它可以安全快取、而玩家對話不行的原因。

數字先正規化成佔位符再當 key：`你获得了 39 金币！` 與 `你获得了 65 金币！` 是同一個
句型，金額每次都不同，不正規化就永遠不會命中。
"""
import re

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
