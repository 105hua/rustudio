//! Avatar web requests for AvatarEditorService (avatar_aes.luau): each one
//! runs on a worker thread with the signed-in session (the same endpoints
//! Studio calls, found in Studio; checks/parity/avatar.md) and
//! the calling script polls it from Luau, so the *Async methods yield on the
//! task scheduler like Studio's. Read-only: GETs, and the catalog's
//! read-only batch-details POST. Signed out, a request fails the way an
//! offline Studio's does ("HTTP 401 (Unauthorized)").

use std::collections::HashMap;
use std::sync::Mutex;

type Reply = Result<(u16, String), String>;

struct Jobs {
    next: u64,
    done: HashMap<u64, Option<Reply>>,
}

static JOBS: std::sync::LazyLock<Mutex<Jobs>> =
    std::sync::LazyLock::new(|| Mutex::new(Jobs { next: 1, done: HashMap::new() }));

/// The signed-in session, waiting briefly for a check still under way.
fn session() -> Option<rustudio_account::session::Session> {
    if !crate::assets::wait_for_session(std::time::Duration::from_secs(20)) {
        return None;
    }
    let client = crate::assets::session_client()?;
    let g = client.lock().ok()?;
    g.as_ref().map(|c| c.session.downloader())
}

/// The signed-in user's id (0 signed out).
pub fn user_id() -> u64 {
    crate::assets::session_client()
        .and_then(|c| c.lock().ok().and_then(|g| g.as_ref().map(|c| c.user_id())))
        .unwrap_or(0)
}

fn reason(status: u16) -> &'static str {
    ureq::http::StatusCode::from_u16(status)
        .ok()
        .and_then(|s| s.canonical_reason())
        .unwrap_or("Error")
}

/// "HTTP 404 (Not Found)", Studio's text for a failed web call.
pub fn http_error(status: u16) -> String {
    format!("HTTP {status} ({})", reason(status))
}

/// Starts a request; poll it with [`poll`].
pub fn start(method: &str, url: String, body: Option<String>) -> u64 {
    let id = {
        let mut j = JOBS.lock().unwrap_or_else(|e| e.into_inner());
        let id = j.next;
        j.next += 1;
        j.done.insert(id, None);
        id
    };
    let post = method.eq_ignore_ascii_case("POST");
    std::thread::spawn(move || {
        let r: Reply = match session() {
            None => Err(http_error(401)),
            Some(s) => {
                let r = if post {
                    s.post_csrf(&url, "application/json", body.unwrap_or_default().as_bytes())
                } else {
                    s.get(&url)
                };
                r.map_err(|e| e.to_string())
            }
        };
        if let Ok(mut j) = JOBS.lock() {
            j.done.insert(id, Some(r));
        }
    });
    id
}

/// None while running; the reply once (then forgotten).
pub fn poll(id: u64) -> Option<Reply> {
    let mut j = JOBS.lock().unwrap_or_else(|e| e.into_inner());
    match j.done.get(&id) {
        Some(Some(_)) => j.done.remove(&id).flatten(),
        Some(None) => None,
        None => Some(Err("request not found".into())),
    }
}

type Bytes = Result<Vec<u8>, String>;

static ASSETS: std::sync::LazyLock<Mutex<(u64, HashMap<u64, Option<Bytes>>)>> =
    std::sync::LazyLock::new(|| Mutex::new((1, HashMap::new())));

/// Fetches an asset's bytes the way Studio's avatar loader does: the
/// batch asset API (`POST assetdelivery.roblox.com/v1/assets/batch`, a
/// read-only lookup) with an optional `assetFormat` (Studio asks for
/// `avatar_meshpart_head` for heads, which turns a DynamicHead into its
/// MeshPart head), then the content URL it answers with. Disk-cached per
/// (id, format). Poll with [`poll_asset`].
pub fn start_asset(id: u64, format: Option<String>) -> u64 {
    let job = {
        let mut g = ASSETS.lock().unwrap_or_else(|e| e.into_inner());
        let j = g.0;
        g.0 += 1;
        g.1.insert(j, None);
        j
    };
    std::thread::spawn(move || {
        let cache = crate::assets::cache_dir().join(format!(
            "assets/{id}_{}.bin",
            format.as_deref().unwrap_or("vlatest")
        ));
        let r: Bytes = match std::fs::read(&cache) {
            Ok(b) if !b.is_empty() => Ok(b),
            _ => (|| {
                if format.is_none() {
                    return crate::assets::remote_cached(id).map(|b| (*b).clone());
                }
                let s = session().ok_or_else(|| http_error(401))?;
                let body = serde_json::json!([{ "assetId": id, "requestId": "0", "assetFormat": format }]).to_string();
                let (status, text) = s
                    .post_csrf("https://assetdelivery.roblox.com/v1/assets/batch", "application/json", body.as_bytes())
                    .map_err(|e| e.to_string())?;
                if status != 200 {
                    return Err(http_error(status));
                }
                let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                let loc = v
                    .pointer("/0/location")
                    .and_then(|l| l.as_str())
                    .ok_or_else(|| {
                        v.pointer("/0/errors/0/message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("asset not available")
                            .to_string()
                    })?;
                let (status, bytes) = s.fetch_bytes(loc, false).map_err(|e| e.to_string())?;
                if status != 200 {
                    return Err(http_error(status));
                }
                if let Some(dir) = cache.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&cache, &bytes);
                Ok(bytes)
            })(),
        };
        if let Ok(mut g) = ASSETS.lock() {
            g.1.insert(job, Some(r));
        }
    });
    job
}

pub fn poll_asset(job: u64) -> Option<Bytes> {
    let mut g = ASSETS.lock().unwrap_or_else(|e| e.into_inner());
    match g.1.get(&job) {
        Some(Some(_)) => g.1.remove(&job).flatten(),
        Some(None) => None,
        None => Some(Err("request not found".into())),
    }
}
