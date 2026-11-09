//! RUPP: the relay header Roblox's UDMUX puts in front of every RbxTransport
//! datagram (docs/PROTOCOL.md §2, "RUPP header").
//!
//! Wire form [verified: capture + live 2026-11-05]:
//!
//! ```text
//! [u8 version=1][u8 flags=0][u16 BE total header length] TLV*
//! TLV = [u8 type][u8 length][value]
//!   type 1 (TokenTlv), length 17: [u8 token type][16-byte token]
//!   type 2 (Ipv4Tlv),  length 6:  [4-byte IPv4][u16 BE port]   (client->relay only)
//! ```
//!
//! - The client's first datagrams carry token type **1** with the 16 bytes of
//!   the join script's base64 `NetStackTokenValue`, and an Ipv4Tlv naming the
//!   RCC (`MachineAddress:NetStackPort`). They go to the UDMUX address
//!   (`UdmuxEndpoints[0].Address`) at `NetStackPort`.
//! - Every relay->client datagram carries token type **2** with a 16-byte
//!   token the relay rotates about once a second. The client echoes the most
//!   recent one ("Switching to new TT token received from RCC").
//!
//! [`RuppSocket`] wraps a UDP socket as a `quinn::AsyncUdpSocket` that adds
//! the header on send and strips it on receive, so quinn speaks plain QUIC.

use std::fmt;
use std::io::{self, IoSliceMut};
use std::net::{Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use quinn::udp::{RecvMeta, Transmit};
use quinn::{AsyncUdpSocket, UdpPoller};

pub const TLV_TOKEN: u8 = 1;
pub const TLV_IPV4: u8 = 2;
pub const TOKEN_TYPE_JOIN: u8 = 1;
pub const TOKEN_TYPE_RELAY: u8 = 2;

/// The token state shared by send and receive.
#[derive(Clone)]
struct TokenState {
    kind: u8,
    token: [u8; 16],
}

/// Build the client->relay header for this token and RCC endpoint.
pub fn client_header(kind: u8, token: &[u8; 16], rcc: (Ipv4Addr, u16)) -> Vec<u8> {
    let mut h = Vec::with_capacity(31);
    h.extend_from_slice(&[1, 0, 0, 0]);
    h.extend_from_slice(&[TLV_TOKEN, 17, kind]);
    h.extend_from_slice(token);
    h.extend_from_slice(&[TLV_IPV4, 6]);
    h.extend_from_slice(&rcc.0.octets());
    h.extend_from_slice(&rcc.1.to_be_bytes());
    let n = h.len() as u16;
    h[2..4].copy_from_slice(&n.to_be_bytes());
    h
}

/// Parse a relay datagram: returns (header length, the token TLV if any).
pub fn parse_header(d: &[u8]) -> Option<(usize, Option<(u8, [u8; 16])>)> {
    if d.len() < 4 || d[0] != 1 {
        return None;
    }
    let hl = u16::from_be_bytes([d[2], d[3]]) as usize;
    if hl < 4 || hl > d.len() {
        return None;
    }
    let mut tok = None;
    let mut o = 4;
    while o + 2 <= hl {
        let (t, l) = (d[o], d[o + 1] as usize);
        let v = d.get(o + 2..o + 2 + l)?;
        if t == TLV_TOKEN && l == 17 {
            let mut k = [0u8; 16];
            k.copy_from_slice(&v[1..]);
            tok = Some((v[0], k));
        }
        o += 2 + l;
    }
    Some((hl, tok))
}

pub struct RuppSocket {
    io: tokio::net::UdpSocket,
    relay: SocketAddr,
    rcc: (Ipv4Addr, u16),
    state: Mutex<TokenState>,
    /// Counters for diagnostics: datagrams in, datagrams out, token switches.
    pub stats: Mutex<(u64, u64, u64)>,
}

impl fmt::Debug for RuppSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuppSocket").field("relay", &self.relay).finish_non_exhaustive()
    }
}

impl RuppSocket {
    /// Bind an ephemeral UDP socket for a relay. Must run inside a tokio runtime.
    pub fn bind(relay: SocketAddr, rcc: (Ipv4Addr, u16), join_token: [u8; 16]) -> io::Result<Arc<RuppSocket>> {
        let std = std::net::UdpSocket::bind("0.0.0.0:0")?;
        std.set_nonblocking(true)?;
        Ok(Arc::new(RuppSocket {
            io: tokio::net::UdpSocket::from_std(std)?,
            relay,
            rcc,
            state: Mutex::new(TokenState { kind: TOKEN_TYPE_JOIN, token: join_token }),
            stats: Mutex::new((0, 0, 0)),
        }))
    }
}

#[derive(Debug)]
struct Poller(Arc<RuppSocket>);
impl UdpPoller for Poller {
    fn poll_writable(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        self.0.io.poll_send_ready(cx)
    }
}

impl AsyncUdpSocket for RuppSocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        Box::pin(Poller(self))
    }

    fn try_send(&self, t: &Transmit) -> io::Result<()> {
        let st = self.state.lock().unwrap().clone();
        let hdr = client_header(st.kind, &st.token, self.rcc);
        // one datagram per segment (max_transmit_segments is 1, but be safe)
        let seg = t.segment_size.unwrap_or(t.contents.len()).max(1);
        for chunk in t.contents.chunks(seg) {
            let mut buf = Vec::with_capacity(hdr.len() + chunk.len());
            buf.extend_from_slice(&hdr);
            buf.extend_from_slice(chunk);
            self.io.try_send_to(&buf, self.relay)?;
            self.stats.lock().unwrap().1 += 1;
        }
        Ok(())
    }

    fn poll_recv(
        &self,
        cx: &mut Context,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        let mut raw = [0u8; 2048];
        loop {
            let mut rb = tokio::io::ReadBuf::new(&mut raw);
            let from = match self.io.poll_recv_from(cx, &mut rb) {
                Poll::Ready(Ok(a)) => a,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            };
            let d = rb.filled();
            if from != self.relay {
                continue;
            }
            let Some((hl, tok)) = parse_header(d) else { continue };
            if let Some((kind, token)) = tok {
                let mut st = self.state.lock().unwrap();
                if st.kind != kind || st.token != token {
                    st.kind = kind;
                    st.token = token;
                    self.stats.lock().unwrap().2 += 1;
                }
            }
            let q = &d[hl..];
            if q.is_empty() || q.len() > bufs[0].len() {
                continue;
            }
            bufs[0][..q.len()].copy_from_slice(q);
            meta[0] = RecvMeta { addr: self.relay, len: q.len(), stride: q.len(), ecn: None, dst_ip: None };
            self.stats.lock().unwrap().0 += 1;
            return Poll::Ready(Ok(1));
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.local_addr()
    }

    fn may_fragment(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip_matches_capture_shape() {
        let h = client_header(1, &[7u8; 16], (Ipv4Addr::new(10, 26, 5, 150), 54991));
        // captured client->relay header length 31 = 0x1f, `01 11 01 <16B> 02 06 <ip> <port>`
        assert_eq!(h.len(), 31);
        assert_eq!(&h[..7], &[1, 0, 0, 31, 1, 17, 1]);
        assert_eq!(&h[23..25], &[2, 6]);
        assert_eq!(&h[25..29], &[10, 26, 5, 150]);
        assert_eq!(u16::from_be_bytes([h[29], h[30]]), 54991);
        let mut d = h.clone();
        d.extend_from_slice(b"quic");
        let (hl, tok) = parse_header(&d).unwrap();
        assert_eq!(hl, 31);
        assert_eq!(tok, Some((1, [7u8; 16])));
        // relay->client: 23 bytes, token type 2, no address
        let mut s = vec![1, 0, 0, 23, 1, 17, 2];
        s.extend_from_slice(&[9u8; 16]);
        assert_eq!(parse_header(&s).unwrap(), (23, Some((2, [9u8; 16]))));
    }
}
