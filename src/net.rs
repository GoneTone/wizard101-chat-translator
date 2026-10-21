//! 網路層共用工具：HTTPS crypto provider、本機位址不走 proxy、錯誤原因鏈。

use std::error::Error as _;

use crate::logs::log;
use crate::py::pystr;

/// 裝上 rustls 的 ring crypto provider；任何要建 HTTPS client 的地方都得先呼叫。
pub fn install_crypto_provider() {
    // 用 ring 而非 rustls 預設的 aws-lc-rs（建置要 cmake／NASM）；已裝過時回 Err，忽略即可
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// 連往本機（`localhost`、`127.0.0.0/8`、`::1`）時不走任何 proxy（環境變數或系統設定）：本機的 LLM 伺服器繞去 proxy 只會連不到。
pub fn bypass_proxy_for_loopback(
    builder: reqwest::ClientBuilder,
    url: &str,
) -> reqwest::ClientBuilder {
    if !is_loopback(url) {
        return builder;
    }
    log(format!(
        "[net] {url} is a loopback address; not using a proxy"
    ));
    builder.no_proxy()
}

fn is_loopback(url: &str) -> bool {
    let Some(host) = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
    else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// reqwest 錯誤去掉 URL 後，連同整條原因鏈折成一行（頂層訊息常只有 "error sending request"）。
pub fn error_chain(error: reqwest::Error) -> String {
    let error = error.without_url();
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    pystr::one_line(&parts.join(": "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_hosts_are_recognised() {
        for url in [
            "http://localhost:11434/v1",
            "http://LOCALHOST:1234",
            "http://127.0.0.1:8080/v1",
            "http://127.5.6.7",
            "http://[::1]:11434/v1",
        ] {
            assert!(is_loopback(url), "{url}");
        }
        for url in [
            "https://api.anthropic.com",
            "http://192.168.1.10:11434/v1",
            "http://localhost.example.com",
            "not a url",
        ] {
            assert!(!is_loopback(url), "{url}");
        }
    }
}
