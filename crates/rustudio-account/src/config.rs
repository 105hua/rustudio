//! `~/.config/rustudio/account.toml`: saved experience/place bookmarks and
//! whether the person signed out. No secret ever lives here: the session
//! cookie is in the login keyring (see [`crate::session`]) and the
//! publishing key in the system keyring (see [`crate::store`]).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::Result;

/// A universe/place pair to show even when the account's own list doesn't
/// have it (places of groups, places shared with the account).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Bookmark {
    pub universe_id: u64,
    pub place_id: u64,
    #[serde(default)]
    pub name: String,
}

/// An account signed in on this computer before (account switching). Only
/// the public profile is here; its session is in the login keyring.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SavedAccount {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountConfig {
    pub bookmarks: Vec<Bookmark>,
    /// Set by Sign out: the cookie file isn't imported again until the
    /// person signs in.
    pub signed_out: bool,
    /// Accounts to switch between, most recently used first.
    pub accounts: Vec<SavedAccount>,
}

impl AccountConfig {
    pub fn path() -> PathBuf {
        if let Ok(p) = std::env::var("RUSTUDIO_ACCOUNT_CONFIG") {
            return PathBuf::from(p);
        }
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rustudio")
            .join("account.toml")
    }

    /// Loads the config. A file from before the cookie session (with an
    /// `[oauth]` table: client id, scopes) is rewritten without it.
    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            let cfg = AccountConfig::default();
            cfg.save()?;
            return Ok(cfg);
        }
        let text = std::fs::read_to_string(&path)?;
        let cfg: AccountConfig = toml::from_str(&text)?;
        if text.contains("[oauth]") || text.contains("avatar_username") {
            cfg.save()?;
        }
        Ok(cfg)
    }

    /// Puts the profile first in the account list.
    pub fn remember(&mut self, p: &crate::Profile) {
        self.accounts.retain(|a| a.id != p.id);
        self.accounts.insert(
            0,
            SavedAccount {
                id: p.id,
                name: p.name.clone(),
                display_name: p.display_name.clone(),
            },
        );
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = toml::to_string_pretty(self)?;
        let text = format!(
            "# rustudio account settings. See docs/AUTH.md. The Roblox session itself is\n\
             # in the login keyring (service rustudio, user roblox-cookie), never here.\n\n{body}"
        );
        std::fs::write(path, text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_oauth_settings_are_dropped() {
        let old = "[oauth]\nclient_id = \"123\"\nredirect_port = 51917\nscopes = \"openid\"\n\n[[bookmarks]]\nuniverse_id = 1\nplace_id = 2\n";
        let cfg: AccountConfig = toml::from_str(old).unwrap();
        assert_eq!(cfg.bookmarks.len(), 1);
        let back = toml::to_string_pretty(&cfg).unwrap();
        assert!(!back.contains("client_id") && !back.contains("oauth"));
    }
}
