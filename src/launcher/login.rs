//! 登入連線：向登入伺服器取得 UserID 與 ck2。
//!
//! 多位元組欄位一律 little-endian。每則訊息包成 `0D F0` ＋ u16 本體長度 ＋ 本體。
//! 流程：伺服器先送 SESSION_OFFER → 回 SESSION_ACCEPT → 送 MSG_USER_AUTHEN_V3 →
//! 等 MSG_USER_AUTHEN_RSP。帳密與 ck2 都以 offer 的 sid／時間為金鑰的 Twofish 串流加密。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use regex::bytes::Regex;
use sha2::{Digest, Sha512};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use twofish::Twofish;
use twofish::cipher::{Block, BlockCipherEncrypt, KeyInit};

use super::{SteamTicket, hkcu_string};
use crate::config::local_state_dir;
use crate::logs::log;
use crate::util::{hex, random_u64, unhex};

const LOGIN_ADDR: &str = "login.us.wizard101.com:12000";
const TIMEOUT: Duration = Duration::from_secs(30);
const FRAME_MAGIC: [u8; 2] = [0x0D, 0xF0];
/// 等認證回應時最多讀幾則訊息（伺服器會先送其他通知）。
const MAX_RESPONSE_FRAMES: usize = 30;

/// SESSION_ACCEPT 範本：10..14 現在秒數、14..18 毫秒、18..20 sid 由 [`build_session_accept`] 改寫。
const SESSION_ACCEPT: &str = "010500000000000000001066016bbd000000bb00010000000000";
/// V3 本體開頭：一般訊息（非控制訊息）的 4 bytes 標頭。
const MESSAGE_PREFIX: [u8; 4] = [0; 4];
const LOGIN_SERVICE: u8 = 7;
const MSG_USER_AUTHEN_RSP: u8 = 14;
const MSG_USER_AUTHEN_V3: u8 = 27;
/// V3 在 Rec1 之後的欄位（Version 起到 SteamAuthTicket），版本相關欄位皆為空。
///
/// 依序：Version／Revision／DataRevision／CRC 空字串、MachineID GID、Locale "en-US"、
/// PatchClientID 佔位（換成 [`machine_id`]）、IsSteamPatcher、ConsoleType、PlatformChatID、
/// SteamID、SteamAuthTicket，最後一個 0 byte。
const REST_OFFICIAL: &str = "0000000000000000722bb7a642e600000500656e2d555349007b30303030303030302d303030302d303030302d303030302d3030303030303030303030307d3a7b30303030303030303030303030303030303030303030303030303030303030307d00000000000000010030000000";
/// 同 [`REST_OFFICIAL`]，但 IsSteamPatcher=1，SteamID 佔 17 字元、SteamAuthTicket 預留 234 bytes。
const REST_STEAM_HEAD: &str = "0000000000000000722bb7a642e600000500656e2d555349007b30303030303030302d303030302d303030302d303030302d3030303030303030303030307d3a7b30303030303030303030303030303030303030303030303030303030303030307d0100000000000011003030303030303030303030303030303030ea00";
/// Steam 範本中票據本體的起點；前 2 bytes 是票據長度，再往前是 SteamID 字串。
const STEAM_TICKET_OFFSET: usize = 126;
const STEAM_TICKET_PLACEHOLDER: usize = 234;
const STEAM_ID_LEN: usize = 17;

/// 登入成功取得的憑證；刻意不實作 `Debug`，免得 ck2 被印進 log。
pub struct LoginOutcome {
    pub user_id: String,
    pub ck2: String,
}

/// 登入失敗；訊息不含帳密、ck2 或票據。
#[derive(Debug, PartialEq, Eq)]
pub enum LoginError {
    /// 連不上或連線中斷、逾時。
    Connect(String),
    /// 伺服器拒絕登入；`raw` 是伺服器回的原因原文（沒有原文時為錯誤碼），直接顯示給使用者。
    Rejected { raw: String },
    /// 回應格式不符預期（協定可能已變更）。
    Protocol(String),
}

/// SESSION_OFFER 裡登入要用到的欄位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Offer {
    sid: u32,
    seconds: u32,
    millis: u32,
}

fn protocol(what: &str) -> LoginError {
    LoginError::Protocol(what.into())
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// 字串以 ISO-8859-1 編碼；超出範圍的字元變 `?`。
fn latin1(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| u8::try_from(c as u32).unwrap_or(b'?'))
        .collect()
}

fn frame(body: &[u8]) -> Vec<u8> {
    let mut out = FRAME_MAGIC.to_vec();
    out.extend((body.len() as u16).to_le_bytes());
    out.extend(body);
    out
}

async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Vec<u8>, LoginError> {
    let io = |e: std::io::Error| LoginError::Connect(e.to_string());
    let mut header = [0u8; 4];
    r.read_exact(&mut header).await.map_err(io)?;
    if header[..2] != FRAME_MAGIC {
        return Err(protocol("bad frame magic"));
    }
    let mut body = vec![0u8; u16::from_le_bytes([header[2], header[3]]) as usize];
    r.read_exact(&mut body).await.map_err(io)?;
    Ok(body)
}

fn parse_session_offer(body: &[u8]) -> Result<Offer, LoginError> {
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

fn build_session_accept(offer: &Offer, now_millis: u64) -> Vec<u8> {
    let mut accept = unhex(SESSION_ACCEPT).expect("valid template");
    accept[10..14].copy_from_slice(&((now_millis / 1000) as u32).to_le_bytes());
    accept[14..18].copy_from_slice(&((now_millis % 1000) as u32).to_le_bytes());
    accept[18..20].copy_from_slice(&(offer.sid as u16).to_le_bytes());
    accept
}

/// 密碼雜湊：`base64(sha512(base64(sha512(密碼)) + sid + 秒 + 毫秒))`，數字為十進位。
fn client_hash(password: &str, offer: &Offer) -> String {
    let inner = BASE64.encode(Sha512::digest(password.as_bytes()));
    let salted = format!("{inner}{}{}{}", offer.sid, offer.seconds, offer.millis);
    BASE64.encode(Sha512::digest(latin1(&salted)))
}

/// 以 offer 為金鑰的 Twofish-256 OFB 串流加解密（同一函式來回皆可）。
fn transform(data: &mut [u8], offer: &Offer) {
    let mut key: [u8; 32] = std::array::from_fn(|i| 23 + i as u8);
    let (sid, secs, ms) = (offer.sid, offer.seconds, offer.millis);
    key[4] = sid as u8;
    key[5] = (sid >> 16) as u8;
    key[6] = (sid >> 8) as u8;
    key[8] = secs as u8;
    key[9] = (secs >> 16) as u8;
    key[12] = (secs >> 8) as u8;
    key[13] = (secs >> 24) as u8;
    key[14] = ms as u8;
    key[15] = (ms >> 8) as u8;
    let cipher = Twofish::new_from_slice(&key).expect("32-byte key");
    let mut feedback = Block::<Twofish>::from(std::array::from_fn::<u8, 16, _>(|i| {
        182u8.wrapping_sub(i as u8)
    }));
    for chunk in data.chunks_mut(16) {
        cipher.encrypt_block(&mut feedback);
        for (byte, k) in chunk.iter_mut().zip(feedback.iter()) {
            *byte ^= k;
        }
    }
}

/// 把 Steam 範本預留的 SteamID 與票據換成實際值。
fn insert_steam_ticket(rest: &mut Vec<u8>, ticket: &SteamTicket) -> Result<(), LoginError> {
    let id = latin1(&ticket.steam_id);
    if id.len() != STEAM_ID_LEN || ticket.ticket.is_empty() || ticket.ticket.len() > 0xFFFF {
        return Err(protocol(
            "steam id or ticket does not fit the login message",
        ));
    }
    let id_at = STEAM_TICKET_OFFSET - 2 - STEAM_ID_LEN;
    rest[id_at..id_at + STEAM_ID_LEN].copy_from_slice(&id);
    rest[STEAM_TICKET_OFFSET - 2..STEAM_TICKET_OFFSET]
        .copy_from_slice(&(ticket.ticket.len() as u16).to_le_bytes());
    rest.splice(
        STEAM_TICKET_OFFSET..STEAM_TICKET_OFFSET + STEAM_TICKET_PLACEHOLDER,
        ticket.ticket.iter().copied(),
    );
    Ok(())
}

/// 組 MSG_USER_AUTHEN_V3 本體（未加框）。
fn build_authentication(
    account: &str,
    password: &str,
    offer: &Offer,
    machine: &str,
    steam: Option<&SteamTicket>,
) -> Result<Vec<u8>, LoginError> {
    let mut encoded = latin1(&format!(
        "{} {account} {}",
        offer.sid,
        client_hash(password, offer)
    ));
    transform(&mut encoded, offer);
    let mut rest = match steam {
        None => unhex(REST_OFFICIAL).expect("valid template"),
        Some(_) => {
            let mut rest = unhex(REST_STEAM_HEAD).expect("valid template");
            rest.extend([0u8; STEAM_TICKET_PLACEHOLDER + 1]);
            rest
        }
    };
    let slot = Regex::new(r"\{[0-9a-fA-F-]{36}\}:\{[0-9a-fA-F]{32}\}")
        .expect("valid regex")
        .find(&rest)
        .map(|m| m.range())
        .expect("template has a machine id slot");
    if slot.len() == machine.len() {
        rest[slot].copy_from_slice(&latin1(machine));
    }
    if let Some(ticket) = steam {
        insert_steam_ticket(&mut rest, ticket)?;
    }
    let total = u16::try_from(encoded.len() + rest.len() + 5)
        .map_err(|_| protocol("authentication message too long"))?;
    let mut body = MESSAGE_PREFIX.to_vec();
    body.extend([LOGIN_SERVICE, MSG_USER_AUTHEN_V3]);
    body.extend(total.to_le_bytes());
    body.extend((encoded.len() as u16).to_le_bytes());
    body.extend(encoded);
    body.extend(rest);
    Ok(body)
}

/// 解析認證回應；不是 MSG_USER_AUTHEN_RSP 的訊息回 `None`（略過繼續等）。
///
/// 欄位：8 Error、12 UserID、20 Rec1（加密的 ck2），其後 Reason 原文。
fn parse_auth_response(body: &[u8], offer: &Offer) -> Option<Result<LoginOutcome, LoginError>> {
    if body.len() < 22 || body[4] != LOGIN_SERVICE || body[5] != MSG_USER_AUTHEN_RSP {
        return None;
    }
    let error = u32_at(body, 8)?;
    let rec1_len = u16_at(body, 20)? as usize;
    if error != 0 {
        let reason = u16_at(body, 22 + rec1_len).and_then(|n| {
            let start = 24 + rec1_len;
            body.get(start..start + n as usize)
        });
        let reason: String = reason
            .unwrap_or_default()
            .iter()
            .map(|&b| b as char)
            .collect();
        let reason = reason.trim_end_matches('\0').trim();
        log(format!(
            "[login] server rejected sign-in: error=0x{error:x}, reason={reason:?}"
        ));
        let raw = if reason.is_empty() {
            format!("0x{error:x}")
        } else {
            reason.to_string()
        };
        return Some(Err(LoginError::Rejected { raw }));
    }
    let invalid = || Some(Err(protocol("server returned no valid session")));
    let user_id = u64_at(body, 12)?;
    let Some(rec1) = body.get(22..22 + rec1_len) else {
        return invalid();
    };
    let mut rec1 = rec1.to_vec();
    transform(&mut rec1, offer);
    let ck2: String = rec1.iter().map(|&b| b as char).collect();
    let ck2 = ck2.trim_end_matches('\0');
    if user_id == 0 || ck2.is_empty() || ck2.chars().any(|c| c.is_control() || !c.is_ascii()) {
        return invalid();
    }
    Some(Ok(LoginOutcome {
        user_id: user_id.to_string(),
        ck2: ck2.to_string(),
    }))
}

/// 在已連上的串流上跑完握手與認證。
async fn sign_in_on<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    account: &str,
    password: &str,
    machine: &str,
    steam: Option<&SteamTicket>,
) -> Result<LoginOutcome, LoginError> {
    let io = |e: std::io::Error| LoginError::Connect(e.to_string());
    let offer_body = read_frame(&mut stream).await?;
    let offer = parse_session_offer(&offer_body)?;
    log(format!(
        "[login] session offer: {} bytes, sid={}",
        offer_body.len(),
        offer.sid
    ));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    stream
        .write_all(&frame(&build_session_accept(&offer, now)))
        .await
        .map_err(io)?;
    let auth = build_authentication(account, password, &offer, machine, steam)?;
    stream.write_all(&frame(&auth)).await.map_err(io)?;
    log(format!(
        "[login] sent authentication ({} bytes, steam={})",
        auth.len(),
        steam.is_some()
    ));
    for _ in 0..MAX_RESPONSE_FRAMES {
        let body = read_frame(&mut stream).await?;
        if let Some(result) = parse_auth_response(&body, &offer) {
            if result.is_ok() {
                log("[login] sign-in accepted");
            }
            return result;
        }
        log(format!(
            "[login] skipped frame: {} bytes, service={:?}, message={:?}",
            body.len(),
            body.get(4),
            body.get(5)
        ));
    }
    Err(protocol("no authentication response"))
}

/// 以帳密（Steam 帳號再附票據）向登入伺服器取得 UserID 與 ck2。
pub async fn sign_in(
    account: &str,
    password: &str,
    machine: &str,
    steam: Option<&SteamTicket>,
) -> Result<LoginOutcome, LoginError> {
    let run = async {
        let stream = TcpStream::connect(LOGIN_ADDR).await.map_err(|e| {
            log(format!("[login] connect {LOGIN_ADDR} failed: {e}"));
            LoginError::Connect(e.to_string())
        })?;
        sign_in_on(stream, account, password, machine, steam).await
    };
    let result = tokio::time::timeout(TIMEOUT, run)
        .await
        .unwrap_or_else(|_| {
            log(format!("[login] timed out after {}s", TIMEOUT.as_secs()));
            Err(LoginError::Connect("timed out".into()))
        });
    if let Err(e @ (LoginError::Connect(_) | LoginError::Protocol(_))) = &result {
        log(format!("[login] sign-in failed: {e:?}"));
    }
    result
}

fn is_machine_id(s: &str) -> bool {
    Regex::new(r"^\{[0-9A-F]{8}(-[0-9A-F]{4}){3}-[0-9A-F]{12}\}:\{[0-9A-F]{32}\}$")
        .expect("valid regex")
        .is_match(s.as_bytes())
}

/// 組一個新的機器識別：沿用遊戲存的 UUID（沒有就隨機產生），配 16 bytes 隨機 hex。
fn new_machine_id(game_uuid: Option<String>) -> String {
    let random16 = || {
        let mut b = random_u64().to_le_bytes().to_vec();
        b.extend(random_u64().to_le_bytes());
        hex(&b).to_uppercase()
    };
    let uuid = game_uuid
        .map(|u| u.trim_matches(['{', '}']).to_uppercase())
        .filter(|u| is_machine_id(&format!("{{{u}}}:{{{}}}", "0".repeat(32))))
        .unwrap_or_else(|| {
            let h = random16();
            format!(
                "{}-{}-{}-{}-{}",
                &h[..8],
                &h[8..12],
                &h[12..16],
                &h[16..20],
                &h[20..]
            )
        });
    format!("{{{uuid}}}:{{{}}}", random16())
}

/// 本機固定的機器識別（V3 的 PatchClientID）；第一次產生後存在本機狀態目錄沿用。
pub fn machine_id() -> String {
    let path = local_state_dir().join("launcher-machine-id.txt");
    if let Ok(saved) = std::fs::read_to_string(&path)
        && is_machine_id(saved.trim())
    {
        return saved.trim().to_string();
    }
    let id = new_machine_id(hkcu_string(r"Software\KingsIsle", "UUID"));
    if let Err(e) = crate::util::write_atomic(&path, &id) {
        log(format!(
            "[login] could not save machine id to {}: {e}",
            path.display()
        ));
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFFER: Offer = Offer {
        sid: 187,
        seconds: 1_795_266_064,
        millis: 189,
    };
    const MACHINE: &str =
        "{0123ABCD-0000-1111-2222-333344445555}:{00112233445566778899AABBCCDDEEFF}";

    fn offer_body() -> Vec<u8> {
        let mut body = vec![0u8; 120];
        body[0] = 1;
        body[4..8].copy_from_slice(&OFFER.sid.to_le_bytes());
        body[10..14].copy_from_slice(&OFFER.seconds.to_le_bytes());
        body[14..18].copy_from_slice(&OFFER.millis.to_le_bytes());
        body
    }

    /// 依回應訊息欄位順序組一份 MSG_USER_AUTHEN_RSP 本體。
    fn response_body(error: u32, user_id: u64, rec1: &[u8], reason: &str) -> Vec<u8> {
        let mut body = vec![0, 0, 0, 0, 7, 14, 0, 0];
        body.extend(error.to_le_bytes());
        body.extend(user_id.to_le_bytes());
        body.extend((rec1.len() as u16).to_le_bytes());
        body.extend(rec1);
        body.extend((reason.len() as u16).to_le_bytes());
        body.extend(reason.as_bytes());
        body.extend([0u8; 12]);
        body
    }

    #[test]
    fn parse_session_offer_reads_fields() {
        assert_eq!(parse_session_offer(&offer_body()).unwrap(), OFFER);
        assert!(matches!(
            parse_session_offer(&offer_body()[..60]),
            Err(LoginError::Protocol(_))
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

    #[test]
    fn client_hash_matches_reference() {
        // 期望值以 Python hashlib／base64 獨立算出
        assert_eq!(
            client_hash("hunter2", &OFFER),
            "+sJVncJlMufgEkb1HJ+c7rOjLgY4oq/aqcbldnEm3NtsdITiWTG2C2CENv+KcsctSpT/LSDqao189pTSq81S6A=="
        );
    }

    #[test]
    fn transform_is_symmetric_and_keyed_by_session() {
        let plain: Vec<u8> = (0..40).collect();
        let mut data = plain.clone();
        transform(&mut data, &OFFER);
        assert_ne!(data, plain);
        let mut other = plain.clone();
        transform(&mut other, &Offer { sid: 188, ..OFFER });
        assert_ne!(other, data);
        transform(&mut data, &OFFER);
        assert_eq!(data, plain);
    }

    #[test]
    fn authentication_frame_has_v3_header_and_machine_id() {
        let body = build_authentication("alice", "hunter2", &OFFER, MACHINE, None).unwrap();
        assert_eq!(body[..6], [0, 0, 0, 0, 7, 27]);
        let encoded_len = u16::from_le_bytes([body[8], body[9]]) as usize;
        let total = u16::from_le_bytes([body[6], body[7]]) as usize;
        let rest = &body[10 + encoded_len..];
        assert_eq!(total, encoded_len + rest.len() + 5);
        let mut encoded = body[10..10 + encoded_len].to_vec();
        transform(&mut encoded, &OFFER);
        let expected = format!("187 alice {}", client_hash("hunter2", &OFFER));
        assert_eq!(encoded, expected.as_bytes());
        assert!(rest.windows(MACHINE.len()).any(|w| w == MACHINE.as_bytes()));
        assert!(!body.windows(7).any(|w| w == b"hunter2"));
    }

    #[test]
    fn steam_authentication_carries_id_and_ticket() {
        let ticket = SteamTicket {
            steam_id: "76561198000000001".into(),
            ticket: vec![0xAB; 300],
        };
        let body = build_authentication("alice", "pw", &OFFER, MACHINE, Some(&ticket)).unwrap();
        let encoded_len = u16::from_le_bytes([body[8], body[9]]) as usize;
        let rest = &body[10 + encoded_len..];
        let at = STEAM_TICKET_OFFSET;
        assert_eq!(rest[at - 2..at], 300u16.to_le_bytes());
        assert_eq!(&rest[at..at + 300], &ticket.ticket[..]);
        assert_eq!(&rest[at - 19..at - 2], b"76561198000000001");
        assert_eq!(rest.len(), at + 300 + 1);
        let bad = SteamTicket {
            steam_id: "123".into(),
            ..ticket
        };
        assert!(build_authentication("alice", "pw", &OFFER, MACHINE, Some(&bad)).is_err());
    }

    #[test]
    fn auth_response_surfaces_server_reason() {
        let body = response_body(0x0d, 0, b"", "AccountClientMismatch");
        assert_eq!(
            parse_auth_response(&body, &OFFER).and_then(Result::err),
            Some(LoginError::Rejected {
                raw: "AccountClientMismatch".into()
            })
        );
        let body = response_body(0x0d, 0, b"", "");
        assert_eq!(
            parse_auth_response(&body, &OFFER).and_then(Result::err),
            Some(LoginError::Rejected { raw: "0xd".into() })
        );
    }

    #[test]
    fn auth_response_success_decodes_session() {
        let ck2 = "Q".repeat(88);
        let mut rec1 = ck2.clone().into_bytes();
        rec1.push(0);
        transform(&mut rec1, &OFFER);
        let body = response_body(0, 1_234_567_890_123, &rec1, "");
        let outcome = parse_auth_response(&body, &OFFER).unwrap().unwrap();
        assert_eq!(outcome.user_id, "1234567890123");
        assert_eq!(outcome.ck2, ck2);
    }

    #[test]
    fn unrelated_frames_are_skipped() {
        assert!(parse_auth_response(&[0, 0, 0, 0, 7, 3, 0, 0], &OFFER).is_none());
        let mut other = response_body(0, 1, b"x", "");
        other[5] = 2;
        assert!(parse_auth_response(&other, &OFFER).is_none());
    }

    #[tokio::test]
    async fn sign_in_runs_the_whole_exchange() {
        let (client, mut server) = tokio::io::duplex(4096);
        let server = tokio::spawn(async move {
            server.write_all(&frame(&offer_body())).await.unwrap();
            let accept = read_frame(&mut server).await.unwrap();
            assert_eq!(accept.len(), 26);
            let auth = read_frame(&mut server).await.unwrap();
            assert_eq!(auth[4..6], [7, 27]);
            server
                .write_all(&frame(&[0, 0, 0, 0, 7, 3, 0, 0]))
                .await
                .unwrap();
            let mut rec1 = b"CK2".to_vec();
            transform(&mut rec1, &OFFER);
            server
                .write_all(&frame(&response_body(0, 42, &rec1, "")))
                .await
                .unwrap();
        });
        let outcome = sign_in_on(client, "alice", "pw", MACHINE, None)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(outcome.user_id, "42");
        assert_eq!(outcome.ck2, "CK2");
    }

    #[tokio::test]
    async fn bad_frame_magic_is_a_protocol_error() {
        let (client, mut server) = tokio::io::duplex(64);
        server.write_all(&[1, 2, 0, 0]).await.unwrap();
        assert!(matches!(
            sign_in_on(client, "alice", "pw", MACHINE, None).await,
            Err(LoginError::Protocol(_))
        ));
    }

    #[test]
    fn machine_id_shape_is_braced_guid_and_hex() {
        let id = new_machine_id(Some("0123abcd-0000-1111-2222-333344445555".into()));
        assert_eq!(id.len(), 73);
        assert!(id.starts_with("{0123ABCD-0000-1111-2222-333344445555}:{"));
        assert!(is_machine_id(&id));
        assert!(is_machine_id(&new_machine_id(None)));
        assert!(!is_machine_id("garbage"));
    }
}
