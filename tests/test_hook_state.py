import json

import pytest

from src.reader import hook_state


@pytest.fixture(autouse=True)
def tmp_appdir(tmp_path, monkeypatch):
    monkeypatch.setattr(hook_state, "STATE_DIR", tmp_path / "state")
    return tmp_path / "state"


def test_save_load_roundtrip():
    ops = [(0x140000000, b"\x48\x89\x5c\x24"), (0x141234567, b"\xe9\x00\x11\x22\x33\x44")]
    hook_state.save_state(4321, 0x140000000, ops)
    base, got = hook_state.load_state(4321)
    assert base == 0x140000000
    assert got == ops


def test_load_missing_returns_empty():
    assert hook_state.load_state(9999) == (None, [])


def test_clear_removes_file():
    hook_state.save_state(4321, 0x1000, [(0x2000, b"\xab")])
    hook_state.clear_state(4321)
    assert hook_state.load_state(4321) == (None, [])


def test_clear_missing_is_noop():
    hook_state.clear_state(4321)  # 不存在也不拋錯


def test_corrupt_file_returns_empty(tmp_appdir):
    tmp_appdir.mkdir(parents=True, exist_ok=True)
    (tmp_appdir / "hookstate-4321.json").write_text("{ not json", encoding="utf-8")
    assert hook_state.load_state(4321) == (None, [])


def test_empty_ops_roundtrip():
    hook_state.save_state(4321, 0x1000, [])
    base, got = hook_state.load_state(4321)
    assert base == 0x1000
    assert got == []


def test_sweep_removes_dead_pid_files_only():
    hook_state.save_state(111, 0x1000, [(0x2000, b"\x01")])   # 存活
    hook_state.save_state(222, 0x1000, [(0x2000, b"\x02")])   # 已死
    hook_state.sweep(is_alive=lambda pid: pid == 111)
    assert hook_state.load_state(111) != (None, [])
    assert hook_state.load_state(222) == (None, [])


def test_sweep_ignores_non_state_files(tmp_appdir):
    tmp_appdir.mkdir(parents=True, exist_ok=True)
    (tmp_appdir / "notes.txt").write_text("keep me", encoding="utf-8")
    hook_state.sweep(is_alive=lambda pid: False)
    assert (tmp_appdir / "notes.txt").exists()
