mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::fake_game::place;
use common::log_capture::log_lines_with;
use serde_json::Value;
use wizard101_chat_translator::reader::chat_hook::{self, AddLineHook};
use wizard101_chat_translator::reader::hook_state;
use wizard101_chat_translator::reader::hooks::{
    AUTOBOT_PATTERN, AutobotCave, MODULE, ROOT_WINDOW_PATTERN, RootWindowHook,
};
use wizard101_chat_translator::reader::memory::{FakeMemory, MemOp};

const BASE: u64 = 0x1_4000_0000;
const RING_SIZE: usize = 0x80040;

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn hex_of(v: &Value) -> Vec<u8> {
    unhex(v.as_str().unwrap())
}

struct Game {
    mem: FakeMemory,
    cave: u64,
    root_jump: u64,
    addline_jump: u64,
    cave_bytes: Vec<u8>,
    root_bytes: Vec<u8>,
    addline_bytes: Vec<u8>,
}

fn game(cave: u64, root_jump: u64, addline_jump: u64) -> Game {
    let mem = FakeMemory::new();
    mem.add_module(MODULE, BASE);
    let cave_bytes = place(&mem, cave, AUTOBOT_PATTERN, 0x1100);
    let root_bytes = place(&mem, root_jump, ROOT_WINDOW_PATTERN, 0x40);
    let addline_bytes = place(&mem, addline_jump, chat_hook::PATTERN, 0x80);
    Game {
        mem,
        cave,
        root_jump,
        addline_jump,
        cave_bytes,
        root_bytes,
        addline_bytes,
    }
}

fn default_game() -> Game {
    game(0x1_4000_1000, 0x1_4080_0000, 0x1_4120_0000)
}

fn n(v: &Value) -> u64 {
    v.as_u64().unwrap()
}

/// 依 golden 案例的位址掛上 cave、root window 與 AddLine，回傳記下的寫入／配置。
fn install_case(case: &Value) -> Vec<MemOp> {
    let input = &case["input"];
    let g = game(
        n(&input["cave"]),
        n(&input["root_jump"]),
        n(&input["addline_jump"]),
    );
    let mut cave = AutobotCave::install(&g.mem).unwrap();
    g.mem.set_next_alloc(n(&input["export"]));
    let root = RootWindowHook::install(&g.mem, &mut cave).unwrap();
    assert_eq!(root.export_addr(), n(&input["export"]));
    g.mem.set_next_alloc(n(&input["ring"]));
    let addline = AddLineHook::install(&g.mem, &mut cave).unwrap();
    assert_eq!(addline.ring_addr(), n(&input["ring"]));
    g.mem.ops()
}

#[test]
fn addline_stub_bytes_match_python() {
    let cases = common::golden("hook_stubs");
    assert!(!cases.is_empty());
    for case in &cases {
        let input = &case["input"];
        let out = &case["output"];
        let stub = chat_hook::stub_bytes(n(&input["ring"]));
        assert_eq!(stub, hex_of(&out["stub_bytes"]));
        assert!(stub.len() + 5 <= 256);

        let addline = &out["addline"];
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
fn root_window_stub_and_jump_match_python() {
    for case in &common::golden("hook_stubs") {
        let input = &case["input"];
        let root = &case["output"]["root"];
        let ops = install_case(case);
        assert_eq!(
            ops[..4],
            [
                MemOp::Write {
                    addr: n(&input["cave"]),
                    bytes: vec![0; 4100]
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
fn cave_saves_208_and_zeroes_4100() {
    let g = default_game();
    let mut cave = AutobotCave::install(&g.mem).unwrap();
    assert_eq!(
        g.mem.ops(),
        [MemOp::Write {
            addr: g.cave,
            bytes: vec![0; 4100]
        }]
    );
    assert_eq!(cave.restore_ops(), [(g.cave, g.cave_bytes[..208].to_vec())]);
    assert_eq!(cave.alloc_stub(50).unwrap(), g.cave);
    assert_eq!(cave.alloc_stub(256).unwrap(), g.cave + 50);
}

struct Installed {
    g: Game,
    cave: AutobotCave,
    export: u64,
    ring: u64,
}

fn install_all() -> Installed {
    let g = default_game();
    let mut cave = AutobotCave::install(&g.mem).unwrap();
    let export = RootWindowHook::install(&g.mem, &mut cave)
        .unwrap()
        .export_addr();
    let ring = AddLineHook::install(&g.mem, &mut cave).unwrap().ring_addr();
    Installed {
        g,
        cave,
        export,
        ring,
    }
}

#[test]
fn uninstall_restores_only_208_when_different() {
    let Installed {
        g,
        cave,
        export,
        ring: _,
    } = install_all();
    assert_eq!(
        cave.restore_ops(),
        [
            (g.root_jump, g.root_bytes[..7].to_vec()),
            (g.addline_jump, g.addline_bytes[..5].to_vec()),
            (g.cave, g.cave_bytes[..208].to_vec()),
        ]
    );
    let before = g.mem.ops().len();
    let mut waited = None;
    // 比對內容在等待之前就讀好（同 wizwalker）：等待中被改回也照樣寫回
    cave.uninstall_with(&g.mem, |d| {
        waited = Some(d);
        g.mem.set_bytes(g.cave, &g.cave_bytes[..208]);
    })
    .unwrap();
    assert_eq!(waited, Some(Duration::from_millis(500)));
    assert_eq!(
        g.mem.ops()[before..],
        [
            MemOp::Write {
                addr: g.root_jump,
                bytes: g.root_bytes[..7].to_vec()
            },
            MemOp::Free(export),
            MemOp::Write {
                addr: g.addline_jump,
                bytes: g.addline_bytes[..5].to_vec()
            },
            MemOp::Write {
                addr: g.cave,
                bytes: g.cave_bytes[..208].to_vec()
            },
        ]
    );

    let g = default_game();
    let cave = AutobotCave::install(&g.mem).unwrap();
    g.mem.set_bytes(g.cave, &g.cave_bytes[..208]);
    let before = g.mem.ops().len();
    cave.uninstall_with(&g.mem, |_| {}).unwrap();
    assert_eq!(g.mem.ops().len(), before);
}

#[test]
fn ring_is_never_freed() {
    let Installed {
        g,
        cave,
        export,
        ring,
    } = install_all();
    cave.uninstall_with(&g.mem, |_| {}).unwrap();
    let ops = g.mem.ops();
    assert!(ops.contains(&MemOp::Free(export)));
    assert!(!ops.contains(&MemOp::Free(ring)));

    let g = default_game();
    let mut cave = AutobotCave::install(&g.mem).unwrap();
    AddLineHook::install(&g.mem, &mut cave).unwrap();
    let before = g.mem.ops().len();
    cave.uninstall_with(&g.mem, |_| {}).unwrap();
    assert_eq!(
        g.mem.ops()[before..],
        [
            MemOp::Write {
                addr: g.addline_jump,
                bytes: g.addline_bytes[..5].to_vec()
            },
            MemOp::Write {
                addr: g.cave,
                bytes: g.cave_bytes[..208].to_vec()
            },
        ]
    );
}

#[test]
fn uninstall_on_a_closed_game_writes_nothing() {
    let Installed { g, cave, .. } = install_all();
    let before = g.mem.ops().len();
    g.mem.kill();
    cave.uninstall_with(&g.mem, |_| panic!("must not wait"))
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
fn hookstate_bytes_match_python() {
    let cases = common::golden("hookstate_bytes");
    assert!(!cases.is_empty());
    let dir = tempfile::tempdir().unwrap();
    for case in &cases {
        let input = &case["input"];
        let pid = pid_of(input);
        hook_state::save_state(dir.path(), pid, n(&input["base"]), &ops_of(input)).unwrap();
        let written = std::fs::read(dir.path().join(format!("hookstate-{pid}.json"))).unwrap();
        assert_eq!(written, case["output"].as_str().unwrap().as_bytes());
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
    assert_eq!(ops.len(), 3);
    assert_eq!(ops[2].1.len(), 208);
    let mem = FakeMemory::new();
    mem.add_module(MODULE, base);
    for (addr, original) in &ops {
        mem.set_bytes(*addr, &vec![0xE9; original.len()]);
    }
    assert_eq!(hook_state::repair(dir.path(), &mem, pid, 0), 3);
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
    log_lines_with("");
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
            "[reader] hook repair write failed at 0x3000 (slot=2, len=1): MemoryWriteError: \
             Unable to write memory at address 12288."
        )
        .len(),
        1
    );
    assert_eq!(hook_state::load_state(dir.path(), 770_001), (None, vec![]));
}

#[test]
fn repair_skips_and_logs_a_stale_state_file() {
    log_lines_with("");
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
    log_lines_with("");
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
            "[reader] cannot read module base, ignoring hook state (slot=4, pid=770003): ValueError: WizardGraphicalClient.exe module not found."
        )),
        "{lines:?}"
    );
}

#[test]
fn corrupt_state_file_is_logged_and_ignored() {
    log_lines_with("");
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        (770_010, "{ not json", "JSONDecodeError: "),
        (
            770_011,
            r#"{"base": 1, "ops": [{"addr": 2, "bytes": "+f"}]}"#,
            "ValueError: ",
        ),
        (
            770_012,
            r#"{"base": 1, "ops": [{"addr": 2, "bytes": "abc"}]}"#,
            "ValueError: ",
        ),
        (770_013, r#"{"ops": []}"#, "ValueError: "),
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
