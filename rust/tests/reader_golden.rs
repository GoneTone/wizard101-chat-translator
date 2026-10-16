//! 收訊端解析（聊天標記、分頁、UI 矩形、環形緩衝記錄、UI 樹）對照 Python 行為。

mod common;

use serde_json::Value;
use wizard101_chat_translator::reader::chat_hook::{
    DATA_OFF, HEADER, HookRecord, MAX_CHARS, SLOT_SIZE, SLOTS, decode_record, group_broadcasts,
    pending_broadcast, read_ring,
};
use wizard101_chat_translator::reader::chat_tabs::{
    CUR_TAB_OFFSET, LABEL_OFFSET, read_current_tab, read_tab_state, visible_label,
};
use wizard101_chat_translator::reader::markup::parse_line;
use wizard101_chat_translator::reader::memory::{FakeMemory, ProcessMemory};
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
                    tab: r[1].as_i64().unwrap() as i32,
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
        assert_eq!(rec.tab as u64, o["tab"].as_u64().unwrap());
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
fn put(mem: &FakeMemory, seq: u64, units: &[u16], tab: u32, slot_seq: Option<u64>) {
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
    let count = mem.read_u64(RING).unwrap_or(0).max(seq);
    mem.set_bytes(RING, &count.to_le_bytes());
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

fn put_name(mem: &FakeMemory, win: u64, name: &str) {
    let mut head = [0u8; 0x20];
    if name.len() >= 16 {
        let heap = win + 0x300_000;
        mem.set_bytes(heap, name.as_bytes());
        head[..8].copy_from_slice(&heap.to_le_bytes());
    } else {
        head[..name.len()].copy_from_slice(name.as_bytes());
    }
    head[16..20].copy_from_slice(&(name.len() as i32).to_le_bytes());
    mem.set_bytes(win + 0x50, &head);
}

fn put_children(mem: &FakeMemory, win: u64, kids: &[u64]) {
    let vec_at = win + 0x100_000;
    let raw: Vec<u8> = kids
        .iter()
        .flat_map(|k| [k.to_le_bytes(), [0; 8]].concat())
        .collect();
    if !raw.is_empty() {
        mem.set_bytes(vec_at, &raw);
    }
    let header = [
        vec_at.to_le_bytes(),
        (vec_at + raw.len() as u64).to_le_bytes(),
    ]
    .concat();
    mem.set_bytes(win + 0x70, &header);
}

#[test]
fn ui_tree_find_by_name_skips_unreadable_nodes() {
    let mem = FakeMemory::new();
    put_children(&mem, 0x1000, &[0x2000, 0x3000, 0x4000, 0x5000]);
    put_name(&mem, 0x2000, "Target");
    put_children(&mem, 0x2000, &[]);
    // 0x3000 的名稱讀得到、子節點向量讀不到
    put_name(&mem, 0x3000, "Target");
    // 0x4000 完全讀不到
    put_name(&mem, 0x5000, "Other");
    put_children(&mem, 0x5000, &[0x6000]);
    put_name(&mem, 0x6000, "Target");
    put_children(&mem, 0x6000, &[]);
    let found = find_by_name(&mem, Window { addr: 0x1000 }, "Target");
    let addrs: Vec<u64> = found.iter().map(|w| w.addr).collect();
    assert_eq!(addrs, [0x2000, 0x3000, 0x6000]);
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

fn put_button(mem: &FakeMemory, win: u64, name: &str, label: &str, flags: u32, parent_flags: u32) {
    put_name(mem, win, name);
    put_children(mem, win, &[]);
    let parent = win + 0x200_000;
    mem.set_bytes(win + 0x88, &parent.to_le_bytes());
    mem.set_bytes(win + 0x9C, &flags.to_le_bytes());
    mem.set_bytes(parent + 0x9C, &parent_flags.to_le_bytes());
    let data: Vec<u8> = label.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut head = vec![0u8; 0x20];
    head[..data.len()].copy_from_slice(&data);
    head[0x10..0x18].copy_from_slice(&(label.encode_utf16().count() as u64).to_le_bytes());
    head[0x18..].copy_from_slice(&7u64.to_le_bytes());
    mem.set_bytes(win + LABEL_OFFSET, &head);
}

#[test]
fn read_tab_state_lists_visible_tabs_in_button_order() {
    let mem = FakeMemory::new();
    let chatbox = 0x10_000;
    mem.set_bytes(chatbox + CUR_TAB_OFFSET, &1i32.to_le_bytes());
    put_children(&mem, chatbox, &[0x20_000, 0x30_000, 0x40_000, 0x50_000]);
    put_button(&mem, 0x20_000, "GroupFilterButton", "Party", 1, 1);
    put_button(&mem, 0x30_000, "MainFilterButton", "Main", 1, 1);
    put_button(&mem, 0x40_000, "HouseFilterButton", "<string;GUI2_1>", 1, 1);
    put_button(&mem, 0x50_000, "FriendFilterButton", "Friends", 1, 0);
    let state = read_tab_state(&mem, chatbox).unwrap();
    assert_eq!(state.current, 1);
    assert_eq!(
        state.tabs,
        [(0, "Main".to_owned()), (1, "Party".to_owned())]
    );
    assert_eq!(read_current_tab(&mem, chatbox), Some(1));
    mem.set_bytes(chatbox + CUR_TAB_OFFSET, &42i32.to_le_bytes());
    assert_eq!(read_tab_state(&mem, chatbox), None);
}
