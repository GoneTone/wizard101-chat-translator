use crate::common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::fake_game::{
    ADDLINE_NEW_BUILD, BASE, RING_SIZE, ROOT_NEW_BUILD, place_addline, place_root_window,
};
use common::log_capture::{capture_logs, log_lines_with};
use serde_json::Value;
use wizard101_chat_translator::game::memory::{FakeMemory, MemOp};
use wizard101_chat_translator::reader::hook::add_line::{self, AddLineHook};
use wizard101_chat_translator::reader::hook::cave::{ARENA_SIZE, MODULE, StubArena};
use wizard101_chat_translator::reader::hook::root_window::{self, RootWindowHook};
use wizard101_chat_translator::reader::hook::state as hook_state;
use wizard101_chat_translator::util;

/// FakeMemory 的 `alloc_near` 第一次回傳的位址。
const ARENA: u64 = BASE - 0x1000_0000;

fn hex_of(v: &Value) -> Vec<u8> {
    util::unhex(v.as_str().unwrap()).unwrap()
}

struct Game {
    mem: FakeMemory,
    root_jump: u64,
    addline_jump: u64,
    /// 跳轉點起的原始位元組。
    root_bytes: Vec<u8>,
    addline_bytes: Vec<u8>,
}

/// `addline_jump` 是 `lea` 的位址；函式開頭由 hex 裡 `lea rbp,[rsp+disp32]` 的位置反推。
fn game(root_jump: u64, root_hex: &str, addline_jump: u64, addline_hex: &str) -> Game {
    let mem = FakeMemory::new();
    mem.add_module(MODULE, BASE);
    let root_bytes = place_root_window(&mem, root_jump, root_hex);
    let lea = util::unhex(addline_hex)
        .unwrap()
        .windows(4)
        .position(|w| w == [0x48, 0x8D, 0xAC, 0x24])
        .unwrap();
    let addline_bytes = place_addline(&mem, addline_jump - lea as u64, addline_hex)[lea..].to_vec();
    Game {
        mem,
        root_jump,
        addline_jump,
        root_bytes,
        addline_bytes,
    }
}

fn default_game() -> Game {
    game(
        0x1_4080_0000,
        ROOT_NEW_BUILD,
        0x1_4120_0010,
        ADDLINE_NEW_BUILD,
    )
}

fn n(v: &Value) -> u64 {
    v.as_u64().unwrap()
}

/// 依 golden 案例放好兩處跳轉點，定位後配置 stub 區並掛上兩個 hook，回傳記下的寫入／配置。
fn install_case(case: &Value) -> Vec<MemOp> {
    let input = &case["input"];
    let g = game(
        n(&input["root_jump"]),
        input["root_hex"].as_str().unwrap(),
        n(&input["addline_jump"]),
        input["addline_hex"].as_str().unwrap(),
    );
    let root = root_window::locate(&g.mem).unwrap();
    let addline = add_line::locate(&g.mem).unwrap();
    assert_eq!((root.jump, addline.jump), (g.root_jump, g.addline_jump));
    let mut arena = StubArena::install(&g.mem).unwrap();
    assert_eq!(arena.addr(), n(&case["output"]["arena"]));
    g.mem.set_next_alloc(n(&input["export"]));
    let hook = RootWindowHook::install(&g.mem, &mut arena, &root).unwrap();
    assert_eq!(hook.export_addr(), n(&input["export"]));
    g.mem.set_next_alloc(n(&input["ring"]));
    let hook = AddLineHook::install(&g.mem, &mut arena, &addline).unwrap();
    assert_eq!(hook.ring_addr(), n(&input["ring"]));
    g.mem.ops()
}

#[test]
fn root_window_stub_and_jump_match_golden() {
    let cases = common::golden("hook_stubs");
    assert!(cases.len() >= 3);
    for case in &cases {
        let input = &case["input"];
        let out = &case["output"];
        let root = &out["root"];
        assert!(hex_of(&root["stub"]).len() <= 50);
        let ops = install_case(case);
        assert_eq!(
            ops[..4],
            [
                MemOp::Alloc {
                    addr: n(&out["arena"]),
                    size: ARENA_SIZE
                },
                MemOp::Alloc {
                    addr: n(&input["export"]),
                    size: 8
                },
                MemOp::Write {
                    addr: n(&root["stub_addr"]),
                    bytes: hex_of(&root["stub"])
                },
                MemOp::Write {
                    addr: n(&input["root_jump"]),
                    bytes: hex_of(&root["jump"])
                },
            ]
        );
    }
}

#[test]
fn addline_stub_and_jump_match_golden() {
    for case in &common::golden("hook_stubs") {
        let input = &case["input"];
        let addline = &case["output"]["addline"];
        assert!(hex_of(&addline["stub"]).len() <= 256);
        let ops = install_case(case);
        assert_eq!(
            ops[4..],
            [
                MemOp::Alloc {
                    addr: n(&input["ring"]),
                    size: RING_SIZE
                },
                MemOp::Write {
                    addr: n(&addline["stub_addr"]),
                    bytes: hex_of(&addline["stub"])
                },
                MemOp::Write {
                    addr: n(&input["addline_jump"]),
                    bytes: hex_of(&addline["jump"])
                },
            ]
        );
    }
}

#[test]
fn arena_is_allocated_near_the_module_and_sliced_in_order() {
    let g = default_game();
    let mut arena = StubArena::install(&g.mem).unwrap();
    assert_eq!(
        g.mem.ops(),
        [MemOp::Alloc {
            addr: ARENA,
            size: ARENA_SIZE
        }]
    );
    assert_eq!(arena.addr(), ARENA);
    assert!(arena.restore_ops().is_empty());
    assert_eq!(arena.alloc_stub(50).unwrap(), ARENA);
    assert_eq!(arena.alloc_stub(256).unwrap(), ARENA + 50);
}

struct Installed {
    g: Game,
    arena: StubArena,
    export: u64,
    ring: u64,
}

fn install_all() -> Installed {
    let g = default_game();
    let root = root_window::locate(&g.mem).unwrap();
    let addline = add_line::locate(&g.mem).unwrap();
    let mut arena = StubArena::install(&g.mem).unwrap();
    let export = RootWindowHook::install(&g.mem, &mut arena, &root)
        .unwrap()
        .export_addr();
    let ring = AddLineHook::install(&g.mem, &mut arena, &addline)
        .unwrap()
        .ring_addr();
    Installed {
        g,
        arena,
        export,
        ring,
    }
}

#[test]
fn uninstall_restores_jumps_then_frees_after_waiting() {
    let Installed {
        g,
        arena,
        export,
        ring: _,
    } = install_all();
    assert_eq!(
        arena.restore_ops(),
        [
            (g.root_jump, g.root_bytes[..7].to_vec()),
            (g.addline_jump, g.addline_bytes[..8].to_vec()),
        ]
    );
    let before = g.mem.ops().len();
    let mut waited = None;
    arena
        .uninstall_with(&g.mem, |d| waited = Some((d, g.mem.ops().len())))
        .unwrap();
    assert_eq!(waited, Some((Duration::from_millis(500), before + 2)));
    assert_eq!(
        g.mem.ops()[before..],
        [
            MemOp::Write {
                addr: g.root_jump,
                bytes: g.root_bytes[..7].to_vec()
            },
            MemOp::Write {
                addr: g.addline_jump,
                bytes: g.addline_bytes[..8].to_vec()
            },
            MemOp::Free(export),
            MemOp::Free(ARENA),
        ]
    );
}

#[test]
fn ring_is_never_freed() {
    let Installed {
        g,
        arena,
        export,
        ring,
    } = install_all();
    arena.uninstall_with(&g.mem, |_| {}).unwrap();
    let ops = g.mem.ops();
    assert!(ops.contains(&MemOp::Free(export)));
    assert!(!ops.contains(&MemOp::Free(ring)));

    let g = default_game();
    let addline = add_line::locate(&g.mem).unwrap();
    let mut arena = StubArena::install(&g.mem).unwrap();
    AddLineHook::install(&g.mem, &mut arena, &addline).unwrap();
    let before = g.mem.ops().len();
    arena.uninstall_with(&g.mem, |_| {}).unwrap();
    assert_eq!(
        g.mem.ops()[before..],
        [
            MemOp::Write {
                addr: g.addline_jump,
                bytes: g.addline_bytes[..8].to_vec()
            },
            MemOp::Free(ARENA),
        ]
    );
}

#[test]
fn uninstall_on_a_closed_game_writes_nothing() {
    let Installed { g, arena, .. } = install_all();
    let before = g.mem.ops().len();
    g.mem.kill();
    arena
        .uninstall_with(&g.mem, |_| panic!("must not wait"))
        .unwrap();
    assert_eq!(g.mem.ops().len(), before);
}

fn ops_of(input: &Value) -> Vec<(u64, Vec<u8>)> {
    input["ops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| (n(&op[0]), hex_of(&op[1])))
        .collect()
}

fn pid_of(input: &Value) -> u32 {
    n(&input["pid"]) as u32
}

#[test]
fn hookstate_roundtrips_the_golden_inputs() {
    let cases = common::golden("hookstate_bytes");
    assert!(!cases.is_empty());
    let dir = tempfile::tempdir().unwrap();
    for case in &cases {
        let input = &case["input"];
        let pid = pid_of(input);
        hook_state::save_state(dir.path(), pid, n(&input["base"]), &ops_of(input)).unwrap();
        let base = n(&input["base"]);
        assert_eq!(
            hook_state::load_state(dir.path(), pid),
            (Some(base), ops_of(input))
        );
    }
}

/// 把 Python `save_state` 寫的檔案放進暫存 state 目錄，回傳（目錄, pid, base, ops）。
fn python_state() -> (tempfile::TempDir, u32, u64, Vec<(u64, Vec<u8>)>) {
    let case = &common::golden("hookstate_bytes")[0];
    let input = &case["input"];
    let pid = pid_of(input);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(format!("hookstate-{pid}.json")),
        case["output"].as_str().unwrap(),
    )
    .unwrap();
    (dir, pid, n(&input["base"]), ops_of(input))
}

fn state_file(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("hookstate-{pid}.json"))
}

#[test]
fn repairs_from_python_written_hookstate() {
    let (dir, pid, base, ops) = python_state();
    assert_eq!(ops.len(), 2);
    let mem = FakeMemory::new();
    mem.add_module(MODULE, base);
    for (addr, original) in &ops {
        mem.set_bytes(*addr, &vec![0xE9; original.len()]);
    }
    assert_eq!(hook_state::repair(dir.path(), &mem, pid, 0), 2);
    let expected: Vec<MemOp> = ops
        .iter()
        .map(|(addr, bytes)| MemOp::Write {
            addr: *addr,
            bytes: bytes.clone(),
        })
        .collect();
    assert_eq!(mem.ops(), expected);
    assert!(!state_file(dir.path(), pid).exists());

    let (dir, pid, base, ops) = python_state();
    let mem = FakeMemory::new();
    mem.add_module(MODULE, base + 0x1_0000);
    for (addr, original) in &ops {
        mem.set_bytes(*addr, &vec![0xE9; original.len()]);
    }
    assert_eq!(hook_state::repair(dir.path(), &mem, pid, 0), 0);
    assert!(mem.ops().is_empty());
    assert!(!state_file(dir.path(), pid).exists());
}

#[test]
fn sweep_removes_dead_pids_only() {
    let dir = tempfile::tempdir().unwrap();
    hook_state::save_state(dir.path(), 111, 0x1000, &[(0x2000, vec![1])]).unwrap();
    hook_state::save_state(dir.path(), 222, 0x1000, &[(0x2000, vec![2])]).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "keep me").unwrap();
    std::fs::write(dir.path().join("hookstate-abc.json"), "{}").unwrap();
    hook_state::sweep(dir.path(), |pid| pid == 111);
    assert_ne!(hook_state::load_state(dir.path(), 111), (None, vec![]));
    assert_eq!(hook_state::load_state(dir.path(), 222), (None, vec![]));
    assert!(dir.path().join("notes.txt").exists());
    assert!(dir.path().join("hookstate-abc.json").exists());
    hook_state::sweep(&dir.path().join("missing"), |_| false);
}

#[test]
fn repair_reports_write_failures_instead_of_claiming_success() {
    capture_logs();
    let dir = tempfile::tempdir().unwrap();
    hook_state::save_state(
        dir.path(),
        770_001,
        0x1000,
        &[(0x2000, vec![0xAB]), (0x3000, vec![0xCD])],
    )
    .unwrap();
    let mem = FakeMemory::new();
    mem.add_module(MODULE, 0x1000);
    mem.set_bytes(0x2000, &[0]);
    assert_eq!(hook_state::repair(dir.path(), &mem, 770_001, 2), 1);
    assert_eq!(
        mem.ops(),
        [MemOp::Write {
            addr: 0x2000,
            bytes: vec![0xAB]
        }]
    );
    let lines = log_lines_with("pid=770001");
    assert!(
        lines.iter().any(|l| l.contains(
            "[reader] repaired hooks leaked by previous dirty exit (writes=1, failed=1, slot=2, \
             pid=770001), restart the game if chat stays silent"
        )),
        "{lines:?}"
    );
    assert_eq!(
        log_lines_with(
            "[reader] hook repair write failed at 0x3000 (slot=2, len=1): \
             Unable to write memory at address 12288."
        )
        .len(),
        1
    );
    assert_eq!(hook_state::load_state(dir.path(), 770_001), (None, vec![]));
}

#[test]
fn repair_writes_every_op_in_order_without_waiting() {
    capture_logs();
    let dir = tempfile::tempdir().unwrap();
    hook_state::save_state(
        dir.path(),
        770_004,
        0x1000,
        &[
            (0x2000, vec![0xAB]),
            (0x3000, vec![0xCD]),
            (0x4000, vec![0xEF]),
        ],
    )
    .unwrap();
    let mem = FakeMemory::new();
    mem.add_module(MODULE, 0x1000);
    mem.set_bytes(0x2000, &[0]);
    mem.set_bytes(0x4000, &[0]);
    assert_eq!(hook_state::repair(dir.path(), &mem, 770_004, 3), 2);
    assert_eq!(
        mem.ops(),
        [
            MemOp::Write {
                addr: 0x2000,
                bytes: vec![0xAB]
            },
            MemOp::Write {
                addr: 0x4000,
                bytes: vec![0xEF]
            },
        ]
    );
    assert_eq!(
        log_lines_with(
            "[reader] repaired hooks leaked by previous dirty exit (writes=2, failed=1, slot=3, \
             pid=770004), restart the game if chat stays silent"
        )
        .len(),
        1
    );
}

#[test]
fn repair_skips_and_logs_a_stale_state_file() {
    capture_logs();
    let dir = tempfile::tempdir().unwrap();
    hook_state::save_state(dir.path(), 770_002, 0x1000, &[(0x2000, vec![0xAB])]).unwrap();
    let mem = FakeMemory::new();
    mem.add_module(MODULE, 0x9000);
    mem.set_bytes(0x2000, &[0]);
    assert_eq!(hook_state::repair(dir.path(), &mem, 770_002, 1), 0);
    assert!(mem.ops().is_empty());
    let lines = log_lines_with("pid=770002");
    assert!(
        lines.iter().any(|l| l.contains(
            "[reader] stale hook state ignored (slot=1, pid=770002 was reused: \
             saved_base=0x1000, current_base=0x9000)"
        )),
        "{lines:?}"
    );
    assert_eq!(hook_state::load_state(dir.path(), 770_002), (None, vec![]));
}

#[test]
fn repair_without_state_touches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mem = FakeMemory::new();
    mem.kill();
    assert_eq!(hook_state::repair(dir.path(), &mem, 770_020, 0), 0);
}

#[test]
fn repair_ignores_state_when_module_base_is_unreadable() {
    capture_logs();
    let dir = tempfile::tempdir().unwrap();
    hook_state::save_state(dir.path(), 770_003, 0x1000, &[(0x2000, vec![0xAB])]).unwrap();
    let mem = FakeMemory::new();
    mem.set_bytes(0x2000, &[0]);
    assert_eq!(hook_state::repair(dir.path(), &mem, 770_003, 4), 0);
    assert!(mem.ops().is_empty());
    assert!(!state_file(dir.path(), 770_003).exists());
    let lines = log_lines_with("pid=770003");
    assert!(
        lines.iter().any(|l| l.contains(
            "[reader] cannot read module base, ignoring hook state (slot=4, pid=770003): WizardGraphicalClient.exe module not found."
        )),
        "{lines:?}"
    );
}

#[test]
fn corrupt_state_file_is_logged_and_ignored() {
    capture_logs();
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        (770_010, "{ not json", "malformed hook state (invalid JSON)"),
        (
            770_011,
            r#"{"base": 1, "ops": [{"addr": 2, "bytes": "+f"}]}"#,
            "malformed hook state",
        ),
        (
            770_012,
            r#"{"base": 1, "ops": [{"addr": 2, "bytes": "abc"}]}"#,
            "malformed hook state",
        ),
        (770_013, r#"{"ops": []}"#, "malformed hook state"),
    ];
    for (pid, text, kind) in cases {
        std::fs::write(state_file(dir.path(), pid), text).unwrap();
        assert_eq!(
            hook_state::load_state(dir.path(), pid),
            (None, vec![]),
            "{text}"
        );
        let want = format!(
            "[reader] hook state unreadable, leaked hooks cannot be repaired (pid={pid}, file=hookstate-{pid}.json): {kind}"
        );
        assert_eq!(log_lines_with(&want).len(), 1, "{text}");
    }
}
