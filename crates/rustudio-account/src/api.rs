//! Account calls over the session: who is signed in, the experiences they
//! own, downloading places and assets, and publishing a place version.

use std::io::Read;

use serde::Serialize;

use crate::config::AccountConfig;
use crate::session::{self, Cookie, CookieSource, ProfileCache, Session};
use crate::store::PublishKeyStore;
use crate::{Error, Result};

/// Open Cloud Place Publishing API, API key only (scope
/// `universe-places:write`). Kept for publishing until Roblox accepts the
/// session cookie for place uploads (docs/AUTH.md).
pub const PUBLISH_URL: &str = "https://apis.roblox.com/universes/v1";
/// Public develop endpoints used for experience and place names.
pub const DEVELOP_URL: &str = "https://develop.roblox.com/v1/universes";

/// Public asset delivery (no authentication; see docs/MATERIALS.md): replies
/// with JSON `locations`, each a CDN URL whose body is often gzip.
pub const PUBLIC_ASSET_DELIVERY_URL: &str = "https://assetdelivery.roblox.com/v2/assetId";

/// Downloads a public asset anonymously: GET the v2 descriptor, then its
/// first location; gzip bodies are inflated. No cookie, key or token is
/// sent, so only assets Roblox serves to anyone come back.
pub fn public_asset(asset_id: u64) -> Result<Vec<u8>> {
    let a = agent();
    let url = format!("{PUBLIC_ASSET_DELIVERY_URL}/{asset_id}");
    let mut r = a.get(&url).call()?;
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string()?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
        Error(format!(
            "asset {asset_id}: public asset delivery returned HTTP {status}"
        ))
    })?;
    let location = v
        .get("locations")
        .and_then(|l| l.get(0))
        .and_then(|l| l.get("location"))
        .and_then(|l| l.as_str());
    let Some(location) = location else {
        let why = v
            .pointer("/errors/0/message")
            .and_then(|m| m.as_str())
            .unwrap_or("no download location");
        return Err(Error(format!(
            "asset {asset_id}: public asset delivery: {why} (HTTP {status})"
        )));
    };
    let mut r = a.get(location).header("Accept-Encoding", "gzip").call()?;
    if !r.status().is_success() {
        return Err(Error(format!(
            "asset {asset_id}: download returned HTTP {}",
            r.status().as_u16()
        )));
    }
    let mut bytes = Vec::new();
    r.body_mut()
        .with_config()
        .limit(256 << 20)
        .reader()
        .read_to_end(&mut bytes)?;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&bytes[..]).read_to_end(&mut out)?;
        bytes = out;
    }
    Ok(bytes)
}

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .user_agent(concat!("rustudio/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionType {
    Saved,
    Published,
}

impl VersionType {
    pub fn as_str(self) -> &'static str {
        match self {
            VersionType::Saved => "Saved",
            VersionType::Published => "Published",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaceInfo {
    pub id: u64,
    pub name: String,
    pub is_root: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UniverseInfo {
    pub id: u64,
    pub name: String,
    pub root_place_id: Option<u64>,
    pub places: Vec<PlaceInfo>,
}

#[derive(Debug, Clone)]
pub struct PublishResult {
    pub version_number: u64,
    /// "publishing key" or "session".
    pub via: &'static str,
}

/// Why there is no session (always shown as "signed out").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignedOut {
    /// No cookie in the keyring or the import file (or the person signed out).
    NoCookie,
    /// Roblox didn't accept the cookie, or couldn't be asked.
    Rejected(String),
}

impl SignedOut {
    pub fn describe(&self) -> String {
        match self {
            SignedOut::NoCookie => format!(
                "signed out (no session: put the .ROBLOSECURITY value in {} with mode 600)",
                session::cookie_file().display()
            ),
            SignedOut::Rejected(why) => why.clone(),
        }
    }
}

/// The signed-in account: one session cookie, validated with Roblox.
pub struct Client {
    pub config: AccountConfig,
    pub session: Session,
    pub source: CookieSource,
}

impl Client {
    /// Opens the session: the cookie from the keyring (or the import file
    /// unless the person signed out), validated against
    /// users.roblox.com/v1/users/authenticated. The profile and the avatar
    /// description are cached for display, keyed to the cookie.
    pub fn open(config: AccountConfig) -> std::result::Result<Client, SignedOut> {
        let loaded = session::load_cookie(!config.signed_out)
            .map_err(|e| SignedOut::Rejected(format!("{} ({e})", session::SIGNED_OUT)))?;
        let Some((cookie, source)) = loaded else {
            return Err(SignedOut::NoCookie);
        };
        Self::open_with(config, cookie, source)
    }

    /// Validates a given cookie (tests use a made-up one).
    pub fn open_with(
        config: AccountConfig,
        cookie: Cookie,
        source: CookieSource,
    ) -> std::result::Result<Client, SignedOut> {
        let session = Session::validate(cookie).map_err(|e| SignedOut::Rejected(e.0))?;
        let avatar = session.avatar_json().ok();
        let _ = ProfileCache {
            key: session.cookie().cache_key(),
            profile: session.profile.clone(),
            avatar,
        }
        .save();
        Ok(Client {
            config,
            session,
            source,
        })
    }

    /// The display cache of the cookie the keyring holds now, if it is the
    /// cached one (shown while the session is being checked).
    pub fn cached_profile() -> Option<ProfileCache> {
        ProfileCache::load(&session::keyring_cookie()?)
    }

    pub fn who(&self) -> String {
        self.session.profile.display()
    }

    pub fn user_id(&self) -> u64 {
        self.session.profile.id
    }

    /// The signed-in person's avatar description (avatar.roblox.com/v1/avatar),
    /// from the cache when it belongs to this cookie.
    pub fn avatar_json(&self) -> Result<serde_json::Value> {
        if let Some(v) = ProfileCache::load(self.session.cookie()).and_then(|c| c.avatar) {
            return Ok(v);
        }
        self.session.avatar_json()
    }

    /// The experiences the account owns, plus bookmarked ones.
    pub fn list_universes(&self) -> Result<Vec<UniverseInfo>> {
        let mut ids: Vec<u64> = self
            .session
            .my_universe_ids()?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        for b in &self.config.bookmarks {
            if !ids.contains(&b.universe_id) {
                ids.push(b.universe_id);
            }
        }
        let mut out = Vec::new();
        for id in ids {
            out.push(self.universe(id)?);
        }
        Ok(out)
    }

    /// Name and places of one universe (develop endpoints, with the session).
    pub fn universe(&self, universe_id: u64) -> Result<UniverseInfo> {
        let mut info = UniverseInfo {
            id: universe_id,
            name: format!("Experience {universe_id}"),
            root_place_id: None,
            places: Vec::new(),
        };
        let (status, text) = self.session.get(&format!("{DEVELOP_URL}/{universe_id}"))?;
        if status == 200 {
            let v: serde_json::Value = serde_json::from_str(&text)?;
            if let Some(n) = v.get("name").and_then(|v| v.as_str()) {
                info.name = n.to_string();
            }
            info.root_place_id = v.get("rootPlaceId").and_then(|v| v.as_u64());
        }
        let mut cursor = String::new();
        loop {
            let mut url = format!("{DEVELOP_URL}/{universe_id}/places?limit=100&sortOrder=Asc");
            if !cursor.is_empty() {
                url.push_str(&format!("&cursor={cursor}"));
            }
            let (status, text) = self.session.get(&url)?;
            if status != 200 {
                break;
            }
            let v: serde_json::Value = serde_json::from_str(&text)?;
            for p in v
                .get("data")
                .and_then(|d| d.as_array())
                .into_iter()
                .flatten()
            {
                let Some(id) = p.get("id").and_then(|v| v.as_u64()) else {
                    continue;
                };
                info.places.push(PlaceInfo {
                    id,
                    name: p
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Place")
                        .to_string(),
                    is_root: Some(id) == info.root_place_id,
                });
            }
            match v.get("nextPageCursor").and_then(|c| c.as_str()) {
                Some(c) if !c.is_empty() => cursor = c.to_string(),
                _ => break,
            }
        }
        if info.places.is_empty() {
            if let Some(root) = info.root_place_id {
                info.places.push(PlaceInfo {
                    id: root,
                    name: info.name.clone(),
                    is_root: true,
                });
            }
        }
        for b in self
            .config
            .bookmarks
            .iter()
            .filter(|b| b.universe_id == universe_id)
        {
            if !info.places.iter().any(|p| p.id == b.place_id) {
                info.places.push(PlaceInfo {
                    id: b.place_id,
                    name: if b.name.is_empty() {
                        format!("Place {}", b.place_id)
                    } else {
                        b.name.clone()
                    },
                    is_root: Some(b.place_id) == info.root_place_id,
                });
            }
        }
        Ok(info)
    }

    /// The latest version of a place (`.rbxl` bytes) through asset delivery
    /// with the session.
    pub fn download_place(&self, place_id: u64) -> Result<Vec<u8>> {
        self.session.download_asset(place_id, None)
    }

    /// Any asset the account may read (meshes, images, clothing, accessory
    /// and dynamic-head models) through asset delivery with the session.
    pub fn download_asset(&self, asset_id: u64, version: Option<u64>) -> Result<Vec<u8>> {
        self.session.download_asset(asset_id, version)
    }

    /// Whether a publishing key is saved.
    pub fn has_publish_key() -> bool {
        PublishKeyStore::load().ok().flatten().is_some()
    }

    /// Saves or publishes the place. With the publishing key: Open Cloud
    /// Place Publishing (Saved or Published). Without it: the session's
    /// place upload (Data/Upload.ashx with the CSRF token), which Roblox
    /// currently refuses (docs/MECHANICS.md, "Account"); its answer is reported.
    pub fn publish_place(
        &self,
        universe_id: u64,
        place_id: u64,
        version_type: VersionType,
        rbxl: &[u8],
        is_xml: bool,
    ) -> Result<PublishResult> {
        let Some(key) = PublishKeyStore::load()? else {
            let reply = self.session.upload_place(place_id, rbxl, is_xml)?;
            return Ok(PublishResult {
                version_number: reply.trim().parse().unwrap_or(0),
                via: "session",
            });
        };
        let url = format!(
            "{PUBLISH_URL}/{universe_id}/places/{place_id}/versions?versionType={}",
            version_type.as_str()
        );
        let content_type = if is_xml {
            "application/xml"
        } else {
            "application/octet-stream"
        };
        let mut r = agent()
            .post(&url)
            .header("x-api-key", &key)
            .header("Content-Type", content_type)
            .send(rbxl)?;
        let status = r.status().as_u16();
        let text = r.body_mut().read_to_string()?;
        if status >= 400 {
            let detail: String = text.chars().take(300).collect();
            return Err(Error(format!(
                "Place Publishing (publishing key) answered HTTP {status}: {}",
                detail.trim()
            )));
        }
        let version_number = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("versionNumber").and_then(|n| n.as_u64()))
            .or_else(|| text.trim().trim_matches('"').parse().ok())
            .unwrap_or(0);
        Ok(PublishResult {
            version_number,
            via: "publishing key",
        })
    }

    /// Keeps this session as a saved account (account switching).
    pub fn remember_account(&self) -> Result<()> {
        session::account_store(self.session.profile.id, self.session.cookie())
    }

    /// Signs in with a pasted `.ROBLOSECURITY` value: validated first, then
    /// made the active session and saved for switching.
    pub fn sign_in_with(config: AccountConfig, raw: &str) -> std::result::Result<Client, SignedOut> {
        let cookie = Cookie::new(raw).ok_or(SignedOut::NoCookie)?;
        let c = Self::open_with(config, cookie, CookieSource::Keyring)?;
        session::mark_file_seen();
        session::keyring_store(c.session.cookie())
            .map_err(|e| SignedOut::Rejected(format!("signed in, but {e}")))?;
        let _ = c.remember_account();
        Ok(c)
    }

    /// Switches to a saved account: its session becomes the active one and
    /// is checked with Roblox.
    pub fn switch_to(config: AccountConfig, user_id: u64) -> std::result::Result<Client, SignedOut> {
        let cookie = session::account_cookie(user_id).ok_or_else(|| {
            SignedOut::Rejected("that account's saved session is gone from the login keyring; sign in to it again".into())
        })?;
        let c = Self::open_with(config, cookie, CookieSource::Keyring)?;
        if c.session.profile.id != user_id {
            return Err(SignedOut::Rejected("the saved session belongs to another account".into()));
        }
        session::mark_file_seen();
        session::keyring_store(c.session.cookie())
            .map_err(|e| SignedOut::Rejected(format!("signed in, but {e}")))?;
        Ok(c)
    }

    /// Removes a saved account (its keyring entry).
    pub fn forget_account(user_id: u64) {
        session::account_forget(user_id);
    }

    /// Signs out: the keyring copy and the display cache go, and the import
    /// file isn't read again until the person signs in.
    pub fn sign_out(mut config: AccountConfig) -> Result<()> {
        session::keyring_clear()?;
        ProfileCache::clear();
        config.signed_out = true;
        config.save()
    }
}

/// Avatar headshots (thumbnails `v1/users/avatar-headshot`, 150x150 PNG) for
/// the account list. Public: no cookie is sent.
pub fn headshots(user_ids: &[u64]) -> Result<Vec<(u64, Vec<u8>)>> {
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    let agent = session::session_agent();
    let ids: Vec<String> = user_ids.iter().map(|i| i.to_string()).collect();
    let url = format!(
        "https://thumbnails.roblox.com/v1/users/avatar-headshot?userIds={}&size=150x150&format=Png&isCircular=false",
        ids.join(",")
    );
    let text = agent.get(&url).call()?.body_mut().read_to_string()?;
    let v: serde_json::Value = serde_json::from_str(&text)?;
    let mut out = Vec::new();
    for t in v.get("data").and_then(|d| d.as_array()).into_iter().flatten() {
        let (Some(id), Some(img)) = (
            t.get("targetId").and_then(|x| x.as_u64()),
            t.get("imageUrl").and_then(|x| x.as_str()),
        ) else {
            continue;
        };
        if let Ok(mut r) = agent.get(img).call() {
            if r.status().as_u16() == 200 {
                if let Ok(bytes) = r.body_mut().with_config().limit(4 << 20).read_to_vec() {
                    out.push((id, bytes));
                }
            }
        }
    }
    Ok(out)
}
