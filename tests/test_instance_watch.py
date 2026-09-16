"""`src/instance_watch.py` 的測試。

專案規則：測試不可干擾使用者正在跑的遊戲或程式，砍程序、列舉真實程序一律禁止碰
真實系統，全部注入假物件。唯一碰真實系統的是 `ReadDirectoryChangesW` 對
`tmp_path` 開的一次性監看 —— 那本身就是這個功能唯一值得驗證的「真的會動」的部分。
"""
import os
import shutil
import sys
import threading
import time

import win32con
import win32file

from src.instance_watch import (
    _watch_iteration,
    is_sibling,
    pid_from_mei_name,
    start_instance_watch,
)

_FILE_LIST_DIRECTORY = 0x0001


def _open_watch_handle(path):
    return win32file.CreateFile(
        str(path), _FILE_LIST_DIRECTORY,
        win32con.FILE_SHARE_READ | win32con.FILE_SHARE_WRITE | win32con.FILE_SHARE_DELETE,
        None, win32con.OPEN_EXISTING, win32con.FILE_FLAG_BACKUP_SEMANTICS, None)


# ---------------------------------------------------------------------------
# pid_from_mei_name
# ---------------------------------------------------------------------------

def test_pid_from_mei_name_parses_hex_pid():
    # 已驗證的實例：bootloader PID 54736 = 0xd5d0，目錄名 _MEI0000d5d02
    assert pid_from_mei_name("_MEI0000d5d02") == 54736


def test_pid_from_mei_name_rejects_non_hex_suffix():
    assert pid_from_mei_name("_MEInothexx") is None


def test_pid_from_mei_name_rejects_non_mei_prefix():
    assert pid_from_mei_name("SomeOtherTempDir") is None


# ---------------------------------------------------------------------------
# is_sibling
# ---------------------------------------------------------------------------

def test_is_sibling_excludes_self():
    assert is_sibling(100, own_pid=100, parent_pid=200, own_exe="C:\\a.exe",
                      exe_path_of=lambda pid: "C:\\a.exe") is False


def test_is_sibling_excludes_parent_bootloader():
    assert is_sibling(200, own_pid=100, parent_pid=200, own_exe="C:\\a.exe",
                      exe_path_of=lambda pid: "C:\\a.exe") is False


def test_is_sibling_matches_path_case_insensitively():
    assert is_sibling(300, own_pid=100, parent_pid=200, own_exe="C:\\App\\Foo.EXE",
                      exe_path_of=lambda pid: "c:\\app\\foo.exe") is True


def test_is_sibling_false_when_exe_path_lookup_fails():
    assert is_sibling(300, own_pid=100, parent_pid=200, own_exe="C:\\a.exe",
                      exe_path_of=lambda pid: None) is False


def test_is_sibling_false_when_path_differs():
    assert is_sibling(300, own_pid=100, parent_pid=200, own_exe="C:\\a.exe",
                      exe_path_of=lambda pid: "C:\\other.exe") is False


# ---------------------------------------------------------------------------
# start_instance_watch
# ---------------------------------------------------------------------------

def test_start_instance_watch_returns_none_when_not_frozen(monkeypatch):
    monkeypatch.setattr(sys, "frozen", False, raising=False)

    class ExplodingThread:
        def __init__(self, *args, **kwargs):
            raise AssertionError("must not start a thread when not frozen")

    monkeypatch.setattr(threading, "Thread", ExplodingThread)
    assert start_instance_watch(lambda: None) is None


def test_start_instance_watch_disables_on_self_check_mismatch(monkeypatch, tmp_path):
    # 目錄名故意編一個跟真正的父程序 PID 不同的值，自我檢查該失敗、整個功能不啟用。
    wrong_pid = os.getppid() + 1
    mei_dir = tmp_path / f"_MEI{wrong_pid:08x}2"
    mei_dir.mkdir()
    monkeypatch.setattr(sys, "frozen", True, raising=False)
    monkeypatch.setattr(sys, "_MEIPASS", str(mei_dir), raising=False)

    class ExplodingThread:
        def __init__(self, *args, **kwargs):
            raise AssertionError("must not start a thread when the self-check fails")

    monkeypatch.setattr(threading, "Thread", ExplodingThread)
    assert start_instance_watch(lambda: None) is None


# ---------------------------------------------------------------------------
# 監看迴圈（真的對 tmp_path 跑一次 ReadDirectoryChangesW，其餘全部注入假物件）
# ---------------------------------------------------------------------------

def test_watch_iteration_preempts_sibling_and_cleans_up(tmp_path):
    own_exe = "C:\\fake\\Wizard101ChatTranslator.exe"
    sibling_pid = 424242
    sibling_dir = f"_MEI{sibling_pid:08x}2"
    calls = []

    def fake_exe_path_of(pid):
        assert pid == sibling_pid
        return own_exe

    def fake_terminate(pid):
        calls.append(("terminate", pid))
        return True

    def fake_alive(pid):
        return False   # 已經"死"了，清理不必等

    def fake_on_preempted():
        calls.append(("preempted",))

    def fake_enum_pids():
        raise AssertionError("a single clean event must not trigger the overflow fallback")

    handle = _open_watch_handle(tmp_path)
    try:
        thread = threading.Thread(
            target=_watch_iteration, args=(handle,),
            kwargs=dict(own_pid=os.getpid(), parent_pid=os.getppid(), own_exe=own_exe,
                       temp_root=str(tmp_path), own_mei_name="_MEI00000000x",
                       exe_path_of=fake_exe_path_of, terminate=fake_terminate,
                       alive=fake_alive, remove_tree=shutil.rmtree,
                       enum_pids=fake_enum_pids, on_preempted=fake_on_preempted),
            daemon=True)
        thread.start()
        time.sleep(0.3)   # 讓 ReadDirectoryChangesW 先進入阻塞等待，才建立目錄觸發事件
        (tmp_path / sibling_dir).mkdir()
        thread.join(timeout=5)
        assert not thread.is_alive()
    finally:
        win32file.CloseHandle(handle)

    assert calls == [("terminate", sibling_pid), ("preempted",)]
    assert not (tmp_path / sibling_dir).exists()


def test_watch_iteration_ignores_non_sibling_directory(tmp_path):
    other_pid = 555555
    other_dir = f"_MEI{other_pid:08x}2"
    calls = []

    def fake_exe_path_of(pid):
        return "C:\\some\\other-program.exe"   # 不是本程式，路徑對不上

    def fake_terminate(pid):
        calls.append(("terminate", pid))
        return True

    def fake_alive(pid):
        return False

    def fake_on_preempted():
        calls.append(("preempted",))

    def fake_enum_pids():
        raise AssertionError("a single clean event must not trigger the overflow fallback")

    handle = _open_watch_handle(tmp_path)
    try:
        thread = threading.Thread(
            target=_watch_iteration, args=(handle,),
            kwargs=dict(own_pid=os.getpid(), parent_pid=os.getppid(),
                       own_exe="C:\\fake\\Wizard101ChatTranslator.exe",
                       temp_root=str(tmp_path), own_mei_name="_MEI00000000x",
                       exe_path_of=fake_exe_path_of, terminate=fake_terminate,
                       alive=fake_alive, remove_tree=shutil.rmtree,
                       enum_pids=fake_enum_pids, on_preempted=fake_on_preempted),
            daemon=True)
        thread.start()
        time.sleep(0.3)
        (tmp_path / other_dir).mkdir()
        thread.join(timeout=5)
        assert not thread.is_alive()
    finally:
        win32file.CloseHandle(handle)

    assert calls == []
    assert (tmp_path / other_dir).exists()
