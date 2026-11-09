//! rustudio Team Create live-session client.
//!
//! This crate carries the network half of "Open a Team Create place from
//! Roblox and join the live session" (see the app's `docs/TEAMCREATE.md` and this
//! crate's `docs/PROTOCOL.md`). It is split so each layer can be built and tested
//! on its own:
//!
//! - [`joinscript`] — parse the join script that `gamejoin /v1/team-create`
//!   returns (server address/port, UDMUX relays, tokens, the server's
//!   ephemeral public key).
//! - [`matchmaker`] — build and (deliberately) run the REST calls that open a
//!   session: `GetCanCollaborate`, the `/v1/team-create` matchmaker POST and
//!   its polling. These change server state, so nothing here is called unless
//!   a caller asks for it explicitly.
//! - [`transport`] — authenticated RbxTransport over QUIC and RUPP relay framing.
//! - [`live`] — the background live session and outgoing edit queue.
//! - [`scripts`] — native atomic script replacements with stale-base protection.
//! - [`replicator`] — the CloudEdit replicator item vocabulary and the live
//!   [`model::LiveModel`] a client builds from the stream.
//!
//! ## Secrets
//!
//! The session cookie, the join tokens/tickets and the session keys never go
//! to a log, a file or any `Debug` output. Token-bearing fields use
//! [`Redacted`], whose `Debug`/`Display` print `<redacted>`.

pub mod error;
pub mod handshake;
pub mod joinscript;
pub mod live;
pub mod matchmaker;
pub mod model;
pub mod replicator;
pub mod rtcio;
pub mod rupp;
pub mod scripts;
pub mod sessioncrypto;
pub mod transport;
pub mod writer;

pub use error::NetError;
pub use joinscript::{JoinScript, ServerEndpoint, TransportChoice};

use std::fmt;

/// A value that must never be printed: tokens, tickets, keys. It compares and
/// is usable in memory, but its `Debug`/`Display` are always `<redacted>`.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Redacted(String);

impl Redacted {
    pub fn new(s: impl Into<String>) -> Self {
        Redacted(s.into())
    }
    /// The underlying value. Callers must not log or persist it.
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}
impl fmt::Display for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Join a Team Create place using the stored account session. Calling this
/// explicitly starts matchmaking and the background QUIC connection.
pub fn open_team_create(universe_id: u64, place_id: u64) -> Result<live::LiveSession, NetError> {
    live::LiveSession::connect(universe_id, place_id)
}
