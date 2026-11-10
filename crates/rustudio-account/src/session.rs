//! The Roblox session: the `.ROBLOSECURITY` cookie is the one credential
//! (docs/AUTH.md). Signed in means `users.roblox.com/v1/users/authenticated`
//! accepted it; anything else is "signed out".
//!
//! Where the cookie comes from: the login keyring (the `keyring` crate,
//! service `rustudio`, user `roblox-cookie`), else `~/.config/rustudio/roblox-cookie`
//! (mode 600, one line, written by hand), which is then imported into the
//! keyring. The value lives only in memory and in those two places: it is
//! never formatted into a message, log line, error or file. [`Cookie`]'s
//! Debug and Display print `<redacted>`.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

pub const AUTHENTICATED_URL: &str = "https://users.roblox.com/v1/users/authenticated";
pub const MY_AVATAR_URL: &str = "https://avatar.roblox.com/v1/avatar";
pub const MY_UNIVERSES_URL: &str = "https://develop.roblox.com/v1/user/universes";
pub const UPLOAD_URL: &str = "https://data.roblox.com/Data/Upload.ashx";
/// The Roblox place the editor has open (0: none), sent with asset requests.
pub static PLACE_CONTEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Sets the place context for asset delivery (see `download_asset`).
pub fn set_place_context(place_id: u64) {
    PLACE_CONTEXT.store(place_id, std::sync::atomic::Ordering::Relaxed);
}

pub const ASSET_V2_URL: &str = "https://assetdelivery.roblox.com/v2/assetId";

const KEYRING_SERVICE: &str = "rustudio";
const KEYRING_KEY: &str = "roblox-cookie";

/// Retries of a CSRF-refused request (Studio: counter at 
///).
pub const CSRF_MAX_RETRIES: usize = 3;

/// Cookie names Studio keeps as secrets:
/// none of them is ever shown.
pub const SECRET_COOKIES: [&str; 3] = [".ROBLOSECURITY", ".RBXIDCHECK", ".RBXID"];

/// Shown for any cookie Roblox doesn't accept (never the value itself).
pub const SIGNED_OUT: &str = "signed out";

/// The session cookie. Never printed.
#[derive(Clone)]
pub struct Cookie(String);

impl std::fmt::Debug for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl std::fmt::Display for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl Cookie {
    /// From the raw line (with or without the `.ROBLOSECURITY=` prefix).
    pub fn new(raw: &str) -> Option<Cookie> {
        let v = raw.lines().next()?.trim();
        let v = v.strip_prefix(".ROBLOSECURITY=").unwrap_or(v).trim();
        (!v.is_empty()).then(|| Cookie(v.to_string()))
    }

    pub(crate) fn header(&self) -> String {
        format!(".ROBLOSECURITY={}", self.0)
    }

    /// A one-way key for caches tied to this cookie (a cookie change
    /// invalidates them). Not reversible to the value.
    pub fn cache_key(&self) -> String {
        let mut h = Sha256::new();
        h.update(b"rustudio-session-cache-v1\0");
        h.update(self.0.as_bytes());
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Whether `text` contains the cookie (to keep it out of anything shown).
    pub fn appears_in(&self, text: &str) -> bool {
        self.0.len() >= 8 && text.contains(&self.0)
    }
}

/// `~/.config/rustudio/roblox-cookie`.
pub fn cookie_file() -> PathBuf {
    if let Ok(p) = std::env::var("RUSTUDIO_COOKIE_FILE") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rustudio")
        .join("roblox-cookie")
}

/// Where a loaded cookie came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieSource {
    Keyring,
    /// Read from the import file (and copied into the keyring if it could be).
    File {
        imported: bool,
    },
}

fn keyring_disabled() -> bool {
    std::env::var_os("RUSTUDIO_NO_KEYRING").is_some()
}

/// Sessions older builds stored with `secret-tool` (attributes `service
/// rustudio key roblox-cookie`, plus `user <id>` for saved accounts). The
/// keyring crate looks items up by `service` and `username`, so it doesn't
/// see them. One is read once, moved into the keyring and then removed.
#[cfg(all(unix, not(target_os = "macos"), not(test)))]
mod legacy {
    use super::Cookie;
    use std::process::{Command, Stdio};

    pub fn lookup(attrs: &[&str]) -> Option<Cookie> {
        let out = Command::new("secret-tool")
            .arg("lookup")
            .args(attrs)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Cookie::new(&String::from_utf8_lossy(&out.stdout))
    }

    pub fn clear(attrs: &[&str]) {
        let _ = Command::new("secret-tool")
            .arg("clear")
            .args(attrs)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Nothing was ever stored with `secret-tool` here.
#[cfg(not(any(all(unix, not(target_os = "macos")), test)))]
mod legacy {
    use super::Cookie;

    pub fn lookup(_attrs: &[&str]) -> Option<Cookie> {
        None
    }

    pub fn clear(_attrs: &[&str]) {}
}

/// Tests stand in for `secret-tool` with a map (per test thread).
#[cfg(test)]
mod legacy {
    use super::Cookie;
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        pub static ITEMS: RefCell<HashMap<String, String>> = Default::default();
    }

    pub fn lookup(attrs: &[&str]) -> Option<Cookie> {
        ITEMS.with(|m| {
            m.borrow()
                .get(&attrs.join(" "))
                .and_then(|v| Cookie::new(v))
        })
    }

    pub fn clear(attrs: &[&str]) {
        ITEMS.with(|m| m.borrow_mut().remove(&attrs.join(" ")));
    }
}

/// Reads a session from the keyring (entry `user` under service
/// `rustudio`), else from where `secret-tool` kept it before.
fn secret_lookup(user: &str, legacy_attrs: &[&str]) -> Option<Cookie> {
    if keyring_disabled() {
        return None;
    }
    if let Ok(Some(v)) = crate::store::load(user) {
        return Cookie::new(&String::from_utf8_lossy(&v));
    }
    let c = legacy::lookup(legacy_attrs)?;
    if crate::store::save(user, c.0.as_bytes()).is_ok() {
        legacy::clear(legacy_attrs);
    }
    Some(c)
}

fn secret_store(user: &str, c: &Cookie) -> Result<()> {
    if keyring_disabled() {
        return Err(Error::new("keyring disabled (RUSTUDIO_NO_KEYRING)"));
    }
    crate::store::save(user, c.0.as_bytes())
        .map_err(|_| Error::new("the login keyring refused to store the session"))
}

/// Removes the keyring entry and any copy `secret-tool` kept, so an old one
/// can't come back after sign-out.
fn secret_clear(user: &str, legacy_attrs: &[&str]) {
    if keyring_disabled() {
        return;
    }
    let _ = crate::store::delete(user);
    legacy::clear(legacy_attrs);
}

const LEGACY_COOKIE: [&str; 4] = ["service", KEYRING_SERVICE, "key", KEYRING_KEY];

/// The keyring copy (service `rustudio`, user `roblox-cookie`).
pub fn keyring_cookie() -> Option<Cookie> {
    secret_lookup(KEYRING_KEY, &LEGACY_COOKIE)
}

/// Stores the cookie in the keyring as the active session.
pub fn keyring_store(c: &Cookie) -> Result<()> {
    secret_store(KEYRING_KEY, c)
}

/// Removes the keyring copy (sign out). The import file is the person's and
/// is left alone.
pub fn keyring_clear() -> Result<()> {
    secret_clear(KEYRING_KEY, &LEGACY_COOKIE);
    Ok(())
}

/// Saved accounts (account switching): each signed-in account's session is
/// kept under user `roblox-account-<id>`, next to the active one. Switching
/// copies a saved session into the active slot.
const ACCOUNT_KEY: &str = "roblox-account";

fn account_user(user_id: u64) -> String {
    format!("{ACCOUNT_KEY}-{user_id}")
}

pub fn account_cookie(user_id: u64) -> Option<Cookie> {
    let id = user_id.to_string();
    secret_lookup(
        &account_user(user_id),
        &["service", KEYRING_SERVICE, "key", ACCOUNT_KEY, "user", &id],
    )
}

pub fn account_store(user_id: u64, c: &Cookie) -> Result<()> {
    secret_store(&account_user(user_id), c)
}

pub fn account_forget(user_id: u64) {
    let id = user_id.to_string();
    secret_clear(
        &account_user(user_id),
        &["service", KEYRING_SERVICE, "key", ACCOUNT_KEY, "user", &id],
    );
}

/// Next to the cookie file: the one-way key of the file's cookie once it has
/// been imported, so a file left in place doesn't take the active slot back
/// after a switch. A new value pasted into the file still wins.
fn file_marker() -> PathBuf {
    let mut p = cookie_file().into_os_string();
    p.push(".imported");
    PathBuf::from(p)
}

fn file_seen(c: &Cookie) -> bool {
    std::fs::read_to_string(file_marker()).is_ok_and(|s| s.trim() == c.cache_key())
}

fn mark_seen(c: &Cookie) {
    let _ = std::fs::write(file_marker(), c.cache_key());
}

/// Marks whatever the cookie file holds now as already imported (before a
/// switch, so the file doesn't switch back).
pub fn mark_file_seen() {
    if let Ok(Some(f)) = file_cookie() {
        mark_seen(&f);
    }
}

/// The import file's cookie (only if it is not readable by others).
pub fn file_cookie() -> Result<Option<Cookie>> {
    let path = cookie_file();
    let Ok(meta) = std::fs::metadata(&path) else {
        return Ok(None);
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(Error(format!(
                "{} is readable by other users; chmod 600 it",
                path.display()
            )));
        }
    }
    #[cfg(not(unix))]
    let _ = meta;
    let mut s = String::new();
    std::fs::File::open(&path)?.read_to_string(&mut s)?;
    Ok(Cookie::new(&s))
}

/// The cookie to use: the keyring, else the import file (copied into the
/// keyring). `allow_file` false skips the file (after an explicit sign-out).
/// A cookie file that differs from the keyring copy wins once: it is the
/// person pasting a fresh session after the old one expired.
pub fn load_cookie(allow_file: bool) -> Result<Option<(Cookie, CookieSource)>> {
    let keyring = keyring_cookie();
    if !allow_file {
        return Ok(keyring.map(|c| (c, CookieSource::Keyring)));
    }
    let file = match file_cookie() {
        Ok(f) => f,
        Err(e) if keyring.is_none() => return Err(e),
        Err(_) => None,
    };
    match (keyring, file) {
        (Some(k), Some(f)) if k.0 == f.0 || file_seen(&f) => Ok(Some((k, CookieSource::Keyring))),
        (_, Some(f)) => {
            let imported = keyring_store(&f).is_ok();
            mark_seen(&f);
            Ok(Some((f, CookieSource::File { imported })))
        }
        (Some(k), None) => Ok(Some((k, CookieSource::Keyring))),
        (None, None) => Ok(None),
    }
}

/// Who the session belongs to (users.roblox.com/v1/users/authenticated).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub id: u64,
    pub name: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
}

impl Profile {
    pub fn display(&self) -> String {
        if self.display_name.is_empty() || self.display_name == self.name {
            format!("@{}", self.name)
        } else {
            format!("{} (@{})", self.display_name, self.name)
        }
    }
}

/// Display cache, keyed to the cookie: `~/.cache/rustudio/session.json`
/// holds the profile and avatar JSON of the last session and the cookie's
/// one-way key; any other cookie misses it. It is never what decides
/// "signed in".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileCache {
    pub key: String,
    pub profile: Profile,
    #[serde(default)]
    pub avatar: Option<serde_json::Value>,
}

pub fn cache_path() -> PathBuf {
    if let Ok(p) = std::env::var("RUSTUDIO_SESSION_CACHE") {
        return PathBuf::from(p);
    }
    let base = std::env::var("RUSTUDIO_CACHE_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("rustudio")
        });
    base.join("session.json")
}

impl ProfileCache {
    /// The cache entry for this cookie, if it is the one cached.
    pub fn load(c: &Cookie) -> Option<ProfileCache> {
        let text = std::fs::read_to_string(cache_path()).ok()?;
        let p: ProfileCache = serde_json::from_str(&text).ok()?;
        (p.key == c.cache_key()).then_some(p)
    }

    pub fn save(&self) -> Result<()> {
        let path = cache_path();
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn clear() {
        let _ = std::fs::remove_file(cache_path());
    }
}

/// HTTP agent for cookie calls: a browser-like user agent (Cloudflare in
/// front of these hosts turns other clients away) and statuses as values.
pub fn session_agent() -> ureq::Agent {
    // Redirects are reported, not followed: a cookie call that is sent to
    // a login page is a refusal, and a POST must not be replayed elsewhere.
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .user_agent(concat!(
            "Mozilla/5.0 (X11; Linux x86_64) rustudio/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .into()
}

/// A validated session.
pub struct Session {
    cookie: Cookie,
    agent: ureq::Agent,
    /// X-CSRF-TOKEN from the last 403, reused until the session rotates it.
    csrf: Mutex<Option<String>>,
    pub profile: Profile,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

/// A reply body read as text (bounded), with its status.
fn text_of(mut r: ureq::http::Response<ureq::Body>) -> Result<(u16, String)> {
    let status = r.status().as_u16();
    let mut s = String::new();
    r.body_mut()
        .with_config()
        .limit(16 << 20)
        .reader()
        .read_to_string(&mut s)?;
    Ok((status, s))
}

impl Session {
    /// Asks Roblox who the cookie belongs to. Err(SIGNED_OUT…) when it isn't
    /// accepted; the cookie never appears in the error.
    pub fn validate(cookie: Cookie) -> Result<Session> {
        let agent = session_agent();
        let r = agent
            .get(AUTHENTICATED_URL)
            .header("Cookie", &cookie.header())
            .call()
            .map_err(|e| Error(format!("{SIGNED_OUT} (could not reach Roblox: {e})")))?;
        let (status, text) = text_of(r)?;
        if status != 200 {
            return Err(Error(format!(
                "{SIGNED_OUT} (Roblox did not accept the saved session: HTTP {status})"
            )));
        }
        let profile: Profile = serde_json::from_str(&text)
            .map_err(|_| Error(format!("{SIGNED_OUT} (unexpected reply from Roblox)")))?;
        Ok(Session {
            cookie,
            agent,
            csrf: Mutex::new(None),
            profile,
        })
    }

    /// A copy for downloads on other threads, so they don't queue on the
    /// account's lock (it shares the connection pool; no CSRF token).
    pub fn downloader(&self) -> Session {
        Session {
            cookie: self.cookie.clone(),
            agent: self.agent.clone(),
            csrf: Mutex::new(None),
            profile: self.profile.clone(),
        }
    }

    /// A session that was not checked with Roblox, for talking to a local
    /// mock server (tests and the self-test's save checks). Never used for
    /// a real cookie: nothing about the profile is verified.
    pub fn unchecked(cookie: Cookie, profile: Profile) -> Session {
        Session {
            cookie,
            agent: session_agent(),
            csrf: Mutex::new(None),
            profile,
        }
    }

    pub fn cookie(&self) -> &Cookie {
        &self.cookie
    }

    /// GET with the session.
    pub fn get(&self, url: &str) -> Result<(u16, String)> {
        let r = self
            .agent
            .get(url)
            .header("Cookie", &self.cookie.header())
            .call()?;
        text_of(r)
    }

    /// GET with the session, also returning one reply header (asset
    /// delivery's `roblox-assetversionnumber`).
    pub fn get_with_header(&self, url: &str, header: &str) -> Result<(u16, Option<String>, String)> {
        let r = self
            .agent
            .get(url)
            .header("Cookie", &self.cookie.header())
            .call()?;
        let h = r
            .headers()
            .get(header)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let (status, text) = text_of(r)?;
        Ok((status, h, text))
    }

    /// GET returning bytes, gzip bodies inflated; the cookie only when asked
    /// (CDN locations and thumbnails don't want it).
    pub fn fetch_bytes(&self, url: &str, with_cookie: bool) -> Result<(u16, Vec<u8>)> {
        self.get_bytes(url, with_cookie)
    }

    /// GET returning bytes (assets), gzip bodies inflated.
    fn get_bytes(&self, url: &str, with_cookie: bool) -> Result<(u16, Vec<u8>)> {
        let mut req = self.agent.get(url).header("Accept-Encoding", "gzip");
        if with_cookie {
            req = req.header("Cookie", &self.cookie.header());
            let place = PLACE_CONTEXT.load(std::sync::atomic::Ordering::Relaxed);
            if place != 0 {
                req = req.header("Roblox-Place-Id", &place.to_string());
            }
        }
        let mut r = req.call()?;
        let status = r.status().as_u16();
        let mut bytes = Vec::new();
        r.body_mut()
            .with_config()
            .limit(1 << 30)
            .reader()
            .read_to_end(&mut bytes)?;
        if bytes.starts_with(&[0x1f, 0x8b]) {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(&bytes[..]).read_to_end(&mut out)?;
            bytes = out;
        }
        Ok((status, bytes))
    }

    /// Engine data requests over the login session, preserving object metadata
    /// response headers. Credentials and response tokens are never returned.
    pub fn engine_data_request(&self, method: &str, url: &str, body: &[u8], extra: &[(&str, &str)]) -> Result<(u16, Vec<(String, String)>, String)> {
        if !matches!(method, "GET" | "POST" | "DELETE") { return Err(Error::new("unsupported engine data HTTP method")); }
        for _ in 0..=CSRF_MAX_RETRIES {
            let token = self.csrf.lock().ok().and_then(|g| g.clone());
            let mut request = ureq::http::Request::builder().method(method).uri(url)
                .header("Cookie", self.cookie.header());
            for (k,v) in extra { request = request.header(*k,*v); }
            if method != "GET" { if let Some(token) = &token { request = request.header("X-CSRF-TOKEN", token); } }
            let request = request.body(body).map_err(|_|Error::new("invalid engine data HTTP request"))?;
            let response = self.agent.run(request)?;
            let fresh = response.headers().get("x-csrf-token").and_then(|v|v.to_str().ok()).map(str::to_owned);
            let headers = response.headers().iter().filter(|(k,_)| k.as_str().starts_with("roblox-object-") || k.as_str()=="retry-after" || k.as_str()=="content-type" || k.as_str()=="etag" || k.as_str()=="usn" || k.as_str()=="content-md5")
                .filter_map(|(k,v)|v.to_str().ok().map(|v|(k.as_str().to_owned(),v.to_owned()))).collect();
            let (status, text) = text_of(response)?;
            if method != "GET" && status == 403 {
                if let Some(fresh) = fresh.filter(|t|!t.is_empty() && token.as_deref()!=Some(t.as_str())) {
                    if let Ok(mut held)=self.csrf.lock(){ *held=Some(fresh); } continue;
                }
            }
            return Ok((status,headers,text));
        }
        Err(Error::new("Roblox kept refusing the CSRF token"))
    }

    /// The CSRF token held now (tests).
    pub fn csrf_token_held(&self) -> bool {
        self.csrf.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// A state-changing POST with the X-CSRF-TOKEN dance, Studio's rule
    /// a reply of 403 to a method other
    /// than GET / HEAD / OPTIONS / TRACE that carries a non-empty
    /// `x-csrf-token` different from the one sent is retried with it, at most
    /// [`CSRF_MAX_RETRIES`] times. The token is kept for the session.
    pub fn post_csrf(&self, url: &str, content_type: &str, body: &[u8]) -> Result<(u16, String)> {
        self.post_csrf_with(url, content_type, body, &[])
    }

    /// [`Session::post_csrf`] with extra request headers (Team Create's
    /// `robloxctx-authenticated-userid`, Studio's user agent).
    pub fn post_csrf_with(
        &self,
        url: &str,
        content_type: &str,
        body: &[u8],
        extra: &[(&str, &str)],
    ) -> Result<(u16, String)> {
        for _ in 0..=CSRF_MAX_RETRIES {
            let token = self.csrf.lock().ok().and_then(|g| g.clone());
            let mut req = self
                .agent
                .post(url)
                .header("Cookie", &self.cookie.header())
                .header("Content-Type", content_type);
            for (k, v) in extra {
                req = req.header(*k, *v);
            }
            if let Some(t) = &token {
                req = req.header("X-CSRF-TOKEN", t);
            }
            let r = req.send(body)?;
            let fresh = r
                .headers()
                .get("x-csrf-token")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let (status, text) = text_of(r)?;
            match fresh {
                Some(t)
                    if status == 403 && !t.is_empty() && token.as_deref() != Some(t.as_str()) =>
                {
                    if let Ok(mut g) = self.csrf.lock() {
                        *g = Some(t);
                    }
                    continue;
                }
                _ => return Ok((status, text)),
            }
        }
        Err(Error::new("Roblox kept refusing the CSRF token"))
    }

    /// The signed-in person's avatar description (avatar.roblox.com/v1/avatar).
    pub fn avatar_json(&self) -> Result<serde_json::Value> {
        let (status, text) = self.get(MY_AVATAR_URL)?;
        if status != 200 {
            return Err(Error(format!(
                "avatar.roblox.com/v1/avatar returned HTTP {status}"
            )));
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// An asset's bytes through asset delivery v2 with the session (assets
    /// served only to signed-in accounts included), then its CDN location.
    pub fn download_asset(&self, asset_id: u64, version: Option<u64>) -> Result<Vec<u8>> {
        let url = match version {
            Some(v) => format!("{ASSET_V2_URL}/{asset_id}/version/{v}"),
            None => format!("{ASSET_V2_URL}/{asset_id}"),
        };
        // Studio asks asset delivery in the open place's context
        // (`Roblox-Place-Id`): assets the place's creator owns (private
        // animations, audio) are then served to its collaborators.
        let place = PLACE_CONTEXT.load(std::sync::atomic::Ordering::Relaxed);
        let mut req = self.agent.get(&url).header("Cookie", &self.cookie.header());
        if place != 0 {
            req = req.header("Roblox-Place-Id", &place.to_string());
        }
        let (status, text) = text_of(req.call()?)?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
            Error(format!(
                "asset {asset_id}: asset delivery returned HTTP {status}"
            ))
        })?;
        let Some(location) = v.pointer("/locations/0/location").and_then(|l| l.as_str()) else {
            let why = v
                .pointer("/errors/0/message")
                .and_then(|m| m.as_str())
                .unwrap_or("no download location");
            return Err(Error(format!(
                "asset {asset_id}: asset delivery: {why} (HTTP {status})"
            )));
        };
        let (status, bytes) = self.get_bytes(location, false)?;
        if !(200..300).contains(&status) {
            return Err(Error(format!(
                "asset {asset_id}: download returned HTTP {status}"
            )));
        }
        Ok(bytes)
    }

    /// Experiences this account owns (develop.roblox.com/v1/user/universes;
    /// `limit` must be 10, 25, 50 or 100).
    pub fn my_universe_ids(&self) -> Result<Vec<(u64, String)>> {
        let mut out = Vec::new();
        let mut cursor = String::new();
        loop {
            let mut url = format!("{MY_UNIVERSES_URL}?limit=50&sortOrder=Asc");
            if !cursor.is_empty() {
                url.push_str(&format!("&cursor={cursor}"));
            }
            let (status, text) = self.get(&url)?;
            if status != 200 {
                return Err(Error(format!(
                    "develop.roblox.com/v1/user/universes returned HTTP {status}"
                )));
            }
            let v: serde_json::Value = serde_json::from_str(&text)?;
            for u in v
                .get("data")
                .and_then(|d| d.as_array())
                .into_iter()
                .flatten()
            {
                if let Some(id) = u.get("id").and_then(|i| i.as_u64()) {
                    let name = u
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    out.push((id, name));
                }
            }
            match v.get("nextPageCursor").and_then(|c| c.as_str()) {
                Some(c) if !c.is_empty() => cursor = c.to_string(),
                _ => break,
            }
        }
        Ok(out)
    }

    /// Uploads a place file as a new version of `place_id`:
    /// `POST data.roblox.com/Data/Upload.ashx?assetid={placeId}` with the
    /// session and the CSRF dance. Returns Roblox's reply text (the new
    /// version's id).
    pub fn upload_place(&self, place_id: u64, bytes: &[u8], is_xml: bool) -> Result<String> {
        let url = format!("{UPLOAD_URL}?assetid={place_id}");
        let ct = if is_xml {
            "application/xml"
        } else {
            "application/octet-stream"
        };
        let (status, text) = self.post_csrf(&url, ct, bytes)?;
        if !(200..300).contains(&status) {
            let detail: String = text.chars().take(200).collect();
            // A reply that echoes the cookie or names a secret cookie is not shown.
            let detail = if self.cookie.appears_in(&detail)
                || SECRET_COOKIES.iter().any(|c| detail.contains(c))
            {
                String::new()
            } else {
                detail
            };
            return Err(Error(format!(
                "Upload.ashx answered HTTP {status}: {}",
                detail.trim()
            )));
        }
        Ok(text.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_never_prints() {
        let c = Cookie::new(".ROBLOSECURITY=_|WARNING:-secret-value-123\n").unwrap();
        assert_eq!(format!("{c}"), "<redacted>");
        assert_eq!(format!("{c:?}"), "<redacted>");
        assert!(!c.cache_key().contains("secret"));
        assert_eq!(c.cache_key().len(), 64);
        assert!(c.appears_in("x _|WARNING:-secret-value-123 y"));
        let other = Cookie::new("another-value-456").unwrap();
        assert_ne!(c.cache_key(), other.cache_key());
    }

    #[test]
    fn empty_lines_are_no_cookie() {
        assert!(Cookie::new("").is_none());
        assert!(Cookie::new("   \n").is_none());
        assert!(Cookie::new(".ROBLOSECURITY=").is_none());
    }

    fn legacy_put(attrs: &[&str], v: &str) {
        legacy::ITEMS.with(|m| m.borrow_mut().insert(attrs.join(" "), v.to_string()));
    }

    fn legacy_has(attrs: &[&str]) -> bool {
        legacy::ITEMS.with(|m| m.borrow().contains_key(&attrs.join(" ")))
    }

    fn stored(user: &str) -> Option<String> {
        crate::store::load(user)
            .unwrap()
            .map(|v| String::from_utf8(v).unwrap())
    }

    #[test]
    fn session_round_trips_through_the_keyring() {
        assert!(keyring_cookie().is_none());
        keyring_store(&Cookie::new("cookie-value-1").unwrap()).unwrap();
        assert_eq!(stored("roblox-cookie").as_deref(), Some("cookie-value-1"));
        assert_eq!(keyring_cookie().unwrap().0, "cookie-value-1");
        keyring_clear().unwrap();
        assert!(keyring_cookie().is_none());
        assert!(stored("roblox-cookie").is_none());
    }

    #[test]
    fn secret_tool_session_moves_into_the_keyring() {
        legacy_put(
            &["service", "rustudio", "key", "roblox-cookie"],
            "old-cookie-value\n",
        );
        assert_eq!(keyring_cookie().unwrap().0, "old-cookie-value");
        assert_eq!(stored("roblox-cookie").as_deref(), Some("old-cookie-value"));
        assert!(!legacy_has(&LEGACY_COOKIE));
        assert_eq!(keyring_cookie().unwrap().0, "old-cookie-value");
    }

    #[test]
    fn keyring_copy_wins_over_secret_tool() {
        keyring_store(&Cookie::new("new-cookie-value").unwrap()).unwrap();
        legacy_put(&LEGACY_COOKIE, "old-cookie-value");
        assert_eq!(keyring_cookie().unwrap().0, "new-cookie-value");
    }

    #[test]
    fn sign_out_clears_the_secret_tool_copy_too() {
        keyring_store(&Cookie::new("new-cookie-value").unwrap()).unwrap();
        legacy_put(&LEGACY_COOKIE, "old-cookie-value");
        keyring_clear().unwrap();
        assert!(!legacy_has(&LEGACY_COOKIE));
        assert!(keyring_cookie().is_none());
    }

    #[test]
    fn saved_accounts_have_their_own_entries() {
        let old = ["service", "rustudio", "key", "roblox-account", "user", "42"];
        legacy_put(&old, "account-42-cookie");
        assert_eq!(account_cookie(42).unwrap().0, "account-42-cookie");
        assert_eq!(
            stored("roblox-account-42").as_deref(),
            Some("account-42-cookie")
        );
        assert!(!legacy_has(&old));
        account_store(7, &Cookie::new("account-7-cookie").unwrap()).unwrap();
        assert_eq!(account_cookie(7).unwrap().0, "account-7-cookie");
        assert!(keyring_cookie().is_none());
        account_forget(42);
        assert!(account_cookie(42).is_none());
        assert_eq!(account_cookie(7).unwrap().0, "account-7-cookie");
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Live probe (network + the person's cookie; `cargo test -- --ignored`).
    /// Prints statuses and the profile name only.
    #[test]
    #[ignore]
    fn probe_live_session() {
        let Some((c, src)) = load_cookie(true).expect("cookie") else {
            println!("no cookie");
            return;
        };
        println!("cookie from {src:?}");
        let s = match Session::validate(c) {
            Ok(s) => s,
            Err(e) => {
                println!("{e}");
                return;
            }
        };
        println!("signed in as {} (id {})", s.profile.display(), s.profile.id);
        match s.avatar_json() {
            Ok(v) => println!(
                "avatar type {:?} scales {:?}",
                v.get("playerAvatarType"),
                v.get("scales")
            ),
            Err(e) => println!("avatar: {e}"),
        }
        match s.my_universe_ids() {
            Ok(u) => println!(
                "{} owned experiences: {:?}",
                u.len(),
                u.iter().map(|x| &x.1).collect::<Vec<_>>()
            ),
            Err(e) => println!("universes: {e}"),
        }
        for id in [123480233606534u64, 76523574292282] {
            match s.download_asset(id, None) {
                Ok(b) => println!(
                    "asset {id}: {} bytes, starts {:?}",
                    b.len(),
                    String::from_utf8_lossy(&b[..b.len().min(12)])
                ),
                Err(e) => println!("asset {id}: {e}"),
            }
        }
        // CSRF: assetid 0 is no place, so nothing can be written.
        match s.post_csrf(
            &format!("{UPLOAD_URL}?assetid=0"),
            "application/octet-stream",
            b"",
        ) {
            Ok((st, t)) => println!(
                "Upload.ashx assetid=0: HTTP {st}, token held {}, reply {:?}",
                s.csrf_token_held(),
                t.chars().take(300).collect::<String>()
            ),
            Err(e) => println!("upload probe: {e}"),
        }
    }
}

#[cfg(test)]
mod live_assets {
    use super::*;

    /// Saves login-only assets to /tmp/rs-auth (RUSTUDIO_FETCH=id,id,…) and
    /// prints the avatar JSON's top-level keys.
    #[test]
    #[ignore]
    fn fetch_login_assets() {
        let Some((c, _)) = load_cookie(true).expect("cookie") else {
            return;
        };
        let s = Session::validate(c).expect("session");
        let _ = std::fs::create_dir_all("/tmp/rs-auth");
        for id in std::env::var("RUSTUDIO_FETCH")
            .unwrap_or_default()
            .split(',')
            .filter_map(|x| x.parse::<u64>().ok())
        {
            match s.download_asset(id, None) {
                Ok(b) => {
                    std::fs::write(format!("/tmp/rs-auth/{id}"), &b).unwrap();
                    println!("{id}: {} bytes", b.len());
                }
                Err(e) => println!("{id}: {e}"),
            }
        }
        if let Ok(v) = s.avatar_json() {
            println!(
                "avatar keys {:?}",
                v.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>())
            );
            println!("bodyColors {:?}", v.get("bodyColors"));
            println!(
                "assets {:?}",
                v.get("assets")
                    .map(|a| a.to_string().chars().take(600).collect::<String>())
            );
        }
    }
}
