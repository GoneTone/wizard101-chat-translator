"""隨視窗寬度重算文字換行寬度（wraplength）的共用工具。

Tk 的 `wraplength` 是固定像素值，視窗放大後說明文字仍卡在原本的寬度、右側留一
大片空白。把 Label 掛上 `bind_wrap()`，換行寬度就跟著容器寬度走。"""
import tkinter as tk

MIN_WRAP = 160  # 換行寬度下限：視窗被拖到極窄時仍讓文字保有可讀寬度
# 說明文字換行時的右側預留：欄位自己的內距（8）＋容器內距（12）＋一點餘裕。
# 少扣了就會把說明的最後一兩個字切在視窗右緣外。
HINT_TRAILING = 24


def wrap_width(container_width: int, reserved: int = 0,
               minimum: int = MIN_WRAP) -> int:
    """容器寬度扣掉 reserved（padding、同列的其他控件）後的換行寬度，不低於 minimum。"""
    return max(minimum, container_width - reserved)


def _current_wrap(label: tk.Widget) -> int:
    """label 目前的 wraplength；沒設定過時 ttk 回空字串、tk 回 0，一律視為 0。"""
    try:
        return int(label.cget("wraplength"))
    except (ValueError, tk.TclError):
        return 0


def apply_wrap(label: tk.Widget, container_width: int, reserved: int = 0,
               minimum: int = MIN_WRAP) -> bool:
    """把算出的換行寬度套到 label；值有變才寫入，並回傳是否真的變了。

    「值有變才寫入」是必要的：改 wraplength 會重排版、重排版又發 Configure，
    無條件寫入就成了無窮回圈。"""
    width = wrap_width(container_width, reserved, minimum)
    if _current_wrap(label) == width:
        return False
    label.configure(wraplength=width)
    return True


def bind_wrap(label: tk.Widget, container: tk.Misc | None = None,
              trailing: int = 8, minimum: int = MIN_WRAP) -> None:
    """讓 label 的換行寬度跟著 container（預設為 label 的父容器）的寬度走。

    預留寬度由 label 在容器內的 x 推算 —— 同一列左邊還有標籤／輸入框時，可用的
    只有它右邊剩下的空間；trailing 是右緣再留的邊距。container 若不是 label 的
    直接父容器，這個推算就不成立（winfo_x 是對父容器的座標）。"""
    box = container if container is not None else label.master
    pending = False

    def _recompute() -> None:
        nonlocal pending
        pending = False
        # 視窗尚未 map 時寬度回報為 1：這時算出的換行寬度沒有意義，等真正排版後的
        # Configure 再套；label 也可能已被銷毀（切步驟、關窗）。
        if not label.winfo_exists() or box.winfo_width() <= 1:
            return
        apply_wrap(label, box.winfo_width(), label.winfo_x() + trailing, minimum)

    def _schedule(_event=None) -> None:
        # 排到 idle 才量：Configure 當下的寬度與 label 位置都還是排版中的過渡值，
        # 直接拿來算會讓同一個視窗寬度收斂到不同的換行寬度（行數因此忽多忽少）。
        nonlocal pending
        if pending or not label.winfo_exists():
            return
        pending = True
        label.after_idle(_recompute)

    box.bind("<Configure>", _schedule, add="+")
    _schedule()
