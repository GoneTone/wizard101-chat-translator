//! 收訊端解析（聊天標記、分頁、UI 矩形、環形緩衝記錄、UI 樹）對照 Python 行為。

mod common;

use std::sync::Mutex;

use common::fake_game::{put_button, put_children, put_name};
use serde_json::Value;
use wizard101_chat_translator::reader::chat_hook::{
    DATA_OFF, HEADER, HookRecord, MAX_CHARS, SLOT_SIZE, SLOTS, decode_record, group_broadcasts,
    pending_broadcast, read_ring,
};
use wizard101_chat_translator::reader::chat_tabs::{
    CUR_TAB_OFFSET, TabError, find_tab_buttons, read_current_tab, read_tab_state, visible_label,
};
use wizard101_chat_translator::reader::markup::parse_line;
use wizard101_chat_translator::reader::memory::{FakeMemory, MemError, ProcessMemory, Region};
use wizard101_chat_translator::reader::ui_rect::client_rect;
use wizard101_chat_translator::reader::ui_tree::{Window, find_by_name};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn pair(v: &Value) -> (i32, i32) {
    let a = v.as_array().unwrap();
    (a[0].as_i64().unwrap() as i32, a[1].as_i64().unwrap() as i32)
}

#[test]
fn parse_line_matches_python() {
    let cases = common::golden("markup_parse");
    assert!(cases.len() >= 200, "only {} cases", cases.len());
    for c in cases {
        let raw = c["input"].as_str().unwrap();
        let got = parse_line(raw).map(|l| serde_json::json!([l.text, l.color, l.own, l.system]));
        let want = &c["output"];
        assert_eq!(got.as_ref().unwrap_or(&Value::Null), want, "{raw:?}");
    }
}

#[test]
fn group_broadcasts_matches_python() {
    for c in common::golden("broadcast_groups") {
        let records: Vec<HookRecord> = c["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                let text = r[2].as_str().unwrap().to_owned();
                let n = text.encode_utf16().count() as u32;
                HookRecord {
                    seq: r[0].as_u64().unwrap(),
                    ret: 1,
                    this: 2,
                    tab: r[1].as_u64().unwrap() as u32,
                    copied: n,
                    length: n,
                    text,
                }
            })
            .collect();
        let groups: Vec<Value> = group_broadcasts(&records)
            .into_iter()
            .map(|(tabs, rec)| serde_json::json!([tabs, rec.seq]))
            .collect();
        assert_eq!(
            Value::Array(groups),
            c["output"]["groups"],
            "{}",
            c["input"]
        );
        assert_eq!(
            pending_broadcast(&records) as u64,
            c["output"]["pending"].as_u64().unwrap(),
            "{}",
            c["input"]
        );
    }
}

#[test]
fn visible_label_matches_python() {
    for c in common::golden("tab_labels") {
        let got = visible_label(c["input"].as_str().unwrap());
        assert_eq!(got.as_deref(), c["output"].as_str(), "{}", c["input"]);
    }
}

#[test]
fn client_rect_matches_python() {
    for c in common::golden("ui_rect") {
        let i = &c["input"];
        let offsets: Vec<(i32, i32)> = i["offsets"].as_array().unwrap().iter().map(pair).collect();
        let got = client_rect(
            &offsets,
            pair(&i["size"]),
            pair(&i["root"]),
            pair(&i["client"]),
        );
        let want = c["output"].as_array().map(|a| {
            (0..4)
                .map(|k| a[k].as_i64().unwrap() as i32)
                .collect::<Vec<_>>()
        });
        assert_eq!(got.map(|(x, y, w, h)| vec![x, y, w, h]), want, "{i}");
    }
}

#[test]
fn ring_record_split_surrogate_matches_python() {
    let cases = common::golden("hook_records");
    assert!(cases.len() >= 15);
    for c in cases {
        let rec = decode_record(&unhex(c["input"].as_str().unwrap()));
        let o = &c["output"];
        assert_eq!(rec.seq, o["seq"].as_u64().unwrap());
        assert_eq!(rec.ret, o["ret"].as_u64().unwrap());
        assert_eq!(rec.this, o["this"].as_u64().unwrap());
        assert_eq!(u64::from(rec.tab), o["tab"].as_u64().unwrap());
        assert_eq!(u64::from(rec.copied), o["copied"].as_u64().unwrap());
        assert_eq!(u64::from(rec.length), o["length"].as_u64().unwrap());
        assert_eq!(rec.text, o["text"].as_str().unwrap(), "{}", c["input"]);
    }
}

const RING: u64 = 0x1000_0000;

fn slot_addr(seq: u64) -> u64 {
    RING + HEADER + ((seq - 1) % SLOTS) * SLOT_SIZE
}

/// 寫一個槽位並把累計數推到至少 `seq`；`slot_seq` 可偽造槽內序號。
fn put(mem: &impl PutBytes, seq: u64, units: &[u16], tab: u32, slot_seq: Option<u64>) {
    let at = slot_addr(seq);
    let mut head = Vec::new();
    head.extend(slot_seq.unwrap_or(seq).to_le_bytes());
    head.extend(0x1407443B1u64.to_le_bytes());
    head.extend(0x7777u64.to_le_bytes());
    head.extend(tab.to_le_bytes());
    head.extend((units.len() as u32).to_le_bytes());
    head.extend((units.len() as u32).to_le_bytes());
    head.resize(DATA_OFF as usize, 0);
    mem.set_bytes(at, &head);
    let data: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    if !data.is_empty() {
        mem.set_bytes(at + DATA_OFF, &data);
    }
    let count = mem.count().max(seq);
    mem.set_bytes(RING, &count.to_le_bytes());
}

trait PutBytes {
    fn set_bytes(&self, addr: u64, bytes: &[u8]);
    fn count(&self) -> u64;
}

impl PutBytes for FakeMemory {
    fn set_bytes(&self, addr: u64, bytes: &[u8]) {
        FakeMemory::set_bytes(self, addr, bytes);
    }

    fn count(&self) -> u64 {
        self.read_u64(RING).unwrap_or(0)
    }
}

type ReadHook = Box<dyn FnMut(u64, usize, &FakeMemory) + Send>;

/// 包住 FakeMemory：記下每次讀取，並可在讀取之後插手改記憶體（模擬遊戲執行緒的覆寫）。
struct SpyMemory {
    inner: FakeMemory,
    reads: Mutex<Vec<(u64, usize)>>,
    after_read: Mutex<Option<ReadHook>>,
}

impl SpyMemory {
    fn new() -> Self {
        Self {
            inner: FakeMemory::new(),
            reads: Mutex::default(),
            after_read: Mutex::default(),
        }
    }
}

impl PutBytes for SpyMemory {
    fn set_bytes(&self, addr: u64, bytes: &[u8]) {
        self.inner.set_bytes(addr, bytes);
    }

    fn count(&self) -> u64 {
        self.inner.read_u64(RING).unwrap_or(0)
    }
}

impl ProcessMemory for SpyMemory {
    fn read(&self, addr: u64, len: usize) -> Result<Vec<u8>, MemError> {
        let out = self.inner.read(addr, len);
        self.reads.lock().unwrap().push((addr, len));
        if let Some(hook) = self.after_read.lock().unwrap().as_mut() {
            hook(addr, len, &self.inner);
        }
        out
    }

    fn write(&self, addr: u64, bytes: &[u8]) -> Result<(), MemError> {
        self.inner.write(addr, bytes)
    }

    fn alloc(&self, size: usize) -> Result<u64, MemError> {
        self.inner.alloc(size)
    }

    fn free(&self, addr: u64) {
        self.inner.free(addr);
    }

    fn module_base(&self, name: &str) -> Result<u64, MemError> {
        self.inner.module_base(name)
    }

    fn regions(&self, module: &str) -> Result<Vec<Region>, MemError> {
        self.inner.regions(module)
    }

    fn is_alive(&self) -> bool {
        self.inner.is_alive()
    }
}

#[test]
fn read_ring_rereads_seq_to_detect_a_torn_read() {
    let mem = SpyMemory::new();
    put(&mem, 1, &units("a"), 0, None);
    put(&mem, 2, &units("torn"), 0, None);
    *mem.after_read.lock().unwrap() = Some(Box::new(|addr, _, inner| {
        if addr == slot_addr(2) + DATA_OFF {
            inner.set_bytes(slot_addr(2), &(2 + SLOTS).to_le_bytes());
        }
    }));
    let got = read_ring(&mem, RING, 0).unwrap();
    let texts: Vec<_> = got.records.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(texts, ["a"]);
    assert_eq!((got.last_seq, got.lost), (2, 1));
}

#[test]
fn read_ring_skips_the_content_read_for_an_empty_line() {
    let mem = SpyMemory::new();
    put(&mem, 1, &[], 0, None);
    let got = read_ring(&mem, RING, 0).unwrap();
    assert_eq!(
        (got.records[0].text.as_str(), got.records[0].length),
        ("", 0)
    );
    let reads = mem.reads.lock().unwrap();
    assert!(
        reads
            .iter()
            .all(|&(addr, len)| len > 0 && addr != slot_addr(1) + DATA_OFF)
    );
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[test]
fn read_ring_returns_new_records_in_order() {
    let mem = FakeMemory::new();
    for (seq, text) in [(1, "a"), (2, "b"), (3, "c")] {
        put(&mem, seq, &units(text), seq as u32, None);
    }
    let got = read_ring(&mem, RING, 0).unwrap();
    let rows: Vec<_> = got
        .records
        .iter()
        .map(|r| (r.seq, r.tab, r.text.as_str()))
        .collect();
    assert_eq!(rows, [(1, 1, "a"), (2, 2, "b"), (3, 3, "c")]);
    assert_eq!((got.last_seq, got.lost), (3, 0));
}

#[test]
fn read_ring_stops_at_a_slot_being_written_or_still_on_the_previous_lap() {
    let mem = FakeMemory::new();
    put(&mem, 1, &units("a"), 0, None);
    put(&mem, 2, &units("b"), 0, Some(0));
    put(&mem, 3, &units("c"), 0, None);
    let got = read_ring(&mem, RING, 0).unwrap();
    assert_eq!(got.records.len(), 1);
    assert_eq!((got.last_seq, got.lost), (1, 0));

    let lap = FakeMemory::new();
    put(&lap, SLOTS + 1, &units("old lap"), 0, Some(1));
    let got = read_ring(&lap, RING, SLOTS).unwrap();
    assert_eq!((got.records.len(), got.last_seq, got.lost), (0, SLOTS, 0));
    put(&lap, SLOTS + 1, &units("new lap"), 0, None);
    let got = read_ring(&lap, RING, SLOTS).unwrap();
    assert_eq!(got.records[0].text, "new lap");
}

#[test]
fn read_ring_counts_an_overwritten_slot_as_lost() {
    let mem = FakeMemory::new();
    put(&mem, 1, &units("a"), 0, None);
    put(&mem, 2, &units("b"), 0, Some(2 + SLOTS));
    put(&mem, 3, &units("c"), 0, None);
    let got = read_ring(&mem, RING, 0).unwrap();
    let texts: Vec<_> = got.records.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(texts, ["a", "c"]);
    assert_eq!((got.last_seq, got.lost), (3, 1));
}

#[test]
fn read_ring_overflow_reports_lost_and_resumes_at_oldest_valid_slot() {
    let mem = FakeMemory::new();
    let (last_seq, count) = (10, 10 + SLOTS + 5);
    for seq in last_seq + 6..=count {
        put(&mem, seq, &units(&seq.to_string()), 0, None);
    }
    let got = read_ring(&mem, RING, last_seq).unwrap();
    assert_eq!(got.lost, 5);
    assert_eq!(got.records.len() as u64, SLOTS);
    assert_eq!(got.records[0].seq, last_seq + 6);
    assert_eq!(got.last_seq, count);
}

#[test]
fn read_ring_empty_line_and_split_surrogate_at_the_cut() {
    let mem = FakeMemory::new();
    put(&mem, 1, &[], 0, None);
    let mut cut = vec![u16::from(b'a'); MAX_CHARS as usize - 1];
    cut.push(0xD83D);
    put(&mem, 2, &cut, 0, None);
    let got = read_ring(&mem, RING, 0).unwrap();
    assert_eq!(got.records[0].text, "");
    assert_eq!(got.records[1].text, format!("{}\u{FFFD}", "a".repeat(999)));
}

#[test]
fn ui_tree_find_by_name_skips_unreadable_nodes() {
    let mem = FakeMemory::new();
    put_children(&mem, 0x1000, &[0x2000, 0x3000, 0x5000]);
    put_name(&mem, 0x2000, "Target");
    put_name(&mem, 0x3000, "Target");
    put_name(&mem, 0x5000, "Other");
    // 0x2000 的子節點向量讀不到：Window.children 回空，走訪繼續
    // 0x3000 同樣讀不到子節點
    put_children(&mem, 0x5000, &[0x6000, 0x7000, 0x8000]);
    put_name(&mem, 0x6000, "Target");
    put_children(&mem, 0x6000, &[]);
    // 0x7000 名稱讀不到：中止 0x5000 剩下的子節點（0x8000 不會被看到）
    put_name(&mem, 0x8000, "Target");
    let found = find_by_name(&mem, Window { addr: 0x1000 }, "Target").unwrap();
    let addrs: Vec<u64> = found.iter().map(|w| w.addr).collect();
    assert_eq!(addrs, [0x2000, 0x3000, 0x6000]);
}

#[test]
fn ui_tree_find_by_name_propagates_unreadable_direct_child_name() {
    let mem = FakeMemory::new();
    put_children(&mem, 0x1000, &[0x2000, 0x3000]);
    put_name(&mem, 0x3000, "Target");
    let err = find_by_name(&mem, Window { addr: 0x1000 }, "Target").unwrap_err();
    assert!(matches!(err, MemError::Read(_)), "{err:?}");
}

#[test]
fn std_string_heap_when_length_ge_16() {
    let mem = FakeMemory::new();
    let text = "WizardChatBoxWindow";
    mem.set_bytes(0x9000, text.as_bytes());
    let mut head = [0u8; 0x20];
    head[..8].copy_from_slice(&0x9000u64.to_le_bytes());
    head[16..20].copy_from_slice(&(text.len() as i32).to_le_bytes());
    mem.set_bytes(0x2000 + 0x50, &head);
    assert_eq!(Window { addr: 0x2000 }.name(&mem).unwrap(), text);
}

#[test]
fn read_tab_state_lists_visible_tabs_in_button_order() {
    let mem = FakeMemory::new();
    let (root, chatbox) = (0x10_000, 0x11_000);
    mem.set_bytes(chatbox + CUR_TAB_OFFSET, &1i32.to_le_bytes());
    put_children(&mem, root, &[0x20_000, 0x30_000, 0x40_000, 0x50_000]);
    put_button(&mem, 0x20_000, "GroupFilterButton", "Party", 1, 1);
    put_button(&mem, 0x30_000, "MainFilterButton", "Main", 1, 1);
    put_button(&mem, 0x40_000, "HouseFilterButton", "<string;GUI2_1>", 1, 1);
    put_button(&mem, 0x50_000, "FriendFilterButton", "Friends", 1, 0);
    let buttons = find_tab_buttons(&mem, Window { addr: root }).unwrap();
    let ids: Vec<i32> = buttons.iter().map(|b| b.0).collect();
    assert_eq!(ids, [0, 1, 2, 3]);
    let state = read_tab_state(&mem, chatbox, &buttons).unwrap();
    assert_eq!(state.current, 1);
    assert_eq!(
        state.tabs,
        [(0, "Main".to_owned()), (1, "Party".to_owned())]
    );
    assert_eq!(read_current_tab(&mem, chatbox), Some(1));

    mem.set_bytes(chatbox + CUR_TAB_OFFSET, &42i32.to_le_bytes());
    let err = read_tab_state(&mem, chatbox, &buttons).unwrap_err();
    assert_eq!(
        err,
        TabError::OutOfRange {
            current: 42,
            chatbox
        }
    );
    assert_eq!(
        err.to_string(),
        "current tab 42 out of range (stale chat box 0x11000?)"
    );
    assert_eq!(err.type_name(), "ValueError");
    assert_eq!(read_current_tab(&mem, chatbox), None);
}
