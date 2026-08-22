"""持久化 wizwalker 掛入的 unhook 狀態，供髒退出（Ctrl+C 硬砍/當機）後，
下次對**同一個仍在執行的遊戲程序**啟動時自動修復遺留的 hook —— 免重開遊戲。

狀態 = 一組 （位址， 原始bytes） 還原操作（autobot 區的原始 prologue + 每個 hook 的
jump 原碼）+ 當時的模組基址（module base）。依 PID 命名存檔。

同一 process（PID 不變）記憶體位址穩定，把原始 bytes 寫回原位址即等同 unhook。
**module base 防護**:PID 可能被作業系統重用給新的遊戲程序（ASLR 後基址不同），
還原前比對存檔基址與現行基址，不符即視為過期、不套用，避免寫到錯的位址。
"""
import json
import os
from pathlib import Path

APP_DIR = Path(os.environ.get("LOCALAPPDATA") or str(Path.home())) / "wizard101-chat-translator"


def _state_path(pid: int) -> Path:
    return APP_DIR / f"hookstate-{pid}.json"


def save_state(pid: int, base: int, ops: list[tuple[int, bytes]]) -> None:
    """存還原操作（位址， 原始bytes）與模組基址；掛入成功後呼叫。"""
    APP_DIR.mkdir(parents=True, exist_ok=True)
    data = {"base": base, "ops": [{"addr": addr, "bytes": b.hex()} for addr, b in ops]}
    _state_path(pid).write_text(json.dumps(data), encoding="utf-8")


def load_state(pid: int) -> tuple[int | None, list[tuple[int, bytes]]]:
    """讀還原操作；回傳 （module base, [（位址， 原始bytes）， ...]）。無檔或壞檔回傳 （None, []）。"""
    path = _state_path(pid)
    if not path.exists():
        return None, []
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        ops = [(int(e["addr"]), bytes.fromhex(e["bytes"])) for e in data["ops"]]
        return int(data["base"]), ops
    except Exception:
        return None, []


def clear_state(pid: int) -> None:
    """刪除該 PID 的狀態檔（乾淨關閉、或修復完成後呼叫）。"""
    try:
        _state_path(pid).unlink(missing_ok=True)
    except OSError:
        pass


def sweep(is_alive) -> None:
    """刪除已不在執行的 PID 的殘留狀態檔。is_alive(pid: int) -> bool。"""
    if not APP_DIR.exists():
        return
    for f in APP_DIR.glob("hookstate-*.json"):
        try:
            pid = int(f.stem.rsplit("-", 1)[1])
        except (ValueError, IndexError):
            continue
        if not is_alive(pid):
            try:
                f.unlink(missing_ok=True)
            except OSError:
                pass
