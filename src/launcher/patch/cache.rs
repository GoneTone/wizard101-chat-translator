//! 讀寫遊戲的 `PatchInfo\CRC_<package>.dat` 檢查紀錄。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::crc::ki_crc;
use super::list::FileEntry;
use crate::logs::log;

const RECORD_LEN: usize = 24;

/// 一個檔案的檢查紀錄：上次驗證時的大小、CRC 與修改時間。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheRecord {
    pub name_crc: u32,
    pub size: u32,
    pub crc: u32,
    pub mtime_ms: u64,
}

/// 解析紀錄檔；尾端不足一筆的位元組丟掉。
pub fn parse_records(bytes: &[u8]) -> Vec<CacheRecord> {
    bytes
        .as_chunks::<RECORD_LEN>()
        .0
        .iter()
        .map(|c| {
            let u32_at = |i: usize| u32::from_le_bytes(c[i..i + 4].try_into().unwrap());
            CacheRecord {
                name_crc: u32_at(0),
                size: u32_at(4),
                crc: u32_at(8),
                mtime_ms: u64::from_le_bytes(c[16..24].try_into().unwrap()),
            }
        })
        .collect()
}

/// 編碼成紀錄檔；第 12～15 位元組為保留欄，寫 0。
pub fn encode_records(records: &[CacheRecord]) -> Vec<u8> {
    let mut out = Vec::with_capacity(records.len() * RECORD_LEN);
    for r in records {
        out.extend(r.name_crc.to_le_bytes());
        out.extend(r.size.to_le_bytes());
        out.extend(r.crc.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(r.mtime_ms.to_le_bytes());
    }
    out
}

/// 套件紀錄檔的路徑。
pub fn record_path(root: &Path, package: &str) -> PathBuf {
    root.join("PatchInfo").join(format!("CRC_{package}.dat"))
}

/// 本機檔案的（大小, 修改時間 Unix 毫秒）；讀不到回 `None`。
pub fn file_stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), mtime.as_millis() as u64))
}

/// 紀錄與本機檔案都吻合清單時，可略過 CRC 重算。
pub fn is_fresh(
    record: Option<&CacheRecord>,
    entry: &FileEntry,
    stamp: Option<(u64, u64)>,
) -> bool {
    let (Some(r), Some((size, mtime_ms))) = (record, stamp) else {
        return false;
    };
    r.crc == entry.crc
        && u64::from(r.size) == size
        && size == u64::from(entry.size)
        && r.mtime_ms == mtime_ms
}

/// 為驗證過的檔案產生紀錄。
pub fn record_for(entry: &FileEntry, stamp: (u64, u64)) -> CacheRecord {
    CacheRecord {
        name_crc: ki_crc(entry.src.as_bytes()),
        size: entry.size,
        crc: entry.crc,
        mtime_ms: stamp.1,
    }
}

/// 一個套件的全部檢查紀錄。
#[derive(Default)]
pub struct PackageRecords {
    records: Vec<CacheRecord>,
}

impl PackageRecords {
    /// 讀取套件紀錄；檔案不存在視為空。
    pub fn load(root: &Path, package: &str) -> io::Result<Self> {
        match fs::read(record_path(root, package)) {
            Ok(bytes) => Ok(Self {
                records: parse_records(&bytes),
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// 以檔名 CRC 查該檔的紀錄。
    pub fn get(&self, entry: &FileEntry) -> Option<&CacheRecord> {
        let key = ki_crc(entry.src.as_bytes());
        self.records.iter().find(|r| r.name_crc == key)
    }

    /// 同名紀錄就地取代，否則附加在最後。
    pub fn set(&mut self, record: CacheRecord) {
        match self
            .records
            .iter_mut()
            .find(|r| r.name_crc == record.name_crc)
        {
            Some(slot) => *slot = record,
            None => self.records.push(record),
        }
    }

    /// 先寫暫存檔再改名，避免中斷留下半個紀錄檔。
    pub fn save(&self, root: &Path, package: &str) -> io::Result<()> {
        let path = record_path(root, package);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("dat.w101tmp");
        fs::write(&tmp, encode_records(&self.records)).inspect_err(|e| {
            log(format!(
                "[patch] failed to write check records: path={} error={e}",
                tmp.display()
            ));
        })?;
        fs::rename(&tmp, &path).inspect_err(|e| {
            log(format!(
                "[patch] failed to save check records: package={package} error={e}"
            ));
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(src: &str, size: u32, crc: u32) -> FileEntry {
        FileEntry {
            package: "Bin".into(),
            src: src.into(),
            tar: String::new(),
            file_type: 0,
            size,
            crc,
        }
    }

    #[test]
    fn records_round_trip() {
        let recs = vec![
            CacheRecord {
                name_crc: 1,
                size: 2,
                crc: 3,
                mtime_ms: 4,
            },
            CacheRecord {
                name_crc: u32::MAX,
                size: 0,
                crc: 9,
                mtime_ms: u64::MAX,
            },
        ];
        let mut bytes = encode_records(&recs);
        assert_eq!(bytes.len(), 48);
        bytes.extend([7, 7, 7]);
        assert_eq!(parse_records(&bytes), recs);
    }

    #[test]
    fn fresh_when_size_mtime_and_crc_match() {
        let e = entry("Bin/a.dll", 10, 0xAB);
        let rec = record_for(&e, (10, 5));
        assert!(is_fresh(Some(&rec), &e, Some((10, 5))));
        assert!(!is_fresh(Some(&rec), &e, Some((10, 6))));
        assert!(!is_fresh(Some(&rec), &e, Some((11, 5))));
        assert!(!is_fresh(None, &e, Some((10, 5))));
    }

    #[test]
    fn stale_record_crc_forces_recompute() {
        let e = entry("Bin/a.dll", 10, 0xAB);
        let rec = record_for(&entry("Bin/a.dll", 10, 0xCD), (10, 5));
        assert!(!is_fresh(Some(&rec), &e, Some((10, 5))));
    }

    #[test]
    fn missing_file_is_not_fresh() {
        let e = entry("Bin/a.dll", 10, 0xAB);
        let rec = record_for(&e, (10, 5));
        assert!(!is_fresh(Some(&rec), &e, None));
        assert_eq!(file_stamp(Path::new("Z:/definitely/missing.bin")), None);
    }

    #[test]
    fn package_records_save_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            PackageRecords::load(dir.path(), "Bin")
                .unwrap()
                .get(&entry("Bin/a", 1, 1))
                .is_none()
        );
        let (a, b) = (entry("Bin/a", 1, 1), entry("Bin/b", 2, 2));
        let mut recs = PackageRecords::default();
        recs.set(record_for(&a, (1, 10)));
        recs.set(record_for(&b, (2, 20)));
        recs.set(record_for(&a, (1, 30)));
        recs.save(dir.path(), "Bin").unwrap();
        assert!(record_path(dir.path(), "Bin").is_file());
        let back = PackageRecords::load(dir.path(), "Bin").unwrap();
        assert_eq!(back.get(&a).unwrap().mtime_ms, 30);
        assert_eq!(back.get(&b).unwrap().mtime_ms, 20);
        assert_eq!(back.records[0].name_crc, ki_crc(b"Bin/a"));
    }
}
