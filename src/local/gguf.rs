//! 以 HTTP Range 讀取遠端 GGUF 檔頭，取出估算記憶體所需的模型形狀。

use std::collections::HashMap;

use ggus::{GGufMetaDataValueType as Ty, GGufMetaKV, GGufReadError, GGufReader};

use crate::logs::log;
use crate::net::error_chain;

const INITIAL_RANGE: usize = 4 << 20;
const MAX_RANGE: usize = 64 << 20;

/// 估算記憶體需要的模型形狀；缺少的欄位已套用預設值。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ModelMeta {
    pub arch: String,
    pub block_count: u32,
    pub head_count: u32,
    pub head_count_kv: u32,
    pub key_length: u32,
    pub value_length: u32,
    pub embedding_length: u32,
    pub expert_count: u32,
    pub has_mtp: bool,
}

/// 解析失敗原因：`NeedMore` 表示位元組不足以讀完檔頭，`Invalid` 表示內容不是可用的 GGUF。
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    NeedMore,
    Invalid(String),
}

fn read_err(e: GGufReadError) -> ParseError {
    match e {
        GGufReadError::Eos => ParseError::NeedMore,
        other => ParseError::Invalid(format!("{other:?}")),
    }
}

fn uint(kv: &GGufMetaKV) -> Option<u32> {
    let mut r = kv.value_reader();
    match kv.ty() {
        Ty::U8 => Some(r.read::<u8>().ok()?.into()),
        Ty::U16 => Some(r.read::<u16>().ok()?.into()),
        Ty::U32 => r.read::<u32>().ok(),
        Ty::U64 => u32::try_from(r.read::<u64>().ok()?).ok(),
        Ty::I8 => u32::try_from(r.read::<i8>().ok()?).ok(),
        Ty::I16 => u32::try_from(r.read::<i16>().ok()?).ok(),
        Ty::I32 => u32::try_from(r.read::<i32>().ok()?).ok(),
        Ty::I64 => u32::try_from(r.read::<i64>().ok()?).ok(),
        _ => None,
    }
}

/// 解析 GGUF 檔頭（metadata 與 tensor 清單）。
pub fn parse(bytes: &[u8]) -> Result<ModelMeta, ParseError> {
    if bytes.len() >= 4 && &bytes[..4] != b"GGUF" {
        return Err(ParseError::Invalid("not a GGUF file".into()));
    }
    let mut reader = GGufReader::new(bytes);
    let header = reader.read_header().map_err(read_err)?;
    let mut kvs = HashMap::new();
    for _ in 0..header.metadata_kv_count {
        let kv = reader.read_meta_kv().map_err(read_err)?;
        kvs.insert(kv.key(), kv);
    }
    let mut tensors = Vec::new();
    for _ in 0..header.tensor_count {
        tensors.push(reader.read_tensor_meta().map_err(read_err)?.name());
    }

    let arch = kvs
        .get("general.architecture")
        .filter(|kv| kv.ty() == Ty::String)
        .map(|kv| kv.value_reader().read_str().map_err(read_err))
        .transpose()?
        .ok_or_else(|| ParseError::Invalid("missing general.architecture".into()))?;
    let num = |name: &str| {
        kvs.get(format!("{arch}.{name}").as_str())
            .and_then(uint)
            .unwrap_or(0)
    };

    let block_count = num("block_count");
    let head_count = num("attention.head_count");
    let embedding_length = num("embedding_length");
    let per_head = embedding_length.checked_div(head_count).unwrap_or(0);
    let or = |v: u32, fallback: u32| if v == 0 { fallback } else { v };
    let mtp_tensor = format!("blk.{}.nextn.eh_proj.weight", block_count.wrapping_sub(1));
    Ok(ModelMeta {
        arch: arch.to_string(),
        block_count,
        head_count,
        head_count_kv: or(num("attention.head_count_kv"), head_count),
        key_length: or(num("attention.key_length"), per_head),
        value_length: or(num("attention.value_length"), per_head),
        embedding_length,
        expert_count: num("expert_count"),
        has_mtp: block_count > 0 && tensors.contains(&mtp_tensor.as_str()),
    })
}

/// 讀取 `url` 開頭的檔頭並解析；範圍不夠時加倍重抓，上限 64 MiB。
pub async fn fetch(http: &reqwest::Client, url: &str) -> Result<ModelMeta, String> {
    let mut n = INITIAL_RANGE;
    loop {
        let bytes = fetch_prefix(http, url, n).await?;
        match parse(&bytes) {
            Ok(meta) => return Ok(meta),
            Err(ParseError::Invalid(e)) => {
                log(format!("[local] GGUF header invalid: {e}"));
                return Err(e);
            }
            Err(ParseError::NeedMore) if bytes.len() < n => {
                return Err("GGUF file ended inside its header".into());
            }
            Err(ParseError::NeedMore) if n >= MAX_RANGE => {
                return Err(format!("GGUF header larger than {} MiB", MAX_RANGE >> 20));
            }
            Err(ParseError::NeedMore) => {
                n *= 2;
                log(format!(
                    "[local] GGUF header needs more, retry with {n} bytes"
                ));
            }
        }
    }
}

/// 取檔案前 `n` 位元組；伺服器忽略 Range 回整檔時讀到 `n` 就停。
async fn fetch_prefix(http: &reqwest::Client, url: &str, n: usize) -> Result<Vec<u8>, String> {
    let mut resp = http
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes=0-{}", n - 1))
        .send()
        .await
        .map_err(error_chain)?;
    if !resp.status().is_success() {
        log(format!(
            "[local] GGUF range request failed: HTTP {}",
            resp.status()
        ));
        return Err(format!("HTTP {}", resp.status()));
    }
    let mut buf = Vec::with_capacity(n);
    while buf.len() < n {
        match resp.chunk().await.map_err(error_chain)? {
            Some(c) => buf.extend_from_slice(&c),
            None => break,
        }
    }
    buf.truncate(n);
    Ok(buf)
}
