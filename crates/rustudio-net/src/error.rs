//! Errors for the Team Create client. Messages are shown in the Output panel
//! and must never carry the cookie, a token/ticket or a key.

use std::fmt;

#[derive(Debug, Clone)]
pub enum NetError {
    /// A REST call came back with an unexpected status or body.
    Http { what: String, status: u16 },
    /// The join script was missing a field or could not be parsed.
    JoinScript(String),
    /// The matchmaker never reached a ready status within the poll budget.
    MatchmakerTimeout,
    /// The account cannot collaborate here (age gate, trusted connection,
    /// no edit permission). The reason is the server's `Error` enum value.
    CannotCollaborate(String),
    /// The transport is not available in this build.
    TransportUnavailable(String),
    /// Anything else, as plain text (no secrets).
    Other(String),
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::Http { what, status } => write!(f, "{what}: HTTP {status}"),
            NetError::JoinScript(m) => write!(f, "join script: {m}"),
            NetError::MatchmakerTimeout => f.write_str("matchmaker never became ready"),
            NetError::CannotCollaborate(e) => write!(f, "cannot collaborate: {e}"),
            NetError::TransportUnavailable(m) => write!(f, "transport unavailable: {m}"),
            NetError::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for NetError {}

impl From<rustudio_account::Error> for NetError {
    fn from(e: rustudio_account::Error) -> Self {
        NetError::Other(e.0)
    }
}
impl From<serde_json::Error> for NetError {
    fn from(e: serde_json::Error) -> Self {
        NetError::Other(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, NetError>;
