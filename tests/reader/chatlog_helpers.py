"""聊天 markup 測試共用的假資料產生器（不需遊戲）。"""


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


def _log(*lines: str) -> str:
    return "\n".join(lines)


def _say_colored(color: str, name: str, text: str) -> str:
    return (f"<color;{color}><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> "
            f"<link;GID:1,{name},2>[{name}]</link> {text} </color>")

