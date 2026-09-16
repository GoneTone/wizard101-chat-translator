"""偵測並攔截「兄弟」bootloader 的第二次啟動。

onefile exe 被再次雙擊時，新的 bootloader 解壓、顯示啟動畫面之後才由既有的 mutex
檢查發現已有實例、關掉畫面、喚起舊視窗；解壓之前我們的程式碼還沒執行，第二份
實例救不了自己。本模組讓第一份存活的實例主動偵測到兄弟啟動，砍掉其 bootloader、
喚起自己視窗、清掉它的半成品暫存目錄；沒攔到時（自我檢查失敗、權限不足等）mutex
檢查仍是保底。

只在 frozen（打包版）模式下生效。內部拆成可獨立測試的純函式，砍程序、列舉程序、
刪目錄一律可注入替換，測試不得碰真實系統。
"""
import os
import shutil
import sys
import threading
import time
from collections.abc import Callable

from src.log import log
from src.reader.process import pid_alive, process_exe_path

_MEI_PREFIX = "_MEI"
# pywin32 沒有把這兩個 Win32 常數包成 win32con 屬性，直接照官方文件的數值定義
_FILE_LIST_DIRECTORY = 0x0001
_FILE_ACTION_ADDED = 1
_CLEANUP_WAIT_SECONDS = 3.0


def pid_from_mei_name(name: str) -> int | None:
    """從 onefile bootloader 的暫存目錄名解出建立它的 bootloader PID。

    命名規則（已驗證，見 `start_instance_watch` 的自我檢查）：`_MEI` ＋ 8 位十六進位
    PID ＋ 一個意義未確認的尾碼字元，例如 `_MEI0000d5d02` 對應 PID `0xd5d0`＝54736。
    不是 `_MEI` 開頭或十六進位解析失敗一律回 None。
    """
    if not name.startswith(_MEI_PREFIX):
        return None
    hex_part = name[len(_MEI_PREFIX):-1]   # 去掉字首與尾碼那個意義未知的字元
    try:
        return int(hex_part, 16)
    except ValueError:
        return None


def is_sibling(pid: int, *, own_pid: int, parent_pid: int, own_exe: str,
              exe_path_of: Callable[[int], str | None]) -> bool:
    """pid 是否為本程序的「兄弟」 —— 同一個 exe 另外啟動的一份，但排除自己與父程序
    （bootloader）本身；兩者都絕不能砍。exe 路徑比對不分大小寫；`exe_path_of` 查不到
    （程序已結束、權限不足）一律當作不是兄弟。
    """
    if pid in (own_pid, parent_pid):
        return False
    exe = exe_path_of(pid)
    if exe is None:
        return False
    return exe.lower() == own_exe.lower()


def _terminate_pid(pid: int) -> bool:
    """真正砍掉 pid（frozen 執行期預設實作）：`OpenProcess(PROCESS_TERMINATE)` 再
    `TerminateProcess`。對方若以系統管理員身分執行會拒絕存取，回 False 讓呼叫端記 log
    後放棄，交由對方自己的 mutex 檢查兜底。
    """
    import win32api
    import win32con

    try:
        handle = win32api.OpenProcess(win32con.PROCESS_TERMINATE, False, pid)
    except Exception as exc:
        log(f"[instance] OpenProcess failed: pid={pid} error={type(exc).__name__}: {exc}")
        return False
    try:
        win32api.TerminateProcess(handle, 0)
    except Exception as exc:
        log(f"[instance] TerminateProcess failed: pid={pid} error={type(exc).__name__}: {exc}")
        return False
    finally:
        win32api.CloseHandle(handle)
    return True


def _enum_pids() -> list[int]:
    """列舉目前所有程序 PID（frozen 執行期預設實作）：`ReadDirectoryChangesW` 的緩衝區
    溢位時，當一次性的後備掃描用。
    """
    import win32process
    return win32process.EnumProcesses()


def _cleanup_sibling_dir(temp_root: str, pid: int, own_mei_name: str, *,
                         alive: Callable[[int], bool],
                         remove_tree: Callable[..., None]) -> None:
    """等剛砍掉的 pid 真的死透（最多等 `_CLEANUP_WAIT_SECONDS` 秒）再清它的殘留目錄。

    只刪 `temp_root` 底下「目錄名解出的 PID 剛好等於 pid」的項目，且一律跳過
    `own_mei_name`（對應自己的 `sys._MEIPASS`，絕不能刪）。寧可留下幾 MB 殘骸，也不能
    刪錯別的程式的目錄。
    """
    deadline = time.monotonic() + _CLEANUP_WAIT_SECONDS
    while alive(pid) and time.monotonic() < deadline:
        time.sleep(0.1)
    if alive(pid):
        log(f"[instance] cleanup skipped: pid={pid} still alive after "
            f"{_CLEANUP_WAIT_SECONDS}s wait")
        return
    try:
        entries = os.listdir(temp_root)
    except OSError as exc:
        log(f"[instance] cleanup listdir failed: temp_root={temp_root} error={exc}")
        return
    for entry in entries:
        if entry == own_mei_name:
            continue   # 絕不刪自己的 sys._MEIPASS
        if pid_from_mei_name(entry) != pid:
            continue
        path = os.path.join(temp_root, entry)
        remove_tree(path, ignore_errors=True)
        log(f"[instance] cleaned up sibling temp dir: path={path} pid={pid}")


def _preempt(pid: int, exe: str | None, *, temp_root: str, own_mei_name: str,
            terminate: Callable[[int], bool], alive: Callable[[int], bool],
            remove_tree: Callable[..., None],
            on_preempted: Callable[[], None]) -> None:
    """對一個已確認是兄弟的 pid 執行完整攔截順序：先砍程序 —— 啟動畫面 always-on-top，
    不先關掉會蓋住我們的視窗 —— 再呼叫 `on_preempted` 喚起自己的視窗，最後清理對方的
    殘留目錄。
    """
    log(f"[instance] sibling detected: pid={pid} exe={exe}")
    if not terminate(pid):
        log(f"[instance] terminate failed for pid={pid} (access denied?); "
            f"leaving it to its own single-instance check")
        return
    log(f"[instance] sibling terminated: pid={pid}")
    on_preempted()
    _cleanup_sibling_dir(temp_root, pid, own_mei_name, alive=alive, remove_tree=remove_tree)


def _maybe_preempt(name: str, *, own_pid: int, parent_pid: int, own_exe: str,
                   temp_root: str, own_mei_name: str,
                   exe_path_of: Callable[[int], str | None],
                   terminate: Callable[[int], bool], alive: Callable[[int], bool],
                   remove_tree: Callable[..., None],
                   on_preempted: Callable[[], None]) -> None:
    """單一個新建目錄名字的處理入口：解不出 PID、或不是兄弟就忽略 —— 可能是別的
    PyInstaller 程式，也可能是自己。
    """
    pid = pid_from_mei_name(name)
    if pid is None:
        return
    if not is_sibling(pid, own_pid=own_pid, parent_pid=parent_pid, own_exe=own_exe,
                      exe_path_of=exe_path_of):
        return
    _preempt(pid, exe_path_of(pid), temp_root=temp_root, own_mei_name=own_mei_name,
             terminate=terminate, alive=alive, remove_tree=remove_tree,
             on_preempted=on_preempted)


def _scan_and_preempt(pids: list[int], *, own_pid: int, parent_pid: int, own_exe: str,
                      temp_root: str, own_mei_name: str,
                      exe_path_of: Callable[[int], str | None],
                      terminate: Callable[[int], bool], alive: Callable[[int], bool],
                      remove_tree: Callable[..., None],
                      on_preempted: Callable[[], None]) -> None:
    """`ReadDirectoryChangesW` 緩衝區溢位時的一次性後備掃描：直接列舉目前所有程序找
    兄弟，而不是默默假設沒事發生。
    """
    for pid in pids:
        if is_sibling(pid, own_pid=own_pid, parent_pid=parent_pid, own_exe=own_exe,
                      exe_path_of=exe_path_of):
            _preempt(pid, exe_path_of(pid), temp_root=temp_root, own_mei_name=own_mei_name,
                     terminate=terminate, alive=alive, remove_tree=remove_tree,
                     on_preempted=on_preempted)


def _handle_batch(results: list[tuple[int, str]], *,
                  enum_pids: Callable[[], list[int]], **ctx) -> None:
    """處理一次 `ReadDirectoryChangesW` 回傳的批次：空清單視為緩衝區可能溢位，改走
    一次性程序掃描；否則只看 `FILE_ACTION_ADDED` 且以 `_MEI` 開頭的項目。
    """
    if not results:
        log("[instance] change buffer overflowed; falling back to a full process scan")
        _scan_and_preempt(enum_pids(), **ctx)
        return
    for action, name in results:
        if action == _FILE_ACTION_ADDED and name.startswith(_MEI_PREFIX):
            _maybe_preempt(name, **ctx)


def _watch_iteration(handle, *, enum_pids: Callable[[], list[int]],
                     ready: threading.Event | None = None, **ctx) -> None:
    """跑一輪監看：阻塞呼叫一次 `ReadDirectoryChangesW`，處理該批次結果；拆成單輪
    函式讓測試能只跑一輪，不必掛著永遠不停的迴圈。

    `ready`（測試專用掛鉤）：在阻塞呼叫之前設好，讓測試知道監看已開始，不必用固定
    sleep 賭時間差；正式呼叫不傳這個參數。
    """
    import win32con
    import win32file

    if ready is not None:
        ready.set()
    results = win32file.ReadDirectoryChangesW(
        handle, 8192, False, win32con.FILE_NOTIFY_CHANGE_DIR_NAME, None, None)
    _handle_batch(results, enum_pids=enum_pids, **ctx)


def _watch_loop(temp_root: str, *, own_pid: int, parent_pid: int, own_exe: str,
                own_mei_name: str, on_preempted: Callable[[], None],
                exe_path_of: Callable[[int], str | None] = process_exe_path,
                terminate: Callable[[int], bool] = _terminate_pid,
                alive: Callable[[int], bool] = pid_alive,
                remove_tree: Callable[..., None] = shutil.rmtree,
                enum_pids: Callable[[], list[int]] = _enum_pids) -> None:
    """監看執行緒本體：對 `temp_root` 開 `ReadDirectoryChangesW`，事件驅動、零輪詢，
    永久重複 `_watch_iteration` 直到例外真的逃出去或程序結束。單輪例外只記 log、不
    中斷迴圈；迴圈結束時（不論原因）也留一行 log，不能無聲消失。
    """
    import win32con
    import win32file

    log(f"[instance] watch loop starting: temp_root={temp_root}")
    ctx = dict(own_pid=own_pid, parent_pid=parent_pid, own_exe=own_exe,
              temp_root=temp_root, own_mei_name=own_mei_name,
              exe_path_of=exe_path_of, terminate=terminate, alive=alive,
              remove_tree=remove_tree, on_preempted=on_preempted)
    handle = None
    try:
        try:
            handle = win32file.CreateFile(
                temp_root, _FILE_LIST_DIRECTORY,
                win32con.FILE_SHARE_READ | win32con.FILE_SHARE_WRITE | win32con.FILE_SHARE_DELETE,
                None, win32con.OPEN_EXISTING, win32con.FILE_FLAG_BACKUP_SEMANTICS, None)
        except Exception as exc:
            log(f"[instance] CreateFile failed for {temp_root}: {type(exc).__name__}: {exc}")
            return
        while True:
            try:
                _watch_iteration(handle, enum_pids=enum_pids, **ctx)
            except Exception as exc:
                log(f"[instance] watch loop error: {type(exc).__name__}: {exc}")
    finally:
        # 正常情況下這個迴圈不會退出（daemon 執行緒隨行程 os._exit 一起消失，handle
        # 沒機會也不需要關）；這裡只是讓「萬一真的退出」時不留一個沒關的 handle。
        if handle is not None:
            try:
                win32file.CloseHandle(handle)
            except Exception:
                pass
        log("[instance] watch loop exited")


def start_instance_watch(on_preempted: Callable[[], None]) -> threading.Thread | None:
    """frozen 模式下啟動監看執行緒（daemon）並回傳；非 frozen 或自我檢查失敗回 None。

    自我檢查：`pid_from_mei_name(basename(sys._MEIPASS)) == os.getppid()` 必須成立 ——
    這是「目錄名編了 bootloader 父程序 PID」這個假設，在這台機器、這個 PyInstaller
    版本上的直接驗證。不成立就整個功能不啟用，因為砍程序與刪目錄都建立在這個假設上。

    `on_preempted` 在成功終止一個兄弟程序後、於監看執行緒（非 Tk 主執行緒）上呼叫 ——
    呼叫端必須自己把 UI 動作丟進 `ui_queue`。
    """
    if not getattr(sys, "frozen", False):
        log("[instance] watch disabled: not a frozen build")
        return None
    own_pid = os.getpid()
    parent_pid = os.getppid()
    own_exe = sys.executable
    mei_path = sys._MEIPASS
    own_mei_name = os.path.basename(mei_path)
    temp_root = os.path.dirname(mei_path)
    decoded = pid_from_mei_name(own_mei_name)
    if decoded != parent_pid:
        log(f"[instance] watch disabled: self-check failed, decoded_pid={decoded!r} "
            f"parent_pid={parent_pid} mei_name={own_mei_name!r}")
        return None
    log(f"[instance] watch enabled: temp_root={temp_root} own_pid={own_pid} "
        f"parent_pid={parent_pid} mei_name={own_mei_name!r}")
    thread = threading.Thread(
        target=_watch_loop, args=(temp_root,),
        kwargs=dict(own_pid=own_pid, parent_pid=parent_pid, own_exe=own_exe,
                   own_mei_name=own_mei_name, on_preempted=on_preempted),
        name="instance-watch", daemon=True)
    thread.start()
    return thread
