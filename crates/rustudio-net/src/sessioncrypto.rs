//! The `RbxTransportSessionCrypto` hello.
//!
//! Studio's `0xa8` message is a fixed two-field container [verified: exe,
//! verified: capture]:
//!
//! ```text
//! a8 11 <len1> <len1 bytes> <len2> <len2 bytes>
//! ```
//!
//! - `0xa8` is the packet id.
//! - `0x11` is the byte the object holds at (the encryption/key
//!   suite selector).
//! - Each length is one byte (the low byte of the `std::string` size in the
//!   builder); live Studio sends `len1 = 33` and `len2 = 66`, for 103 bytes
//!   total.
//!
//! The field *contents* are session-specific key material supplied by the
//! server in the join script's `ClientTicket` bundle. Studio splits that
//! bundle on `;`, Base64-decodes fields 3 and 4 (zero-based indices 2 and 3),
//! and parses the final field as the suite byte [verified: exe, parser
//! Base64 decoder]. No
//! client ephemeral key is generated for this message.
//!
//! This module never logs or formats the decoded bodies.

use crate::error::{NetError, Result};
use base64::Engine;

/// Packet id of the client SessionCrypto hello [verified: capture].
pub const HELLO_ID: u8 = 0xa8;
/// The suite byte Studio sends in this build [verified: capture].
pub const SUITE: u8 = 0x11;
/// Length of the first field in a live 0.746 hello [verified: capture].
pub const FIRST_LEN: usize = 33;
/// Length of the second field in a live 0.746 hello [verified: capture].
pub const SECOND_LEN: usize = 66;
/// Total encoded length of the reference capture: 3 header bytes + 33 + 1 + 66.
/// Other valid tickets can decode to different body lengths.
pub const WIRE_LEN: usize = 3 + FIRST_LEN + 1 + SECOND_LEN;

/// A SessionCrypto hello, with the bodies kept opaque.
#[derive(Clone, PartialEq, Eq)]
pub struct Hello {
    /// The byte at object (`0x11` on the live build).
    pub suite: u8,
    /// First length-prefixed, server-supplied ticket body.
    pub first: Vec<u8>,
    /// Second length-prefixed field.
    pub second: Vec<u8>,
}

impl Hello {
    pub fn new(first: Vec<u8>, second: Vec<u8>) -> Self {
        Hello {
            suite: SUITE,
            first,
            second,
        }
    }

    /// Build Studio's hello from the `ClientTicket` join field. The first two
    /// fields are not used by this builder; fields 3 and 4 hold the Base64
    /// bodies and the final field is a decimal suite number.
    pub fn from_client_ticket(ticket: &str) -> Result<Self> {
        let bad = |why: &str| NetError::Other(format!("sessioncrypto join bundle: {why}"));
        let fields: Vec<&str> = ticket.split(';').collect();
        if fields.len() < 5 {
            return Err(bad("fewer than five semicolon-separated fields"));
        }
        let decode = |s: &str| {
            base64::engine::general_purpose::STANDARD
                .decode(s)
                .map_err(|_| bad("invalid Base64 body"))
        };
        let suite = fields
            .last()
            .unwrap()
            .parse::<u8>()
            .map_err(|_| bad("invalid decimal suite"))?;
        let hello = Hello {
            suite,
            first: decode(fields[2])?,
            second: decode(fields[3])?,
        };
        // The one-byte wire lengths are a hard format limit, not merely a
        // property of the currently observed 33/66-byte suite.
        if hello.first.len() > u8::MAX as usize || hello.second.len() > u8::MAX as usize {
            return Err(bad("decoded body exceeds one-byte length"));
        }
        Ok(hello)
    }

    /// Encode the message in the builder's format. Unlike Studio's unchecked
    /// low-byte truncation, this rejects bodies too large for the one-byte
    /// prefix rather than emitting a malformed packet.
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.first.len() > u8::MAX as usize || self.second.len() > u8::MAX as usize {
            return Err(NetError::Other(
                "sessioncrypto field exceeds one-byte length".into(),
            ));
        }
        let mut v = Vec::with_capacity(3 + self.first.len() + 1 + self.second.len());
        v.push(HELLO_ID);
        v.push(self.suite);
        v.push(self.first.len() as u8);
        v.extend_from_slice(&self.first);
        v.push(self.second.len() as u8);
        v.extend_from_slice(&self.second);
        Ok(v)
    }

    /// Parse a full `0xa8` message. Rejects trailing bytes and wrong ids.
    pub fn decode(m: &[u8]) -> Result<Hello> {
        let bad = |why: &str| NetError::Other(format!("sessioncrypto hello: {why}"));
        if m.len() < 3 || m[0] != HELLO_ID {
            return Err(bad("not a 0xa8 message"));
        }
        let n1 = m[2] as usize;
        let p1: usize = 3;
        let n2_at = p1.checked_add(n1).ok_or_else(|| bad("length overflow"))?;
        if m.len() < n2_at + 1 {
            return Err(bad("truncated first field"));
        }
        let n2 = m[n2_at] as usize;
        let p2 = n2_at + 1;
        if m.len() != p2 + n2 {
            return Err(bad("truncated or trailing bytes"));
        }
        Ok(Hello {
            suite: m[1],
            first: m[p1..n2_at].to_vec(),
            second: m[p2..].to_vec(),
        })
    }

    /// Check against the suite observed in the reference capture. This is a
    /// compatibility assertion for our current client, not a server rule.
    pub fn check_suite(&self) -> Result<()> {
        if self.suite != SUITE {
            return Err(NetError::Other(format!(
                "sessioncrypto suite {:#x}, this build sends {SUITE:#x}",
                self.suite
            )));
        }
        Ok(())
    }
}

impl std::fmt::Debug for Hello {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the bodies.
        f.debug_struct("Hello")
            .field("suite", &self.suite)
            .field("first_len", &self.first.len())
            .field("second_len", &self.second.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_verified_live_shape() {
        let h = Hello::new(vec![0x11; FIRST_LEN], vec![0x22; SECOND_LEN]);
        let w = h.encode().unwrap();
        assert_eq!(w.len(), WIRE_LEN);
        assert_eq!(&w[..3], &[0xa8, 0x11, 33]);
        assert_eq!(w[3 + 33], 66);
        assert_eq!(Hello::decode(&w).unwrap(), h);
    }

    #[test]
    fn rejects_malformed_bodies() {
        assert!(Hello::decode(&[]).is_err());
        assert!(Hello::decode(&[0xa8, 0x11, 1]).is_err());
        let mut w = Hello::new(vec![1, 2, 3], vec![4, 5]).encode().unwrap();
        w.push(0);
        assert!(Hello::decode(&w).is_err());
    }

    #[test]
    fn decodes_the_join_bundle_like_studio() {
        use base64::engine::general_purpose::STANDARD;
        let first = vec![0x51; FIRST_LEN];
        let second = vec![0xa2; SECOND_LEN];
        let bundle = format!(
            "ignored-zero;ignored-one;{};{};ignored-four;17",
            STANDARD.encode(&first),
            STANDARD.encode(&second)
        );
        let h = Hello::from_client_ticket(&bundle).unwrap();
        assert_eq!(h.suite, SUITE);
        assert_eq!(h.first, first);
        assert_eq!(h.second, second);
        assert_eq!(h.encode().unwrap().len(), WIRE_LEN);
    }

    #[test]
    fn accepts_exactly_five_fields_and_other_body_lengths() {
        use base64::engine::general_purpose::STANDARD;
        let ticket = format!(
            "a;b;{};{};17",
            STANDARD.encode([7; 32]),
            STANDARD.encode([8; 64])
        );
        let h = Hello::from_client_ticket(&ticket).unwrap();
        assert_eq!(h.first.len(), 32);
        assert_eq!(h.second.len(), 64);
        assert_eq!(h.encode().unwrap().len(), 100);
    }

    #[test]
    fn rejects_malformed_join_bundles() {
        assert!(Hello::from_client_ticket("a;b;c;d").is_err());
        assert!(Hello::from_client_ticket("a;b;!;ZA==;17").is_err());
        assert!(Hello::from_client_ticket("a;b;WA==;!;17").is_err());
        assert!(Hello::from_client_ticket("a;b;WA==;Wg==;no").is_err());
        assert!(Hello::from_client_ticket("a;b;WA==;Wg==;256").is_err());
    }

    #[test]
    fn rejects_wrong_ids_and_oversized_encodes() {
        assert!(Hello::decode(&[0xa7, 0x11, 0]).is_err());
        assert!(Hello::new(vec![0; 256], vec![]).encode().is_err());
        assert!(Hello::new(vec![], vec![0; 256]).encode().is_err());
    }

    #[test]
    fn debug_does_not_show_bodies() {
        let h = Hello::new(vec![0xab; FIRST_LEN], vec![0xcd; SECOND_LEN]);
        assert_eq!(
            format!("{h:?}"),
            "Hello { suite: 17, first_len: 33, second_len: 66 }"
        );
    }
}
