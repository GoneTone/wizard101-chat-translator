"""chatLog 測試共用的假資料產生器與腳本化的 FakeWiz（不需遊戲）。"""
from src.reader.mem_reader import WizChatReader


def _texts(lines):
    """只比對文字內容（顏色另有專門測試）。"""
    return [line.text for line in lines]


def _texts_colors(lines):
    """只比對文字與遊戲顯示色。"""
    return [(line.text, line.color) for line in lines]


def _say(gid: int, name: str, text: str) -> str:
    return (f"<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> "
            f"<link;GID:{gid},{name},2>[{name}]</link> {text} </color>")


def _system(text: str) -> str:
    return f"<color;00FF00><image;Art/Art_Chat_System.dds;24;24;FFFFFFFF> {text}</color>"


def _own(text: str) -> str:
    # 自己的發言：[你] 開頭，帶 Art_Chat 圖示但無 <link;GID>
    return f"<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [你] {text} </color>"


def _system_colored(color: str, text: str) -> str:
    return (f"<color;{color}><image;Art/Art_Chat_System.dds;24;24;FFFFFFFF> "
            f"{text}</color>")


def _broadcast(color: str, text: str) -> str:
    return f"<color;{color}>{text}</color>"


class FakeWiz(WizChatReader):
    """以腳本化的 chatLog 全文序列取代 wizwalker I/O。inputs＝每輪輸入框開關狀態。"""

    def __init__(self, texts, inputs=None, message_log=None):
        super().__init__(0x1, message_log=message_log)
        self.texts = texts
        self.inputs = list(inputs or [])
        self.n = 0
        self._connected = True

    def input_open(self):
        return self.inputs.pop(0) if self.inputs else False

    def _grab_texts(self) -> list[str]:
        i = min(self.n, len(self.texts) - 1)
        self.n += 1
        t = self.texts[i]
        if isinstance(t, Exception):
            raise t
        return t if isinstance(t, list) else [t]  # 腳本給 list＝多個 chatLog 節點


def _log(*lines: str) -> str:
    return "\n".join(lines)


def _say_colored(color: str, name: str, text: str) -> str:
    return (f"<color;{color}><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> "
            f"<link;GID:1,{name},2>[{name}]</link> {text} </color>")


def _burst_lines(n: int) -> list[str]:
    return [_say(2, "B", f"old{i}") for i in range(n)]


def _tab_switch_script(mine: str, other: str) -> list[str]:
    """前 6 輪停在同一視圖（建立基準並耗掉 RESET_WARMUP_POLLS 的暖機吸收期），
    其後在兩個視圖之間來回切，模擬切聊天頁籤。"""
    return [mine] * 6 + [other, mine, other, mine]
