//! 手動驗證下載規則：連官方更新伺服器，下載清單與每種 FileType 各兩個最小的檔並核對 CRC。

use std::io::Read;

use flate2::read::GzDecoder;
use wizard101_chat_translator::launcher::patch::crc::ki_crc;
use wizard101_chat_translator::launcher::patch::list::{FileEntry, parse_file_list};
use wizard101_chat_translator::launcher::patch::server::list_location;
use wizard101_chat_translator::launcher::patch::url::{file_url, is_gzip};
use wizard101_chat_translator::net::download_client;

const HOST: &str = "patch.us.wizard101.com";
const PORT: u16 = 12500;
const PROBED_TYPES: [u32; 4] = [1, 3, 4, 5];
const PER_TYPE: usize = 2;

/// 下載一個檔，回傳（原始位元組, HTTP 狀態, 下載位元組數）；失敗回傳錯誤字串。
async fn fetch(client: &reqwest::Client, url: &str) -> Result<(Vec<u8>, u16, usize), String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("GET failed: {e}"))?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        return Err(format!("HTTP {status}"));
    }
    let raw = resp
        .bytes()
        .await
        .map_err(|e| format!("body failed: {e}"))?;
    Ok((raw.to_vec(), status, raw.len()))
}

fn decode(entry: &FileEntry, raw: Vec<u8>) -> Result<Vec<u8>, String> {
    if !is_gzip(entry) {
        return Ok(raw);
    }
    let mut out = Vec::new();
    GzDecoder::new(raw.as_slice())
        .read_to_end(&mut out)
        .map_err(|e| format!("gunzip failed: {e}"))?;
    Ok(out)
}

#[tokio::test]
#[ignore = "connects to the official patch server; run only with the user's consent"]
async fn probe_download_rules() {
    let loc = list_location(HOST, PORT).await.expect("list location");
    println!("list location: {loc:?}");
    let client = download_client(&loc.url);

    let (list_bytes, status, _) = fetch(&client, &loc.url).await.expect("download list");
    println!(
        "list: url={} status={status} bytes={}",
        loc.url,
        list_bytes.len()
    );
    assert_eq!(list_bytes.len() as u32, loc.size, "list size");
    assert_eq!(ki_crc(&list_bytes), loc.crc, "list crc");
    let entries = parse_file_list(&list_bytes).expect("parse list");

    // 每個檔的結果先收集，最後一次斷言，避免一種型別失敗就蓋掉其他型別的觀察。
    let mut failures = Vec::new();
    for file_type in PROBED_TYPES {
        let mut candidates: Vec<&FileEntry> = entries
            .iter()
            .filter(|e| e.file_type == file_type && e.size > 0)
            .collect();
        candidates.sort_by_key(|e| e.size);
        if candidates.is_empty() {
            println!("type {file_type}: no entries");
            failures.push(format!("type {file_type}: no entries in list"));
        }
        for entry in candidates.into_iter().take(PER_TYPE) {
            let url = file_url(&loc, entry);
            let result = async {
                let (raw, status, downloaded) = fetch(&client, &url).await?;
                let bytes = decode(entry, raw)?;
                let crc_ok = ki_crc(&bytes) == entry.crc;
                let size_ok = bytes.len() as u32 == entry.size;
                println!(
                    "type {file_type}: url={url} status={status} downloaded={downloaded} decoded={} expected={} crc_ok={crc_ok}",
                    bytes.len(),
                    entry.size
                );
                if crc_ok && size_ok {
                    Ok(())
                } else {
                    Err(format!("crc_ok={crc_ok} size_ok={size_ok}"))
                }
            }
            .await;
            if let Err(e) = result {
                println!("type {file_type}: url={url} FAILED {e}");
                failures.push(format!("type {file_type} {url}: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "probe failures: {failures:#?}");
}
