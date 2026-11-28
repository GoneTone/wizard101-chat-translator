//! KI 訊息框架：登入伺服器與更新伺服器共用的框、讀框與 session 握手。
//!
//! 多位元組欄位一律 little-endian。每則訊息包成 `0D F0` ＋ u16 本體長度 ＋ 本體。

use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::logs::log;
use crate::util::unhex;

pub const FRAME_MAGIC: [u8; 2] = [0x0D, 0xF0];
/// 本體開頭：一般訊息（非控制訊息）的 4 bytes 標頭。
pub const MESSAGE_PREFIX: [u8; 4] = [0; 4];
/// SESSION_ACCEPT 範本：10..14 現在秒數、14..18 毫秒、18..20 sid 由 [`build_session_accept`] 改寫。
const SESSION_ACCEPT: &str = "010500000000000000001066016bbd000000bb00010000000000";

/// 框架層錯誤；訊息不含帳密或票據。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireError {
    /// 連線中斷或讀寫失敗。
    Io(String),
    /// 回應格式不符預期。
    Protocol(String),
}

/// SESSION_OFFER 裡要用到的欄位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offer {
    pub sid: u32,
    pub seconds: u32,
    pub millis: u32,
}

fn protocol(what: &str) -> WireError {
    WireError::Protocol(what.into())
}

fn io(e: std::io::Error) -> WireError {
    WireError::Io(e.to_string())
}

pub fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

pub fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

pub fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// 把本體加上 `0D F0` ＋ 長度。
pub fn frame(body: &[u8]) -> Vec<u8> {
    let mut out = FRAME_MAGIC.to_vec();
    out.extend((body.len() as u16).to_le_bytes());
    out.extend(body);
    out
}

/// 讀一個完整的框並回傳本體。
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Vec<u8>, WireError> {
    let mut header = [0u8; 4];
    r.read_exact(&mut header).await.map_err(io)?;
    if header[..2] != FRAME_MAGIC {
        return Err(protocol("bad frame magic"));
    }
    let mut body = vec![0u8; u16::from_le_bytes([header[2], header[3]]) as usize];
    r.read_exact(&mut body).await.map_err(io)?;
    Ok(body)
}

pub fn parse_session_offer(body: &[u8]) -> Result<Offer, WireError> {
    if body.len() < 100 {
        return Err(protocol("session offer too short"));
    }
    let read = |at| u32_at(body, at).ok_or_else(|| protocol("session offer truncated"));
    // sid 本身是 u16，緊接的保留欄位為 0；照 u32 讀，兩者結果相同
    Ok(Offer {
        sid: read(4)?,
        seconds: read(10)?,
        millis: read(14)?,
    })
}

pub fn build_session_accept(offer: &Offer, now_millis: u64) -> Vec<u8> {
    let mut accept = unhex(SESSION_ACCEPT).expect("valid template");
    accept[10..14].copy_from_slice(&((now_millis / 1000) as u32).to_le_bytes());
    accept[14..18].copy_from_slice(&((now_millis % 1000) as u32).to_le_bytes());
    accept[18..20].copy_from_slice(&(offer.sid as u16).to_le_bytes());
    accept
}

/// 讀伺服器的 SESSION_OFFER、回 SESSION_ACCEPT，回傳 offer。
pub async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S) -> Result<Offer, WireError> {
    let offer_body = read_frame(s).await?;
    let offer = parse_session_offer(&offer_body)?;
    log(format!(
        "[wire] session offer: {} bytes, sid={}",
        offer_body.len(),
        offer.sid
    ));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    s.write_all(&frame(&build_session_accept(&offer, now)))
        .await
        .map_err(io)?;
    Ok(offer)
}

/// 測試用的固定 offer。
#[cfg(test)]
pub(crate) const OFFER: Offer = Offer {
    sid: 187,
    seconds: 1_795_266_064,
    millis: 189,
};

/// 依 [`OFFER`] 組一份 SESSION_OFFER 本體。
#[cfg(test)]
pub(crate) fn offer_body() -> Vec<u8> {
    let mut body = vec![0u8; 120];
    body[0] = 1;
    body[4..8].copy_from_slice(&OFFER.sid.to_le_bytes());
    body[10..14].copy_from_slice(&OFFER.seconds.to_le_bytes());
    body[14..18].copy_from_slice(&OFFER.millis.to_le_bytes());
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_session_offer_reads_fields() {
        assert_eq!(parse_session_offer(&offer_body()).unwrap(), OFFER);
        assert!(matches!(
            parse_session_offer(&offer_body()[..60]),
            Err(WireError::Protocol(_))
        ));
    }

    #[test]
    fn build_session_accept_is_26_bytes() {
        let accept = build_session_accept(&OFFER, 1_700_000_123_456);
        assert_eq!(accept.len(), 26);
        assert_eq!(accept[..2], [1, 5]);
        assert_eq!(accept[10..14], 1_700_000_123u32.to_le_bytes());
        assert_eq!(accept[14..18], 456u32.to_le_bytes());
        assert_eq!(accept[18..20], 187u16.to_le_bytes());
    }

    #[tokio::test]
    async fn bad_frame_magic_is_a_protocol_error() {
        let (mut client, mut server) = tokio::io::duplex(64);
        server.write_all(&[1, 2, 0, 0]).await.unwrap();
        assert!(matches!(
            read_frame(&mut client).await,
            Err(WireError::Protocol(_))
        ));
    }

    #[tokio::test]
    async fn handshake_reads_offer_and_sends_accept() {
        let (mut client, mut server) = tokio::io::duplex(4096);
        server.write_all(&frame(&offer_body())).await.unwrap();
        let offer = handshake(&mut client).await.unwrap();
        assert_eq!(offer.sid, OFFER.sid);
        let accept = read_frame(&mut server).await.unwrap();
        assert_eq!(accept.len(), 26);
        assert_eq!(accept[18..20], (OFFER.sid as u16).to_le_bytes());
    }
}
