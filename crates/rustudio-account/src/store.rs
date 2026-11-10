//! The publishing key in the system keyring (Secret Service on Linux,
//! Credential Manager on Windows), and the entries the session cookie and
//! saved accounts use (see [`crate::session`]).
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

fn entry(user: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, user)
        .map_err(|e| Error(format!("system keyring unavailable: {e}")))
}

/// The secret stored for `user` under service `rustudio`, if any. Raw
/// bytes, so Windows (which stores a password as UTF-16) fits twice as much.
#[cfg(not(test))]
pub(crate) fn load(user: &str) -> Result<Option<Vec<u8>>> {
    match entry(user)?.get_secret() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(not(test))]
pub(crate) fn save(user: &str, secret: &[u8]) -> Result<()> {
    entry(user)?.set_secret(secret)?;
    Ok(())
}

#[cfg(not(test))]
pub(crate) fn delete(user: &str) -> Result<()> {
    match entry(user)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
thread_local! {
    /// Tests keep the entries in memory (per test thread) instead of the
    /// person's keyring.
    static MEMORY: std::cell::RefCell<std::collections::HashMap<String, Vec<u8>>> =
        Default::default();
}

#[cfg(test)]
pub(crate) fn load(user: &str) -> Result<Option<Vec<u8>>> {
    Ok(MEMORY.with(|m| m.borrow().get(user).cloned()))
}

#[cfg(test)]
pub(crate) fn save(user: &str, secret: &[u8]) -> Result<()> {
    MEMORY.with(|m| m.borrow_mut().insert(user.to_string(), secret.to_vec()));
    Ok(())
}

#[cfg(test)]
pub(crate) fn delete(user: &str) -> Result<()> {
    MEMORY.with(|m| m.borrow_mut().remove(user));
    Ok(())
}

pub struct PublishKeyStore;

impl PublishKeyStore {
    fn entry(user: &str) -> Result<keyring::Entry> {
        entry(user)
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
