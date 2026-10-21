use crate::common;

use windows_sys::Win32::System::Memory::{MEM_COMMIT, PAGE_EXECUTE_READ};
use wizard101_chat_translator::game::memory::{FakeMemory, Region};
use wizard101_chat_translator::game::pattern::{self, ScanError};
use wizard101_chat_translator::util;

const MODULE: &str = "WizardGraphicalClient.exe";
const REGION_BASE: u64 = 0x1_4000_1000;

#[test]
fn scan_matches_python_regex_semantics() {
    let cases = common::golden("pattern_scan");
    assert!(!cases.is_empty());
    for case in cases {
        let pat = case["input"]["pattern"].as_str().unwrap();
        let buf = util::unhex(case["input"]["buffer"].as_str().unwrap()).unwrap();
        let want: Vec<(usize, usize)> = case["output"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                (
                    m[0].as_u64().unwrap() as usize,
                    m[1].as_u64().unwrap() as usize,
                )
            })
            .collect();

        let re = pattern::compile(pat).unwrap_or_else(|e| panic!("{pat}: {e}"));
        let got: Vec<(usize, usize)> = re.find_iter(&buf).map(|m| (m.start(), m.end())).collect();
        assert_eq!(got, want, "pattern {pat}");

        let mem = FakeMemory::new();
        mem.add_module(MODULE, 0x1_4000_0000);
        mem.add_region(
            MODULE,
            Region {
                base: REGION_BASE,
                size: buf.len(),
                protect: PAGE_EXECUTE_READ,
                state: MEM_COMMIT,
            },
        );
        mem.set_bytes(REGION_BASE, &buf);
        let scanned = pattern::scan(&mem, MODULE, pat);
        match want.as_slice() {
            [] => assert!(
                matches!(scanned, Err(ScanError::NotFound(_))),
                "{pat}: {scanned:?}"
            ),
            [(start, _)] => assert_eq!(scanned.unwrap(), REGION_BASE + *start as u64, "{pat}"),
            many => assert!(
                matches!(scanned, Err(ScanError::Multiple(n, _)) if n == many.len()),
                "{pat}: {scanned:?}"
            ),
        }
    }
}
