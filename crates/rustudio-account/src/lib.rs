//! Roblox account layer for rustudio (docs/AUTH.md).
//!
//! One sign-in: the `.ROBLOSECURITY` session cookie, from the login keyring
//! (or imported from `~/.config/rustudio/roblox-cookie`). Signed in means
//! `users.roblox.com/v1/users/authenticated` accepted it. Over the session:
//! the profile, the avatar description, the account's experiences, asset
//! and place downloads, and state-changing calls with the X-CSRF-TOKEN
//! dance. Public assets also download anonymously ([`public_asset`]).
//!
//! An Open Cloud API key is still accepted for publishing only, until
//! Roblox takes the cookie for place uploads.

pub mod api;
pub mod assets_cloud;
pub mod avatar;
pub mod cloud;
pub mod config;
pub mod data;
pub mod mock;
pub mod quick_login;
pub mod session;
pub mod store;
pub mod web_social;

pub use api::{
    headshots, public_asset, Client, PlaceInfo, PublishResult, SignedOut, UniverseInfo, VersionType,
};
pub use cloud::{
    open_mode, parse_place_ref, CloudApi, CloudOrigin, CreatorKind, Endpoints, Experience, ExperienceSource, PlaceCache,
    OpenMode, SaveOutcome, TeamCreateStatus,
};
pub use config::{AccountConfig, Bookmark, SavedAccount};
pub use session::{Cookie, CookieSource, Profile, ProfileCache, Session};
pub use store::PublishKeyStore;

/// Errors produced by the account layer. Kept as plain strings so they can be
/// shown directly in the editor's Output panel; they never hold the cookie.
#[derive(Debug, Clone)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl Error {
    pub fn new(msg: impl Into<String>) -> Self {
        Error(msg.into())
    }
}

macro_rules! impl_from {
    ($($t:ty),*) => {$(
        impl From<$t> for Error {
            fn from(e: $t) -> Self { Error(e.to_string()) }
        }
    )*};
}
impl_from!(
    std::io::Error,
    ureq::Error,
    serde_json::Error,
    toml::de::Error,
    toml::ser::Error,
    keyring::Error
);

pub type Result<T> = std::result::Result<T, Error>;
