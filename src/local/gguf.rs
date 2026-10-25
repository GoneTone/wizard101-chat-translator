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

fn uint_of(r: &mut GGufReader, ty: Ty) -> Option<u32> {
    match ty {
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

/// 讀整數值；逐層陣列取最大值（估算記憶體取最壞情況）。
fn uint(kv: &GGufMetaKV) -> Option<u32> {
    let mut r = kv.value_reader();
    if kv.ty() != Ty::Array {
        return uint_of(&mut r, kv.ty());
    }
    let (ty, len) = r.read_arr_header().ok()?;
    let mut max = 0;
    for _ in 0..len {
        max = max.max(uint_of(&mut r, ty)?);
    }
    Some(max)
}

/// 解析 GGUF 檔頭（metadata 與 tensor 清單）；位元組來自網路，任何輸入都不會 panic。
pub fn parse(bytes: &[u8]) -> Result<ModelMeta, ParseError> {
    // ggus 對惡意長度（如陣列長度 u64::MAX）會在 Layout 計算 panic
    std::panic::catch_unwind(|| parse_inner(bytes)).unwrap_or_else(|_| {
        log("[local] GGUF parser panicked on malformed header");
        Err(ParseError::Invalid("malformed GGUF header".into()))
    })
}

fn parse_inner(bytes: &[u8]) -> Result<ModelMeta, ParseError> {
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
    fetch_with(http, url, INITIAL_RANGE, MAX_RANGE).await
}

/// 內部用：可指定起始與上限範圍，讓測試不必配置 64 MiB。
#[doc(hidden)]
pub async fn fetch_with(
    http: &reqwest::Client,
    url: &str,
    initial: usize,
    max: usize,
) -> Result<ModelMeta, String> {
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    let mut n = initial;
    loop {
        let bytes = match fetch_prefix(http, url, n).await {
            Ok(b) => b,
            Err(e) => {
                log(format!("[local] GGUF header from {host} failed: {e}"));
                return Err(e);
            }
        };
        let err = match parse(&bytes) {
            Ok(meta) => return Ok(meta),
            Err(ParseError::Invalid(e)) => e,
            Err(ParseError::NeedMore) if bytes.len() < n => {
                format!("GGUF file ended inside its header ({} bytes)", bytes.len())
            }
            Err(ParseError::NeedMore) if n >= max => {
                format!("GGUF header larger than {} MiB", max >> 20)
            }
            Err(ParseError::NeedMore) => {
                n *= 2;
                log(format!(
                    "[local] GGUF header needs more, retry with {n} bytes"
                ));
                continue;
            }
        };
        log(format!("[local] GGUF header from {host} failed: {err}"));
        return Err(err);
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
