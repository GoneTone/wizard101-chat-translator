mod common;

use windows_sys::Win32::System::Memory::{MEM_COMMIT, PAGE_EXECUTE_READ};
use wizard101_chat_translator::reader::memory::{FakeMemory, Region};
use wizard101_chat_translator::reader::pattern::{self, ScanError};

const MODULE: &str = "WizardGraphicalClient.exe";
const REGION_BASE: u64 = 0x1_4000_1000;

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn scan_matches_python_regex_semantics() {
    for case in common::golden("pattern_scan") {
        let pat = case["input"]["pattern"].as_str().unwrap();
        let buf = unhex(case["input"]["buffer"].as_str().unwrap());
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
                matches!(scanned, Err(ScanError::NotFound)),
                "{pat}: {scanned:?}"
            ),
            [(start, _)] => assert_eq!(scanned.unwrap(), REGION_BASE + *start as u64, "{pat}"),
            many => assert!(
                matches!(scanned, Err(ScanError::Multiple(n)) if n == many.len()),
                "{pat}: {scanned:?}"
            ),
        }
    }
}
