//! The publishing key in the system keyring (Secret Service on Linux).
//!
//! The session cookie is the only sign-in (docs/AUTH.md). This Open Cloud API key
//! is kept only for publishing places, because Roblox doesn't accept the
//! cookie for uploading a place version yet (docs/MECHANICS.md, "Account"); it
//! goes once publishing over the cookie works.

use crate::{Error, Result};

const SERVICE: &str = "rustudio";
const API_KEY_USER: &str = "roblox-open-cloud-api-key";
/// Left behind by OAuth sign-in before the cookie session; removed on sight.
const OLD_OAUTH_USER: &str = "roblox-oauth-tokens";

pub struct PublishKeyStore;

impl PublishKeyStore {
    fn entry(user: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, user)
            .map_err(|e| Error(format!("system keyring unavailable: {e}")))
    }

    pub fn save(key: &str) -> Result<()> {
        Self::entry(API_KEY_USER)?.set_password(key.trim())?;
        Ok(())
    }

    /// The saved publishing key, if any.
    pub fn load() -> Result<Option<String>> {
        match Self::entry(API_KEY_USER)?.get_password() {
            Ok(s) => Ok(Some(s).filter(|k| !k.trim().is_empty())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn clear() -> Result<()> {
        match Self::entry(API_KEY_USER)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Deletes the OAuth tokens an older rustudio saved.
    pub fn forget_oauth_tokens() {
        if let Ok(e) = Self::entry(OLD_OAUTH_USER) {
            let _ = e.delete_credential();
        }
    }
}
