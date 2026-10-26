//! Hugging Face repo 解析、檔案列表與 GGUF 檔案分類。

use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde::Deserialize;

use super::download::body_preview;
use crate::logs::log;
use crate::net::{bypass_proxy_for_loopback, error_chain, install_crypto_provider};

/// Hugging Face 正式站台根網址。
pub const HF_BASE: &str = "https://huggingface.co";

const TIMEOUT: Duration = Duration::from_secs(60);

static SHARD_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)-(\d{5})-of-(\d{5})$").expect("shard regex"));
static QUANT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[-_.])((?:UD-)?(?:I?Q\d(?:_[A-Z0-9]+)*|MXFP4(?:_MOE)?|BF16|F16|F32))$")
        .expect("quant regex")
});

/// repo 內的一個檔案；`sha256` 只有 LFS 檔才有。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoFile {
    pub path: String,
    pub size: u64,
    pub sha256: Option<String>,
}

/// 量化名稱與位元數（認不出時 `bits` 為 `None`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quant {
    pub label: String,
    pub bits: Option<u8>,
}

/// 一個可下載的主模型量化版本；分檔依序。
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub quant: Quant,
    pub files: Vec<RepoFile>,
}

/// 一個 MTP（投機解碼）模型檔；`shared` 表示與主模型共用 embedding。
#[derive(Debug, Clone, PartialEq)]
pub struct MtpChoice {
    pub quant: Quant,
    pub file: RepoFile,
    pub shared: bool,
}

/// repo 內可選的主模型與 MTP 檔案。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalog {
    pub choices: Vec<Choice>,
    pub mtp: Vec<MtpChoice>,
}

/// 建立連 `base`（正式為 `HF_BASE`）用的 HTTP client；`base` 是本機位址時不走 proxy。
pub fn hf_client(base: &str) -> reqwest::Client {
    install_crypto_provider();
    let builder = reqwest::Client::builder()
        .connect_timeout(TIMEOUT)
        .read_timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(format!(
            "wizard101-chat-translator/{}",
            env!("CARGO_PKG_VERSION")
        ));
    bypass_proxy_for_loopback(builder, base)
        .build()
        .expect("build HTTP client")
}

/// 從 `org/repo` 或 Hugging Face 網址取出 `org/repo`；其他輸入回 `None`。
pub fn parse_repo(input: &str) -> Option<String> {
    let input = input.trim();
    let rest = input
        .strip_prefix("https://")
        .or_else(|| input.strip_prefix("http://"))
        .unwrap_or(input);
    let (path, is_url) = match rest.strip_prefix("huggingface.co/") {
        Some(path) => (path, true),
        None => (rest, false),
    };
    let segments: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    let valid = |s: &&str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    };
    let exact = if is_url {
        segments.len() >= 2
    } else {
        segments.len() == 2
    };
    (exact && segments[..2].iter().all(valid)).then(|| segments[..2].join("/"))
}

#[derive(Deserialize)]
struct TreeEntry {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    #[serde(default)]
    size: u64,
    lfs: Option<Lfs>,
}

#[derive(Deserialize)]
struct Lfs {
    oid: String,
}

/// 取 `Link` 標頭中 `rel="next"` 的網址。
fn next_page(response: &reqwest::Response) -> Option<String> {
    next_link(
        response
            .headers()
            .get(reqwest::header::LINK)?
            .to_str()
            .ok()?,
    )
}

/// 解析 `<url>; rel="next", <url>; rel="last"`；網址本身可含逗號與分號，所以以角括號切開。
fn next_link(header: &str) -> Option<String> {
    let mut rest = header;
    while let Some(start) = rest.find('<') {
        let (url, after) = rest[start + 1..].split_once('>')?;
        let params_end = after.find('<').unwrap_or(after.len());
        let is_next = after[..params_end].split([';', ',']).any(|param| {
            let Some((key, value)) = param.split_once('=') else {
                return false;
            };
            key.trim().eq_ignore_ascii_case("rel")
                && value
                    .trim()
                    .trim_matches('"')
                    .split_ascii_whitespace()
                    .any(|rel| rel.eq_ignore_ascii_case("next"))
        });
        if is_next {
            return Some(url.to_owned());
        }
        rest = &after[params_end..];
    }
    None
}

/// HF 對 `repo` 預設的模型檔 sha256（小寫）：取 `/v2/{repo}/manifests/latest` 的 model 層。
///
/// 這是 HF 給 Ollama／llama.cpp 用的同一份預設；分檔的 repo 會回錯誤。查不到一律回 `None`，只記 log。
pub async fn default_model_sha256(
    http: &reqwest::Client,
    base: &str,
    repo: &str,
) -> Option<String> {
    let url = format!("{base}/v2/{repo}/manifests/latest");
    let response = match http
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            log(format!(
                "[local] {repo} default quant lookup failed: {}",
                error_chain(e)
            ));
            return None;
        }
    };
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        log(format!(
            "[local] {repo} has no default quant: HTTP {status}: {}",
            body_preview(&body)
        ));
        return None;
    }
    let manifest: Manifest = match response.json().await {
        Ok(m) => m,
        Err(e) => {
            log(format!(
                "[local] {repo} default quant manifest unreadable: {}",
                error_chain(e)
            ));
            return None;
        }
    };
    manifest
        .layers
        .into_iter()
        .find(|l| l.media_type == "application/vnd.ollama.image.model")
        .and_then(|l| {
            l.digest
                .strip_prefix("sha256:")
                .map(str::to_ascii_lowercase)
        })
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(default)]
    layers: Vec<Layer>,
}

#[derive(Deserialize)]
struct Layer {
    #[serde(rename = "mediaType")]
    media_type: String,
    digest: String,
}

/// 列出 repo `main` 分支所有檔案（跟著分頁連結走完）；非 2xx 回 `HTTP {status}: {body}`。
pub async fn list_files(
    http: &reqwest::Client,
    base: &str,
    repo: &str,
) -> Result<Vec<RepoFile>, String> {
    let mut next = Some(format!("{base}/api/models/{repo}/tree/main?recursive=true"));
    let mut files = Vec::new();
    while let Some(url) = next {
        let response = http.get(&url).send().await.map_err(|e| {
            let e = error_chain(e);
            log(format!("[local] listing {repo} failed: {e}"));
            e
        })?;
        let status = response.status();
        if !status.is_success() {
            let body = match response.text().await {
                Ok(body) => body_preview(&body),
                Err(e) => format!("(body unreadable: {})", error_chain(e)),
            };
            log(format!(
                "[local] listing {repo} returned HTTP {status}: {body}"
            ));
            return Err(format!("HTTP {}: {body}", status.as_u16()));
        }
        next = next_page(&response);
        let entries: Vec<TreeEntry> = response.json().await.map_err(|e| {
            let e = error_chain(e);
            log(format!(
                "[local] listing {repo} returned an unreadable body: {e}"
            ));
            e
        })?;
        files.extend(
            entries
                .into_iter()
                .filter(|e| e.kind == "file")
                .map(|e| RepoFile {
                    path: e.path,
                    size: e.size,
                    sha256: e.lfs.map(|l| l.oid),
                }),
        );
    }
    log(format!("[local] listed {} files in {repo}", files.len()));
    Ok(files)
}

/// 檔案的下載網址。
pub fn resolve_url(base: &str, repo: &str, path: &str) -> String {
    format!("{base}/{repo}/resolve/main/{path}")
}

/// 由檔名取量化名稱與位元數；認不出時 `label` 為原檔名。
pub fn parse_quant(file_name: &str) -> Quant {
    let stem = SHARD_TAIL.replace(strip_gguf(file_name), "");
    let Some(label) = QUANT.captures(&stem).map(|c| c[1].to_string()) else {
        return Quant {
            label: file_name.to_string(),
            bits: None,
        };
    };
    let upper = label.to_ascii_uppercase();
    let core = upper.strip_prefix("UD-").unwrap_or(&upper);
    let bits = if core.starts_with("MXFP4") {
        Some(4)
    } else {
        match core {
            "BF16" | "F16" => Some(16),
            "F32" => Some(32),
            _ => core
                .trim_start_matches("IQ")
                .trim_start_matches('Q')
                .chars()
                .next()
                .and_then(|c| c.to_digit(10))
                .map(|d| d as u8),
        }
    };
    Quant { label, bits }
}

fn strip_gguf(name: &str) -> &str {
    let cut = name.len().saturating_sub(".gguf".len());
    match name.get(cut..) {
        Some(tail) if tail.eq_ignore_ascii_case(".gguf") => &name[..cut],
        _ => name,
    }
}

/// 分檔是否剛好是 `1..=N`（N 為檔名 `-of-N`）；非分檔的單一檔案視為完整。
fn shards_complete(files: &[RepoFile]) -> bool {
    let parts: Vec<(u32, u32)> = files
        .iter()
        .filter_map(|f| SHARD_TAIL.captures(strip_gguf(&f.path)))
        .map(|c| (c[1].parse().unwrap_or(0), c[2].parse().unwrap_or(0)))
        .collect();
    if parts.is_empty() {
        return files.len() == 1;
    }
    let total = parts[0].1 as usize;
    parts.len() == files.len()
        && total == files.len()
        && parts.iter().all(|&(_, t)| t as usize == total)
        && (1..=total as u32).all(|n| parts.iter().any(|&(p, _)| p == n))
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// 把 repo 檔案分成主模型量化版本與 MTP 檔案；略過 mmproj 與非 GGUF。
pub fn classify(files: &[RepoFile]) -> Catalog {
    let mut shards: BTreeMap<String, Vec<RepoFile>> = BTreeMap::new();
    let mut mtp = Vec::new();
    for file in files {
        let name = basename(&file.path);
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with(".gguf") || lower.contains("mmproj") {
            continue;
        }
        if lower.starts_with("mtp-") {
            mtp.push(MtpChoice {
                quant: parse_quant(name),
                file: file.clone(),
                shared: lower.contains("-shared-"),
            });
        } else {
            let group = SHARD_TAIL.replace(strip_gguf(&file.path), "").into_owned();
            shards.entry(group).or_default().push(file.clone());
        }
    }
    let mut choices: Vec<Choice> = shards
        .into_iter()
        .filter(|(group, files)| {
            let complete = shards_complete(files);
            if !complete {
                log(format!("[local] skipping incomplete shard set {group}"));
            }
            complete
        })
        .map(|(_, mut files)| {
            files.sort_by(|a, b| a.path.cmp(&b.path));
            Choice {
                quant: parse_quant(basename(&files[0].path)),
                files,
            }
        })
        .collect();
    let total = |c: &Choice| c.files.iter().map(|f| f.size).sum::<u64>();
    choices.sort_by_key(|c| (c.quant.bits.unwrap_or(u8::MAX), total(c)));
    mtp.sort_by_key(|m| (m.quant.bits.unwrap_or(u8::MAX), m.file.size));
    Catalog { choices, mtp }
}

#[cfg(test)]
mod tests {
    use super::next_link;

    /// 找出 `rel="next"` 那一段；網址含逗號、分號，或 `rel` 有多個值也認得。
    #[test]
    fn next_link_parses_link_headers_robustly() {
        let header = r#"<https://h/api?a=1,2&b=x;y>; rel="next", <https://h/api?last>; rel="last""#;
        assert_eq!(
            next_link(header).as_deref(),
            Some("https://h/api?a=1,2&b=x;y")
        );
        let reversed =
            r#"<https://h/last>; rel="last", <https://h/next>; title="a, b"; rel="prev next""#;
        assert_eq!(next_link(reversed).as_deref(), Some("https://h/next"));
        assert_eq!(
            next_link(r#"<https://h/p>; rel=NEXT"#).as_deref(),
            Some("https://h/p")
        );
        assert_eq!(next_link(r#"<https://h/last>; rel="last""#), None);
        assert_eq!(next_link("garbage"), None);
    }
}
