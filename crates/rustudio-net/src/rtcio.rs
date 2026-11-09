//! The `rbx-rtcio` channel layer on top of QUIC (docs/PROTOCOL.md §3b).
//!
//! - Every unidirectional stream starts `06 01 <u8 channel> <4-byte tag>`:
//!   the tag is a big-endian lane sequence number, or ASCII `ctrl` on a
//!   channel's control stream.
//! - Then messages, each `<u32 BE 0x80000000 | length> <bytes>`.
//! - Channel 4 lane 1 carries the replicator (classic Roblox packet ids:
//!   0x83 ID_DATA, 0x90 protocol sync, …). Channel 6 lane 1 carries the
//!   77-byte ping the server echoes. DATAGRAM frames carry
//!   `<u8 2><QUIC varint seq><u8 0>` + an unreliable replicator message.

use crate::error::{NetError, Result};

pub const CH_CONTROL0: u8 = 0;
pub const CH_REPLICATION: u8 = 4;
pub const CH_PING: u8 = 6;
pub const TAG_CTRL: [u8; 4] = *b"ctrl";

pub fn lane_tag(seq: u32) -> [u8; 4] {
    seq.to_be_bytes()
}

/// One framed message.
pub fn frame(msg: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + msg.len());
    v.extend_from_slice(&(0x8000_0000u32 | msg.len() as u32).to_be_bytes());
    v.extend_from_slice(msg);
    v
}

/// Open a client lane and write its header.
pub async fn open_lane(
    conn: &quinn::Connection,
    channel: u8,
    tag: [u8; 4],
) -> Result<quinn::SendStream> {
    let mut s = conn
        .open_uni()
        .await
        .map_err(|e| NetError::Other(format!("open stream: {e}")))?;
    let hdr = [6, 1, channel, tag[0], tag[1], tag[2], tag[3]];
    s.write_all(&hdr)
        .await
        .map_err(|e| NetError::Other(format!("write: {e}")))?;
    Ok(s)
}

pub async fn send(s: &mut quinn::SendStream, msg: &[u8]) -> Result<()> {
    s.write_all(&frame(msg))
        .await
        .map_err(|e| NetError::Other(format!("write: {e}")))
}

/// Read a server lane's 7-byte header: (channel, tag).
pub async fn read_header(r: &mut quinn::RecvStream) -> Option<(u8, [u8; 4])> {
    let mut h = [0u8; 7];
    r.read_exact(&mut h).await.ok()?;
    if h[0] != 6 || h[1] != 1 {
        return None;
    }
    Some((h[2], [h[3], h[4], h[5], h[6]]))
}

/// Read the next message from a lane, preserving QUIC reset/finish errors for
/// diagnostics. Callers that only need `Option` can use [`read_msg`].
pub async fn read_msg_result(r: &mut quinn::RecvStream) -> Result<Vec<u8>> {
    let mut l = [0u8; 4];
    r.read_exact(&mut l)
        .await
        .map_err(|e| NetError::Other(format!("read frame length: {e}")))?;
    let n = (u32::from_be_bytes(l) & 0x7fff_ffff) as usize;
    let mut m = vec![0u8; n];
    r.read_exact(&mut m)
        .await
        .map_err(|e| NetError::Other(format!("read {n}-byte frame: {e}")))?;
    Ok(m)
}

/// Read the next message from a lane, `None` at the end.
pub async fn read_msg(r: &mut quinn::RecvStream) -> Option<Vec<u8>> {
    read_msg_result(r).await.ok()
}

/// Split an unreliable DATAGRAM payload: (lane byte, sequence, message).
pub fn split_datagram(d: &[u8]) -> Option<(u8, u64, &[u8])> {
    let lane = *d.first()?;
    let (seq, n) = qvarint(&d[1..])?;
    let rest = d.get(1 + n..)?;
    Some((lane, seq, rest.get(1..)?))
}

/// Build a DATAGRAM payload.
pub fn datagram(lane: u8, seq: u64, msg: &[u8]) -> Vec<u8> {
    let mut v = vec![lane];
    put_qvarint(&mut v, seq);
    v.push(0);
    v.extend_from_slice(msg);
    v
}

pub fn qvarint(b: &[u8]) -> Option<(u64, usize)> {
    let f = *b.first()?;
    let n = 1usize << (f >> 6);
    let s = b.get(..n)?;
    let mut v = (f & 0x3f) as u64;
    for x in &s[1..] {
        v = (v << 8) | *x as u64;
    }
    Some((v, n))
}

pub fn put_qvarint(v: &mut Vec<u8>, x: u64) {
    if x < 64 {
        v.push(x as u8);
    } else if x < 16384 {
        v.extend_from_slice(&((x as u16) | 0x4000).to_be_bytes());
    } else if x < (1 << 30) {
        v.extend_from_slice(&((x as u32) | 0x8000_0000).to_be_bytes());
    } else {
        v.extend_from_slice(&(x | 0xc000_0000_0000_0000).to_be_bytes());
    }
}

/// The 77-byte channel-6 ping Studio sends once a second [verified:
/// capture]: `<u64 LE timestamp µs><u32 LE 64><u8 1><64 × 0x50>`. The server
/// echoes it byte for byte.
pub fn ping_message(timestamp_us: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(77);
    v.extend_from_slice(&timestamp_us.to_le_bytes());
    v.extend_from_slice(&64u32.to_le_bytes());
    v.push(1);
    v.extend_from_slice(&[0x50; 64]);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn datagram_header_matches_capture() {
        // captured: `02 40 41 00 83 05 …` and `02 01 00 83 05 …`
        let d = datagram(2, 0x41, &[0x83, 0x05]);
        assert_eq!(d, vec![2, 0x40, 0x41, 0, 0x83, 5]);
        assert_eq!(split_datagram(&d), Some((2, 0x41, &[0x83u8, 5][..])));
        assert_eq!(
            split_datagram(&[2, 1, 0, 0x83]),
            Some((2, 1, &[0x83u8][..]))
        );
        assert_eq!(ping_message(0).len(), 77);
        assert_eq!(frame(&[1, 2]), vec![0x80, 0, 0, 2, 1, 2]);
    }
}
