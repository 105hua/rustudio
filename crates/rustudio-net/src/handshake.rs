//! The client's opening messages on the replication lane (channel 4, lane 1),
//! as Studio 0.746 sends them:
//!
//! | id | what |
//! |---|---|
//! | `0xa8` | session-crypto hello from `ClientTicket` (103 bytes in the reference capture) |
//! | `0xa7` | shared-string cache: `<u16le count>`, then count 16-byte hashes |
//! | `0x90` | protocol sync: flag names to sync + the player JSON |
//! | `0x92` | place verification (9-byte varint) |
//! | `0x8a` | submit ticket |
//! | `0x8f` | preferred spawn name (`8f 00`) |
//!
//! Builders fill the per-session values from the join script. Secrets (the
//! ticket, `APIsecurityToken`, `SessionId`) only ever live in the returned
//! buffers, which go straight to the encrypted QUIC stream.

use crate::error::{NetError, Result};
use crate::joinscript::JoinScript;

/// Build the client's shared-string cache announcement. A new client has no
/// cached entries and correctly sends an empty list; the server then supplies
/// the strings it needs in the replication stream.
pub fn shared_string_cache_message(hashes: &[[u8; 16]]) -> Result<Vec<u8>> {
    let count = u16::try_from(hashes.len())
        .map_err(|_| NetError::Other("too many shared-string cache hashes".into()))?;
    let mut out = Vec::with_capacity(3 + hashes.len() * 16);
    out.push(0xa7);
    out.extend_from_slice(&count.to_le_bytes());
    for hash in hashes {
        out.extend_from_slice(hash);
    }
    Ok(out)
}

/// The per-session 0x92 nonce. Studio combines a fresh random u32 with a
/// build-specific mask, then writes the combined i64 as a ZigZag varint
///. The mask is recovered from the reference
/// capture as `random ^ low_word`.
pub fn place_verification(random: u32) -> Vec<u8> {
    const BUILD_MASK: u32 = 0x9d58_f351;
    let combined = ((random as u64) << 32) | ((random ^ BUILD_MASK) as u64);
    let mut out = vec![0x92];
    put_varint(&mut out, zigzag(combined as i64));
    out
}

/// Generate a fresh nonce without reusing a captured session's value.
pub fn fresh_place_verification() -> Result<Vec<u8>> {
    use ring::rand::SecureRandom;
    let rng = ring::rand::SystemRandom::new();
    let mut random = [0u8; 4];
    rng.fill(&mut random)
        .map_err(|_| NetError::Other("OS random source failed for 0x92".into()))?;
    Ok(place_verification(u32::from_le_bytes(random)))
}

pub fn put_varint(v: &mut Vec<u8>, mut x: u64) {
    loop {
        let b = (x & 0x7f) as u8;
        x >>= 7;
        if x == 0 {
            v.push(b);
            return;
        }
        v.push(b | 0x80);
    }
}

pub fn put_varstr(v: &mut Vec<u8>, s: &[u8]) {
    put_varint(v, s.len() as u64);
    v.extend_from_slice(s);
}

pub fn zigzag(x: i64) -> u64 {
    ((x << 1) ^ (x >> 63)) as u64
}

pub fn get_varint(b: &[u8], o: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    let mut s = 0;
    loop {
        let x = *b.get(*o)?;
        *o += 1;
        v |= ((x & 0x7f) as u64) << s;
        if x & 0x80 == 0 {
            return Some(v);
        }
        s += 7;
        if s > 63 {
            return None;
        }
    }
}

/// The parts of a captured `0x90` that are per build, not per session.
#[derive(Clone, Debug)]
pub struct ProtocolSyncTemplate {
    /// Everything before the JSON varint (header + flag-name list).
    pub head: Vec<u8>,
    /// The 20 bytes after the JSON.
    pub tail: Vec<u8>,
}

impl ProtocolSyncTemplate {
    /// Split a captured `0x90` message around its player JSON.
    pub fn from_capture(m: &[u8]) -> Option<ProtocolSyncTemplate> {
        let i = find(m, b"{\"AccountAge")?;
        // the JSON length varint sits right before the '{' (1-3 bytes)
        let mut start = None;
        for k in 1..=3 {
            if i < k {
                break;
            }
            let mut o = i - k;
            if let Some(n) = get_varint(m, &mut o) {
                if o == i && i + n as usize <= m.len() {
                    start = Some((i - k, n as usize));
                }
            }
        }
        let (vs, n) = start?;
        Some(ProtocolSyncTemplate {
            head: m[..vs].to_vec(),
            tail: m[i + n..].to_vec(),
        })
    }
}

/// The player JSON of `0x90`, from the join script, with Studio's key order.
pub fn player_json(raw: &serde_json::Value) -> String {
    let g = |k: &str| raw.get(k).cloned().unwrap_or(serde_json::Value::Null);
    let under13 = g("IsUnknownOrUnder13");
    let mut m = serde_json::Map::new();
    m.insert("AccountAge".into(), g("AccountAge"));
    m.insert("UserId".into(), g("UserId"));
    m.insert("CharacterAppearanceId".into(), g("characterAppearanceId"));
    m.insert("IsTeleportedIn".into(), false.into());
    m.insert("UserName".into(), g("UserName"));
    m.insert("CharacterAppearance".into(), g("CharacterAppearance"));
    m.insert("PrivateServerID".into(), g("PrivateServerID"));
    m.insert("IsUnknownOrUnder13".into(), under13.clone());
    m.insert("IsUnder13".into(), under13);
    m.insert(
        "WhoCanWhisperChatWithMeInExperiences".into(),
        g("WhoCanWhisperChatWithMeInExperiences"),
    );
    m.insert("SuperSafeChat".into(), g("SuperSafeChat"));
    m.insert("MembershipType".into(), g("MembershipType"));
    m.insert("GameChatType".into(), g("GameChatType"));
    m.insert("HasRobloxSubscription".into(), g("HasRobloxSubscription"));
    m.insert(
        "AgeChecked".into(),
        serde_json::Value::String(match g("AgeChecked") {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s,
            _ => "0".into(),
        }),
    );
    m.insert("DomainUserId".into(), g("DomainUserId"));
    m.insert("UserIdMode".into(), g("UserIdMode"));
    m.insert("ServerClaimedTime".into(), g("ServerClaimedTime"));
    m.insert("CountryRegionCode".into(), g("CountryCode"));
    m.insert("DisplayName".into(), g("DisplayName"));
    m.insert("HasVerifiedBadge".into(), g("HasVerifiedBadge"));
    m.insert("VerifiedAMP".into(), g("VerifiedAMP"));
    m.insert("APIsecurityToken".into(), g("APIsecurityToken"));
    serde_json::Value::Object(m).to_string()
}

pub fn protocol_sync(t: &ProtocolSyncTemplate, raw: &serde_json::Value) -> Vec<u8> {
    let j = player_json(raw);
    let mut v = t.head.clone();
    put_varstr(&mut v, j.as_bytes());
    v.extend_from_slice(&t.tail);
    v
}

/// The build-specific middle of a captured `0x8a`: the bytes between the
/// ticket and the SessionId JSON (`u32 36`, a 74-char build hash string,
/// `"Win32"`, two flag bytes), and the 4-byte tail.
#[derive(Clone, Debug)]
pub struct TicketTemplate {
    pub middle: Vec<u8>,
    pub tail: Vec<u8>,
}

impl TicketTemplate {
    pub fn from_capture(m: &[u8]) -> Option<TicketTemplate> {
        let mut o = 1;
        get_varint(m, &mut o)?; // user id
        let tl = get_varint(m, &mut o)? as usize;
        o += tl;
        let j = find(&m[o..], b"{\"SessionId")? + o;
        // the JSON length varint before '{'
        for k in 1..=3 {
            let mut p = j - k;
            if let Some(n) = get_varint(m, &mut p) {
                // The 0x8a body ends with four bytes after the JSON. Require
                // the candidate length to land exactly there: the last byte
                // of a multi-byte varint is often itself a valid short
                // varint, and accepting it duplicates most of the JSON.
                if p == j && j + n as usize + 4 == m.len() {
                    return Some(TicketTemplate {
                        middle: m[o..j - k].to_vec(),
                        tail: m[j + n as usize..].to_vec(),
                    });
                }
            }
        }
        None
    }
}

fn session_json_for_wire(sid: &str) -> String {
    // Preserve every matchmaker value, including BrowserTrackerId=0: changing
    // that value makes the server reject the submit ticket (live verified).
    serde_json::from_str::<serde_json::Value>(sid)
        .ok()
        .and_then(|v| serde_json::to_string(&v).ok())
        .unwrap_or_else(|| sid.to_string())
}

pub fn submit_ticket(t: &TicketTemplate, js: &JoinScript, raw: &serde_json::Value) -> Vec<u8> {
    let mut v = vec![0x8a];
    put_varint(&mut v, zigzag(js.user_id.unwrap_or(0) as i64));
    put_varstr(&mut v, js.client_ticket.expose().as_bytes());
    v.extend_from_slice(&t.middle);
    let sid = raw
        .get("SessionId")
        .and_then(|s| s.as_str())
        .unwrap_or_default();
    // Studio parses and re-serializes this nested JSON instead of forwarding
    // the matchmaker's formatting. Keep its key order (preserve_order), but
    // remove whitespace and any transport-specific escaping.
    let compact = session_json_for_wire(sid);
    put_varstr(&mut v, compact.as_bytes());
    v.extend_from_slice(&t.tail);
    v
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn varints() {
        let mut v = vec![];
        put_varint(&mut v, 510);
        assert_eq!(v, vec![0xfe, 0x03]);
        let mut v = vec![];
        put_varint(&mut v, zigzag(10000000001));
        assert_eq!(v, vec![0x82, 0x90, 0xdf, 0xc0, 0x4a]);
    }

    #[test]
    fn session_json_preserves_matchmaker_browser_tracker() {
        let zero = session_json_for_wire(r#"{"SessionId":"s","BrowserTrackerId":0}"#);
        assert_eq!(zero, r#"{"SessionId":"s","BrowserTrackerId":0}"#);
        let existing = session_json_for_wire(r#"{"SessionId":"s","BrowserTrackerId":99}"#);
        assert_eq!(existing, r#"{"SessionId":"s","BrowserTrackerId":99}"#);
    }

    #[test]
    fn ticket_template_does_not_duplicate_multibyte_json() {
        let json = format!("{{\"SessionId\":\"{}\"}}", "x".repeat(180));
        let mut captured = vec![0x8a];
        put_varint(&mut captured, zigzag(42));
        put_varstr(&mut captured, b"ticket");
        captured.extend_from_slice(&[9, 8, 7]);
        put_varstr(&mut captured, json.as_bytes());
        captured.extend_from_slice(&[1, 2, 3, 4]);

        let t = TicketTemplate::from_capture(&captured).unwrap();
        assert_eq!(t.middle, [9, 8, 7]);
        assert_eq!(t.tail, [1, 2, 3, 4]);

        let mut rebuilt = vec![0x8a];
        put_varint(&mut rebuilt, zigzag(42));
        put_varstr(&mut rebuilt, b"ticket");
        rebuilt.extend_from_slice(&t.middle);
        put_varstr(&mut rebuilt, json.as_bytes());
        rebuilt.extend_from_slice(&t.tail);
        assert_eq!(rebuilt, captured);
    }

    #[test]
    fn place_verification_matches_the_captured_known_answer() {
        assert_eq!(
            place_verification(0x0403_531d),
            [0x92, 0x98, 0x81, 0xdd, 0x95, 0xb3, 0xc7, 0xa9, 0x83, 0x08]
        );
        assert_ne!(
            fresh_place_verification().unwrap(),
            fresh_place_verification().unwrap()
        );
    }

    #[test]
    fn shared_string_cache_has_little_endian_block_count() {
        assert_eq!(shared_string_cache_message(&[]).unwrap(), [0xa7, 0, 0]);
        let hashes = [[0x11; 16], [0x22; 16]];
        let m = shared_string_cache_message(&hashes).unwrap();
        assert_eq!(&m[..3], &[0xa7, 2, 0]);
        assert_eq!(m.len(), 35);
        assert_eq!(&m[3..19], &[0x11; 16]);
        assert_eq!(&m[19..], &[0x22; 16]);
    }
}
