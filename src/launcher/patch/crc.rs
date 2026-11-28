//! KI 的 CRC-32：反射表、初值 0、結尾不反轉，等價於 `zlib.crc32(data, 0xFFFFFFFF) ^ 0xFFFFFFFF`。

use std::io::{self, Read};

const INIT: u32 = 0xFFFF_FFFF;

/// 計算一段位元組的 KI CRC；檔名索引也用這個。
pub fn ki_crc(data: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new_with_initial(INIT);
    hasher.update(data);
    hasher.finalize() ^ INIT
}

/// 分塊讀完整個 reader 並算 KI CRC，不把整檔讀進記憶體。
pub fn ki_crc_reader(mut r: impl Read) -> io::Result<u32> {
    let mut hasher = crc32fast::Hasher::new_with_initial(INIT);
    let mut buf = [0u8; 64 * 1024];
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(hasher.finalize() ^ INIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 已知值；`123456789` 的結果以本機 `CRC_Base.dat` 紀錄驗證過。
    #[test]
    fn ki_crc_matches_known_values() {
        assert_eq!(ki_crc(b""), 0);
        assert_eq!(ki_crc(b"123456789"), 0x2DFD_2D88);
    }

    /// 分塊讀取與一次算完結果一致。
    #[test]
    fn reader_matches_slice() {
        let data: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
        assert_eq!(ki_crc_reader(&data[..]).unwrap(), ki_crc(&data));
    }
}
