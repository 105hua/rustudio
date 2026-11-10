//! Assets for meshes, textures, decals and clothing: resolving content ids,
//! downloading cloud assets through the account crate (Open Cloud Asset
//! Delivery only), a disk cache, parsing, and GPU upload caches.
//!
//! Sources:
//! * `rbxassetid://N`, `rbxassetid://N?version=V`, `…/asset/?id=N`, or a bare
//!   number: a cloud asset, read from the disk cache or downloaded (needs a
//!   signed-in account or API key with `legacy-asset:manage`).
//! * `rbxasset://path`: inside the content folder the person chooses
//!   (RUSTUDIO_CONTENT_DIR, else ~/.local/share/rustudio/content).
//! * `file://` URLs and plain paths (relative ones next to the open file).

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use glam::{Vec2, Vec3};

use crate::mesh;
use crate::texture::{self, Rgba};

// Shared by render, audio and script preload requests. 0 = None, 1 = Loading,
// 2 = Success, 3 = Failure; keys use one canonical asset URI.
static CONTENT_STATUS: std::sync::LazyLock<Mutex<HashMap<String, u8>>> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
fn content_key(id: &str) -> String {
    match asset_ref(id) { Some(a) => format!("rbxassetid://{}{}",a.id,a.version.map(|v|format!("?version={v}")).unwrap_or_default()), None => id.to_string() }
}
/// Counts every change of a content status: who polls statuses (the
/// AssetFetchStatus signals, domains/assets_cloud.rs) looks again only when
/// this moved.
static CONTENT_STATUS_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
pub fn content_status_generation() -> u64 {
    CONTENT_STATUS_GEN.load(std::sync::atomic::Ordering::Relaxed)
}
pub fn content_fetch_status(id: &str) -> u8 {
    if id.is_empty() {return 3}
    CONTENT_STATUS.lock().ok().and_then(|s|s.get(&content_key(id)).copied()).unwrap_or(0)
}
pub fn content_queue_size() -> usize {
    CONTENT_STATUS.lock().map(|s|s.values().filter(|v|**v==1).count()).unwrap_or(0)
}
static CONTENT_FAILURES: Mutex<(u64,Vec<(u64,String)>)> = Mutex::new((0,Vec::new()));
pub fn content_failures_since(cursor: u64) -> (u64,Vec<String>) {
    let f=CONTENT_FAILURES.lock().unwrap_or_else(|e|e.into_inner());
    (f.0,f.1.iter().filter(|(seq,_)|*seq>cursor).map(|(_,id)|id.clone()).collect())
}
pub fn content_track(id: &str, status: u8) {
    if let Ok(mut s)=CONTENT_STATUS.lock(){
        let old=s.insert(content_key(id),status);
        if old!=Some(status) {
            CONTENT_STATUS_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if status==3 && old!=Some(3) && asset_ref(id).is_some() {
            let mut f=CONTENT_FAILURES.lock().unwrap_or_else(|e|e.into_inner());
            f.0+=1;let seq=f.0;f.1.push((seq,id.to_string()));
            if f.1.len()>256 {f.1.remove(0);}
        }
    }
}
fn track_fetch(id: &str, result: &Fetch) {
    content_track(id,match result {Fetch::Ready(_)=>2,Fetch::Pending=>1,Fetch::Unavailable(_)=>3});
}

pub type SharedClient = Arc<Mutex<Option<rustudio_account::Client>>>;

pub fn content_dir() -> PathBuf {
    if let Ok(p) = std::env::var("RUSTUDIO_CONTENT_DIR") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let link = crate::paths::data().join("rustudio/content");
    if !link.exists() {
        // No link to a Studio install: the copy rustudio downloaded.
        if let Some(d) = crate::studio_content::installed() {
            return d.join("content");
        }
    }
    link
}

/// The asset cache folder: RUSTUDIO_CACHE_DIR, else ~/.cache/rustudio.
pub fn cache_dir() -> PathBuf {
    if let Ok(p) = std::env::var("RUSTUDIO_CACHE_DIR") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    crate::paths::cache().join("rustudio")
}

/// A local asset store: `<id>` or `<id>.<ext>` files, e.g. the one the
/// offline Studio serves (~/.local/share/rustudio/assets). Read only.
pub fn local_assets_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RUSTUDIO_LOCAL_ASSETS") {
        return (!p.is_empty()).then(|| PathBuf::from(p));
    }
    let p = crate::paths::data().join("rustudio/assets");
    p.is_dir().then_some(p)
}

fn local_asset(id: u64) -> Option<Vec<u8>> {
    local_asset_path(id).and_then(|p| std::fs::read(p).ok())
}

/// The file behind an asset id on this machine, looked up the way an offline Studio
/// does: its asset folder, then the rbxasset upload ledger
/// (~/.local/share/rbxasset/ledger.json: your uploads → their original files).
pub fn local_asset_path(id: u64) -> Option<PathBuf> {
    if let Some(dir) = local_assets_dir() {
        for ext in [
            "",
            ".mesh",
            ".png",
            ".jpg",
            ".dds",
            ".wav",
            ".ogg",
            ".mp3",
            ".webm",
            ".rbxm",
            ".rbxmx",
            ".anim.rbxm",
            ".decal.rbxmx",
            ".fbx",
        ] {
            let p = dir.join(format!("{id}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    ledger_path(id)
}

/// An upload's source file on this machine: rbxasset's upload ledger
/// (`path` per entry), else rustudio's own `asset-sources.json`
/// (`{"<asset id>": "<file>"}`, ~/.local/share/rustudio/) for uploads whose
/// recorded source is gone (a /tmp upload copy) but an equivalent file
/// exists. Needed because Roblox refuses another creator's private models to
/// a collaborator's session (a live Team Create join), so the place's own
/// InsertService:LoadAsset packs can only come from disk.
fn ledger_path(id: u64) -> Option<PathBuf> {
    let data = crate::paths::data();
    let ledger = std::env::var("RUSTUDIO_ASSET_LEDGER")
        .map(PathBuf::from)
        .unwrap_or_else(|_| data.join("rbxasset/ledger.json"));
    let sources = std::env::var("RUSTUDIO_ASSET_SOURCES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| data.join("rustudio/asset-sources.json"));
    static LEDGER: Mutex<Option<(std::time::SystemTime, HashMap<u64, PathBuf>)>> = Mutex::new(None);
    static SOURCES: Mutex<Option<(std::time::SystemTime, HashMap<u64, PathBuf>)>> = Mutex::new(None);
    path_map(&LEDGER, &ledger, id)
        .or_else(|| path_map(&SOURCES, &sources, id))
}

/// One id → file map from a JSON file, reloaded when the file changes. Takes
/// either the ledger's shape (objects with `assetId` + `path`) or plain
/// `"id": "path"` pairs. Only files that exist count.
fn path_map(
    cache: &Mutex<Option<(std::time::SystemTime, HashMap<u64, PathBuf>)>>,
    file: &std::path::Path,
    id: u64,
) -> Option<PathBuf> {
    let mtime = std::fs::metadata(file).and_then(|m| m.modified()).ok()?;
    let mut guard = cache.lock().ok()?;
    if guard.as_ref().is_none_or(|(t, _)| *t != mtime) {
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
        let mut map = HashMap::new();
        if let Some(obj) = json.as_object() {
            for (k, v) in obj {
                if let Some(path) = v.as_str() {
                    if let Ok(aid) = k.parse() {
                        map.insert(aid, PathBuf::from(path));
                    }
                    continue;
                }
                let aid = v.get("assetId").and_then(|a| {
                    a.as_u64()
                        .or_else(|| a.as_str().and_then(|s| s.parse().ok()))
                });
                if let (Some(aid), Some(path)) = (aid, v.get("path").and_then(|p| p.as_str())) {
                    map.insert(aid, PathBuf::from(path));
                }
            }
        }
        *guard = Some((mtime, map));
    }
    guard.as_ref()?.1.get(&id).filter(|p| p.is_file()).cloned()
}

/// Content made while the program runs (meshes and textures from imported
/// FBX models), under `rbxtemp://rustudio/...` ids.
static GENERATED: Mutex<Option<HashMap<String, Arc<Vec<u8>>>>> = Mutex::new(None);

pub fn register_generated(id: &str, bytes: Vec<u8>) {
    if let Ok(mut g) = GENERATED.lock() {
        g.get_or_insert_with(HashMap::new)
            .insert(id.to_string(), Arc::new(bytes));
    }
}

/// Release content whose owning Play DataModel has ended.
pub fn unregister_generated(id:&str){
    if let Ok(mut generated)=GENERATED.lock(){if let Some(generated)=generated.as_mut(){generated.remove(id);}}
}

pub fn generated(id: &str) -> Option<Arc<Vec<u8>>> {
    if !id.starts_with("rbxtemp://rustudio/") {
        return None;
    }
    GENERATED.lock().ok()?.as_ref()?.get(id).cloned()
}

/// A cloud asset reference: id and optional version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssetRef {
    pub id: u64,
    pub version: Option<u64>,
}

/// Parses the asset id forms Roblox content uses.
pub fn asset_ref(content: &str) -> Option<AssetRef> {
    let s = content.trim();
    let num = |t: &str| -> Option<u64> {
        t.chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .ok()
    };
    let version = |t: &str| t.find("version=").and_then(|i| num(&t[i + 8..]));
    if let Some(rest) = s.strip_prefix("rbxassetid://") {
        return num(rest).map(|id| AssetRef {
            id,
            version: version(rest),
        });
    }
    // `?ID=` counts too (old models: `http://www.roblox.com/asset/?ID=94746105`).
    let lower = s.to_ascii_lowercase();
    if let Some(i) = lower.find("?id=").or_else(|| lower.find("&id=")) {
        if lower.contains("roblox.com") {
            return num(&s[i + 4..]).map(|id| AssetRef {
                id,
                version: version(s),
            });
        }
    }
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
        return s.parse().ok().map(|id| AssetRef { id, version: None });
    }
    if let Some(rest) = s.strip_prefix("rbxasset://") {
        return legacy_content(rest).map(|id| AssetRef { id, version: None });
    }
    None
}

/// The asset id Studio loads for an old built-in file that content/ no
/// longer ships (`rbxasset://fonts/sword.mesh` is asset 12221720): Studio's
/// LegacyContentTable, `assets/legacy-content.tsv`. Paths match without case
/// (Studio's content folder is on Windows). A file that is there locally
/// (content/ or PlatformContent/pc) still wins.
fn legacy_content(rest: &str) -> Option<u64> {
    static TABLE: std::sync::OnceLock<HashMap<String, u64>> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let content = content_dir();
        let platform = content
            .canonicalize()
            .ok()
            .and_then(|c| c.parent().map(|p| p.join("PlatformContent/pc")));
        include_str!("../assets/legacy-content.tsv")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once('\t'))
            .filter(|(p, _)| {
                !content.join(p).is_file() && !platform.as_ref().is_some_and(|d| d.join(p).is_file())
            })
            .filter_map(|(p, id)| Some((p.to_ascii_lowercase(), id.trim().parse().ok()?)))
            .collect()
    });
    let key = rest.trim_start_matches('/').replace('\\', "/").to_ascii_lowercase();
    table.get(&key).copied()
}

/// A local file for a non-cloud content id.
pub fn resolve(id: &str, doc_dir: Option<&Path>) -> Option<PathBuf> {
    let id = id.trim();
    if let Some(rest) = id.strip_prefix("rbxasset://") {
        let rest = rest.trim_start_matches('/');
        // rbxasset:// names a file in the content folder, nothing outside it.
        if !Path::new(rest)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return None;
        }
        let p = content_dir().join(rest);
        // Like Studio: what isn't in content/ may be in the platform
        // content beside it (PlatformContent/pc/textures/sky/... holds the
        // default skybox).
        if !p.exists() {
            if let Some(alt) = content_dir()
                .canonicalize()
                .ok()
                .and_then(|c| c.parent().map(|s| s.join("PlatformContent/pc").join(rest)))
                .filter(|a| a.is_file())
            {
                return Some(alt);
            }
            // An old built-in file Studio loads by asset id instead.
            if legacy_content(rest).is_some() {
                return None;
            }
        }
        return Some(p);
    }
    if let Some(rest) = id.strip_prefix("file://") {
        return Some(PathBuf::from(rest));
    }
    if id.contains("://") || asset_ref(id).is_some() {
        return None;
    }
    let p = PathBuf::from(id);
    if p.is_absolute() {
        Some(p)
    } else {
        doc_dir.map(|d| d.join(p))
    }
}

pub enum Fetch {
    Ready(Arc<Vec<u8>>),
    Pending,
    Unavailable(String),
}

/// Downloads cloud assets in the background into the disk cache.
pub struct AssetStore {
    client: Option<SharedClient>,
    /// Treat scripts as running whatever the live state (tests).
    pub scripts_running: bool,
    pending: HashSet<AssetRef>,
    failed: HashMap<AssetRef, String>,
    memory: HashMap<AssetRef, Arc<Vec<u8>>>,
    tx: Sender<(AssetRef, Result<(), String>)>,
    rx: Receiver<(AssetRef, Result<(), String>)>,
    /// Lines for the Output panel.
    pub messages: Vec<String>,
    /// Said once: Roblox wants a signed-in session for these downloads.
    auth_told: bool,
    /// Asked for while the session check was running, or failed before a
    /// sign-in: handed back from `poll` for the caches to retry.
    deferred: Vec<AssetRef>,
}

impl Default for AssetStore {
    fn default() -> Self {
        let (tx, rx) = channel();
        AssetStore {
            client: None,
            scripts_running: false,
            pending: HashSet::new(),
            failed: HashMap::new(),
            memory: HashMap::new(),
            tx,
            rx,
            messages: Vec::new(),
            auth_told: false,
            deferred: Vec::new(),
        }
    }
}

/// The signed-in session for background loads (set with the asset store's).
static SESSION: Mutex<Option<SharedClient>> = Mutex::new(None);

pub fn session_client() -> Option<SharedClient> {
    SESSION.lock().ok()?.clone()
}

/// Whether a session is signed in (for downloads that wait).
pub fn signed_in() -> bool {
    session_client().is_some()
}

/// A session check is under way (set when it starts).
static CHECKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn session_checking(on: bool) {
    CHECKING.store(on, std::sync::atomic::Ordering::SeqCst);
}

/// The session check ended (from its worker thread): signed in or not.
pub fn session_ready(client: Option<SharedClient>) {
    if let Ok(mut g) = SESSION.lock() {
        *g = client;
    }
    session_checking(false);
}

/// Waits (up to `max`) for a session check that is still running, so a
/// download made right at Play start sees the session (LoadAsset in a
/// server script's first frame).
pub fn wait_for_session(max: std::time::Duration) -> bool {
    let t0 = std::time::Instant::now();
    while CHECKING.load(std::sync::atomic::Ordering::SeqCst) && t0.elapsed() < max {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    signed_in()
}

/// Whether an asset can be had: already on disk, or a session can fetch it.
pub fn reachable(id: u64) -> bool {
    cache_path(AssetRef { id, version: None }).exists()
        || local_asset_path(id).is_some()
        || session_client().is_some()
}

/// A public asset's bytes, blocking: the disk cache, the local asset store,
/// else public asset delivery (anonymous; docs/MATERIALS.md), saved to the cache.
/// For background threads (material packs, animations, avatar parts).
pub fn public_cached(id: u64) -> Result<Arc<Vec<u8>>, String> {
    let key=format!("rbxassetid://{id}");
    content_track(&key,1);
    let result=fetch_cached(id,true);
    content_track(&key,if result.is_ok(){2}else{3});
    result
}

/// Roblox's own copy of an asset (disk cache, else download), skipping the
/// local store: the upload ledger holds an upload's source file (a decal's
/// .png), not the asset Roblox made of it (a Model holding the Decal).
pub fn remote_cached(id: u64) -> Result<Arc<Vec<u8>>, String> {
    let key=format!("rbxassetid://{id}");
    content_track(&key,1);
    let result=fetch_cached(id,false);
    content_track(&key,if result.is_ok(){2}else{3});
    result
}

fn fetch_cached(id: u64, local: bool) -> Result<Arc<Vec<u8>>, String> {
    let a = AssetRef { id, version: None };
    if let Ok(b) = std::fs::read(cache_path(a)) {
        return Ok(Arc::new(b));
    }
    if let Some(b) = local.then(|| local_asset(id)).flatten() {
        return Ok(Arc::new(b));
    }
    // Prefer Studio's login session. Its downloader preserves Roblox-Place-Id,
    // which asset delivery requires for assets private to the current place.
    let client = session_client();
    let b = if let Some(c) = client {
        let downloader = {
            let guard = c.lock().map_err(|_| "account busy".to_string())?;
            guard.as_ref().map(|c| c.session.downloader())
        };
        match downloader {
            Some(d) => d.download_asset(id, None).map_err(|e| e.to_string())?,
            None => rustudio_account::public_asset(id).map_err(|e| e.to_string())?,
        }
    } else {
        match rustudio_account::public_asset(id) {
            Ok(b) => b,
            Err(public) => {
                let c = wait_for_session(std::time::Duration::from_secs(30))
                    .then(session_client).flatten().ok_or_else(|| public.to_string())?;
                let d = {
                    let guard = c.lock().map_err(|_| "account busy".to_string())?;
                    guard.as_ref().ok_or_else(|| public.to_string())?.session.downloader()
                };
                d.download_asset(id, None).map_err(|e| e.to_string())?
            }
        }
    };
    let path = cache_path(a);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, &b);
    Ok(Arc::new(b))
}

/// Any content id's bytes, blocking (background threads): generated
/// meshes, local files (rbxasset://, paths), else a public asset.
pub fn content_blocking(id: &str) -> Result<Arc<Vec<u8>>, String> {
    content_tracked(id, true)
}

// ------------------------------------------------- place content in Play

/// Play/Run sessions alive (each holds a [`ScriptsRunning`]).
static SCRIPTS_RUNNING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Held while a place's scripts run (a Play/Run session): place content then
/// loads only Roblox content, see [`place_content_ok`].
pub struct ScriptsRunning(());
impl ScriptsRunning {
    pub fn new() -> Self {
        SCRIPTS_RUNNING.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ScriptsRunning(())
    }
}
impl Drop for ScriptsRunning {
    fn drop(&mut self) {
        SCRIPTS_RUNNING.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}
pub fn scripts_running() -> bool {
    SCRIPTS_RUNNING.load(std::sync::atomic::Ordering::SeqCst) > 0
}

/// RUSTUDIO_ALLOW_LOCAL_CONTENT=1: local development, place properties may
/// name files on this computer in Play too (scripts can then read them back
/// through audio analysis or captures, so only for your own places).
pub fn local_content_opt_in() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RUSTUDIO_ALLOW_LOCAL_CONTENT").is_ok_and(|v| v == "1"))
}

/// Whether `id` names a file on this computer (`file://`, an absolute or a
/// relative path) rather than Roblox content (asset ids, `rbxasset://` in
/// the content folder, engine-generated `rbxtemp://`).
pub fn is_local_path(id: &str) -> bool {
    let id = id.trim();
    !id.is_empty() && (id.starts_with("file://") || (!id.contains("://") && asset_ref(id).is_none()))
}

/// Whether place content (a property's image, mesh or sound) may load given
/// whether scripts are running. Roblox never loads local files for a place;
/// rustudio's editor does (MECHANICS.md), but not while scripts run: what is
/// rendered or played can be read back (CaptureService +
/// CreateEditableImageAsync, GetWaveformAsync, AudioAnalyzer, raycasts and
/// unions against a mesh). Checked before any cache keyed by id (#16).
pub fn place_content_allowed(id: &str, running: bool) -> bool {
    !running || !is_local_path(id) || local_content_opt_in()
}

/// [`place_content_allowed`] for the live Play state.
pub fn place_content_ok(id: &str) -> bool {
    place_content_allowed(id, scripts_running())
}

const LOCAL_IN_PLAY: &str = "local files aren't place content while scripts run (RUSTUDIO_ALLOW_LOCAL_CONTENT=1 allows them)";

/// [`content_blocking`] for a running script that gets back what it loads
/// (EditableImage pixels, EditableMesh vertices, video frames): Roblox content
/// only. `file://` and plain paths would let a place's scripts read the
/// person's own files.
pub fn script_content_blocking(id: &str) -> Result<Arc<Vec<u8>>, String> {
    content_tracked(id, false)
}

fn content_tracked(id: &str, paths: bool) -> Result<Arc<Vec<u8>>, String> {
    content_track(id,1);
    let result=content_blocking_inner(id, paths);
    content_track(id,if result.is_ok(){2}else{3});
    result
}
fn content_blocking_inner(id: &str, paths: bool) -> Result<Arc<Vec<u8>>, String> {
    if paths && !place_content_ok(id) {
        return Err(format!("{id}: {LOCAL_IN_PLAY}"));
    }
    if let Some(b) = generated(id) {
        return Ok(b);
    }
    if let Some(a) = asset_ref(id) {
        return public_cached(a.id);
    }
    match resolve(id, None) {
        Some(p) if paths || id.trim().starts_with("rbxasset://") => std::fs::read(&p)
            .map(Arc::new)
            .map_err(|e| format!("{}: {e}", p.display())),
        _ => Err(format!("{id} is not a local or cloud asset id")),
    }
}

/// Non-blocking [`public_cached`]: Ready once the bytes are on disk (or in
/// the local store), Pending while a background thread downloads them.
/// Scripts use it so an asset load never stalls a frame (Roblox loads
/// animations the same way: a track's Length is 0 until its asset arrives).
pub fn public_prefetch(id: u64) -> Fetch {
    let key=format!("rbxassetid://{id}");
    content_track(&key,1);
    let result=public_prefetch_inner(id);
    // A fast worker may already have completed before Pending is returned.
    if !matches!(&result,Fetch::Pending){track_fetch(&key,&result);}
    result
}
fn public_prefetch_inner(id: u64) -> Fetch {
    use std::sync::Mutex;
    static STATES: Mutex<Option<HashMap<u64, Option<Result<(), String>>>>> = Mutex::new(None);
    let a = AssetRef { id, version: None };
    if cache_path(a).exists() || local_asset_path(id).is_some() {
        return match public_cached(id) {
            Ok(b) => Fetch::Ready(b),
            Err(e) => Fetch::Unavailable(e),
        };
    }
    let mut g = STATES.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(HashMap::new);
    match map.get(&id) {
        Some(Some(Err(e))) => return Fetch::Unavailable(e.clone()),
        Some(Some(Ok(()))) => {
            return match public_cached(id) {
                Ok(b) => Fetch::Ready(b),
                Err(e) => Fetch::Unavailable(e),
            }
        }
        Some(None) => return Fetch::Pending,
        None => {}
    }
    map.insert(id, None);
    std::thread::spawn(move || {
        let r = public_cached(id).map(|_| ());
        content_track(&format!("rbxassetid://{id}"),if r.is_ok(){2}else{3});
        if let Ok(mut g) = STATES.lock() {
            g.get_or_insert_with(HashMap::new).insert(id, Some(r));
        }
    });
    Fetch::Pending
}

/// MarketplaceService:GetProductInfo's catalog record, non-blocking like
/// [`public_prefetch`]: Ready with the JSON body, Pending while a background
/// thread asks the web, or the error Roblox raises (the HTTP status). Kind is
/// Enum.InfoType's name; only Asset and GamePass have public endpoints.
/// Roblox's reply for an asset is economy's details record, field for field
/// (Name, Creator, AssetTypeId, IsForSale, …).
pub fn product_info(id: u64, kind: &str) -> Fetch {
    type States = HashMap<(u64, String), Option<Result<Arc<Vec<u8>>, String>>>;
    static STATES: Mutex<Option<States>> = Mutex::new(None);
    let key = (id, kind.to_string());
    let mut g = STATES.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(HashMap::new);
    match map.get(&key) {
        Some(Some(Ok(b))) => return Fetch::Ready(b.clone()),
        Some(Some(Err(e))) => return Fetch::Unavailable(e.clone()),
        Some(None) => return Fetch::Pending,
        None => {}
    }
    let url = match kind {
        "Asset" => format!("https://economy.roblox.com/v2/assets/{id}/details"),
        "GamePass" => format!("https://apis.roblox.com/game-passes/v1/game-passes/{id}/product-info"),
        // Developer products, bundles, subscriptions: their endpoints need
        // the experience's universe; an empty record, as before round 26.
        _ => return Fetch::Ready(Arc::new(b"{}".to_vec())),
    };
    map.insert(key.clone(), None);
    std::thread::spawn(move || {
        let r = (|| {
            let mut r = rustudio_account::api::agent()
                .get(&url)
                .call()
                .map_err(|e| e.to_string())?;
            let status = r.status();
            if !status.is_success() {
                return Err(format!(
                    "HTTP {} ({})",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or("Error")
                ));
            }
            let body = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
            Ok(Arc::new(body.into_bytes()))
        })();
        if let Ok(mut g) = STATES.lock() {
            g.get_or_insert_with(HashMap::new).insert(key, Some(r));
        }
    });
    Fetch::Pending
}

pub fn cache_path(a: AssetRef) -> PathBuf {
    let v = a
        .version
        .map(|v| v.to_string())
        .unwrap_or_else(|| "latest".into());
    cache_dir()
        .join("assets")
        .join(format!("{}_v{v}.bin", a.id))
}

impl AssetStore {
    /// Gives the store the account's client (None when signed out).
    pub fn set_client(&mut self, client: Option<SharedClient>) {
        let had = self.client.is_some();
        if had != client.is_some() {
            if let Ok(mut g) = SESSION.lock() {
                *g = client.clone();
            }
        }
        self.client = client;
        if !had && self.client.is_some() {
            // Retry what failed only because nobody was signed in.
            self.deferred.extend(self.failed.drain().map(|(a, _)| a));
            self.auth_told = false;
        }
    }

    /// The bytes of a cloud asset: from memory, the disk cache, or a
    /// background download.
    pub fn fetch(&mut self, a: AssetRef) -> Fetch {
        let key=format!("rbxassetid://{}{}",a.id,a.version.map(|v|format!("?version={v}")).unwrap_or_default());
        content_track(&key,1);
        let result=self.fetch_inner(a);
        if !matches!(&result,Fetch::Pending){track_fetch(&key,&result);}
        result
    }
    fn fetch_inner(&mut self, a: AssetRef) -> Fetch {
        if let Some(b) = self.memory.get(&a) {
            return Fetch::Ready(b.clone());
        }
        if let Ok(b) = std::fs::read(cache_path(a)) {
            let b = Arc::new(b);
            self.memory.insert(a, b.clone());
            return Fetch::Ready(b);
        }
        if let Some(b) = local_asset(a.id) {
            let b = Arc::new(b);
            self.memory.insert(a, b.clone());
            return Fetch::Ready(b);
        }
        if let Some(why) = self.failed.get(&a) {
            return Fetch::Unavailable(why.clone());
        }
        if self.pending.contains(&a) {
            return Fetch::Pending;
        }
        // The session check is still running: wait for it rather than
        // fail anonymously (most assets need the session).
        if self.client.is_none() && CHECKING.load(std::sync::atomic::Ordering::SeqCst) {
            if !self.deferred.contains(&a) {
                self.deferred.push(a);
            }
            return Fetch::Pending;
        }
        // Signed in: the account's Open Cloud download first; anything it
        // refuses (and everything when signed out) from public asset
        // delivery, which serves public assets anonymously (docs/MATERIALS.md).
        // The check's worker sets SESSION a frame before `set_client` runs.
        let client = self.client.clone().or_else(session_client);
        self.pending.insert(a);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                // Copy the session out so downloads run side by side.
                let account = client
                    .and_then(|client| Some(client.lock().ok()?.as_ref()?.session.downloader()))
                    .map(|d| d.download_asset(a.id, a.version).map_err(|e| e.to_string()));
                let bytes = match account {
                    Some(Ok(b)) => b,
                    Some(Err(e)) if a.version.is_some() => return Err(e),
                    Some(Err(e)) => {
                        rustudio_account::public_asset(a.id).map_err(|p| format!("{e}; {p}"))?
                    }
                    None if a.version.is_some() => {
                        return Err("sign in (Account) to download asset versions".into())
                    }
                    None => rustudio_account::public_asset(a.id).map_err(|e| e.to_string())?,
                };
                let path = cache_path(a);
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, bytes).map_err(|e| e.to_string())
            })();
            content_track(&format!("rbxassetid://{}{}",a.id,a.version.map(|v|format!("?version={v}")).unwrap_or_default()),if result.is_ok(){2}else{3});
            let _ = tx.send((a, result));
        });
        Fetch::Pending
    }

    /// Finished downloads since last call (callers retry those ids).
    pub fn poll(&mut self) -> Vec<AssetRef> {
        let mut done = Vec::new();
        if !self.deferred.is_empty() && !CHECKING.load(std::sync::atomic::Ordering::SeqCst) {
            done.append(&mut self.deferred);
        }
        while let Ok((a, r)) = self.rx.try_recv() {
            self.pending.retain(|p| *p != a);
            match r {
                Ok(()) => {
                    self.messages.push(format!("Downloaded asset {}", a.id));
                    done.push(a);
                }
                Err(e) => {
                    if !self.auth_told && (e.contains("Authentication required") || e.contains("not authorized")) {
                        self.auth_told = true;
                        self.messages.push(
                            "Roblox only gives most assets to a signed-in session: sign in (Account) \
                             to see this place's meshes and textures. Until then they're left out, \
                             as Studio shows assets it can't load."
                                .to_string(),
                        );
                    }
                    self.messages
                        .push(format!("Asset {} could not be downloaded: {e}", a.id));
                    self.failed.insert(a, e);
                }
            }
        }
        done
    }

    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Why a cloud asset isn't here (round 23's asset report): the
    /// download error, "downloading", or None when it is (or wasn't asked).
    pub fn why_not(&self, id: &str) -> Option<String> {
        let a = asset_ref(id)?;
        if let Some(e) = self.failed.get(&a) {
            return Some(e.clone());
        }
        self.pending.contains(&a).then(|| "still downloading".to_string())
    }

    /// Whether place content `id` may load now ([`place_content_allowed`]):
    /// callers with a cache keyed by id ask before looking in it.
    pub fn allows(&self, id: &str) -> bool {
        place_content_allowed(id, self.scripts_running || scripts_running())
    }

    /// Bytes for any content id (cloud or local; local only while no
    /// scripts run).
    pub fn content(&mut self, id: &str, doc_dir: Option<&Path>) -> Fetch {
        if !self.allows(id) {
            return Fetch::Unavailable(format!("{id}: {LOCAL_IN_PLAY}"));
        }
        let result=self.content_inner(id,doc_dir);
        track_fetch(id,&result);
        result
    }
    fn content_inner(&mut self, id: &str, doc_dir: Option<&Path>) -> Fetch {
        if let Some(b) = generated(id) {
            return Fetch::Ready(b);
        }
        if let Some(a) = asset_ref(id) {
            return self.fetch(a);
        }
        match resolve(id, doc_dir) {
            Some(p) => match std::fs::read(&p) {
                Ok(b) => Fetch::Ready(Arc::new(b)),
                Err(e) => Fetch::Unavailable(format!("{}: {e}", p.display())),
            },
            None => Fetch::Unavailable(format!("{id} is not a local or cloud asset id")),
        }
    }
}

/// A mesh normalised to the unit box (-0.5..0.5) so it can be stretched to
/// any size, plus its original extent.
#[derive(Debug, Clone, Copy)]
pub struct LoadedMesh {
    pub index: u32,
    pub extent: Vec3,
}

/// Parses mesh bytes and normalises them into the unit box.
pub fn load_mesh(bytes: &[u8]) -> Result<(Vec<Vec3>, Vec<Vec3>, Vec<Vec2>, Vec3, Vec<glam::Vec4>), String> {
    let (p,n,u,e,c,_) = load_mesh_projected(bytes)?;
    Ok((p,n,u,e,c))
}

fn load_mesh_projected(bytes: &[u8]) -> Result<(Vec<Vec3>, Vec<Vec3>, Vec<Vec2>, Vec3, Vec<glam::Vec4>, Vec<glam::Vec4>), String> {
    let m = mesh::parse(bytes)?;
    if m.positions.is_empty() {
        return Err("the mesh has no triangles".into());
    }
    let extent = (m.max - m.min).max(Vec3::splat(1e-4));
    let center = (m.max + m.min) * 0.5;
    if std::env::var_os("RUSTUDIO_MESH_DENSITY_TRACE").is_some() {
        eprintln!("MESH_BOUNDS {:?}..{:?} extent={:?} authored_uvs={} projection_tags={}", m.min, m.max, extent, m.uvs.len(), m.projection_tags.len());
    }
    let positions = m.positions.iter().map(|p| (*p - center) / extent).collect();
    // Normals are pre-scaled by the extent; the shader divides by the drawn size.
    let normals = m.normals.iter().map(|n| *n * extent).collect();
    let planes = (0..m.positions.len()).map(|i| extent.extend(m.projection_tags.get(i).copied().unwrap_or(0) as f32)).collect();
    Ok((positions, normals, m.uvs, extent, m.colors, planes))
}

/// Uploads positions, normals, UVs and (possibly empty) per-corner colours.
pub type MeshUpload<'a> = dyn FnMut(&[Vec3], &[Vec3], &[Vec2], &[glam::Vec4], &[glam::Vec4]) -> u32 + 'a;
pub type TextureUpload<'a> = dyn FnMut(&Rgba) -> u32 + 'a;

/// Loaded meshes by content id (None = failed for good).
#[derive(Default)]
pub struct MeshCache {
    entries: HashMap<String, Option<LoadedMesh>>,
    /// Why an entry is None (fetch or parse error).
    reasons: HashMap<String, String>,
    /// Messages for the Output panel (one per new failure).
    pub messages: Vec<String>,
}

impl MeshCache {
    pub fn get(
        &mut self,
        id: &str,
        doc_dir: Option<&Path>,
        store: &mut AssetStore,
        upload: &mut MeshUpload,
    ) -> Option<LoadedMesh> {
        if !store.allows(id) {
            return None;
        }
        if let Some(e) = self.entries.get(id) {
            return *e;
        }
        let loaded = match store.content(id, doc_dir) {
            Fetch::Pending => return None,
            Fetch::Unavailable(why) => {
                if asset_ref(id).is_none() {
                    self.messages.push(format!(
                        "Mesh {id} not loaded ({why}); showing a placeholder"
                    ));
                }
                self.reasons.insert(id.to_string(), format!("not fetched: {why}"));
                None
            }
            Fetch::Ready(bytes) => match load_mesh_projected(&bytes) {
                Ok((pos, nrm, uv, extent, mut colors, planes)) => {
                    // A union with UsePartColor: its own Color, not its children's.
                    if id.ends_with("#partcolor") {
                        colors.clear();
                    }
                    Some(LoadedMesh {
                        index: upload(&pos, &nrm, &uv, &colors, &planes),
                        extent,
                    })
                }
                Err(e) => {
                    self.messages
                        .push(format!("Mesh {id} not loaded: {e}; showing a placeholder"));
                    self.reasons.insert(id.to_string(), format!("not decoded: {e}"));
                    None
                }
            },
        };
        self.entries.insert(id.to_string(), loaded);
        loaded
    }

    /// Forget entries for assets that just finished downloading.
    pub fn retry(&mut self, done: &[AssetRef]) {
        self.entries
            .retain(|id, _| asset_ref(id).is_none_or(|a| !done.contains(&a)));
    }

    /// Loaded (Ok), failed (Err with why), or never asked for (None).
    pub fn state(&self, id: &str) -> Option<Result<(), String>> {
        match self.entries.get(id)? {
            Some(_) => Some(Ok(())),
            None => Some(Err(self.reasons.get(id).cloned().unwrap_or_else(|| "failed".into()))),
        }
    }
}

/// RUSTUDIO_SYNC_DECODE=1 decodes every texture in the frame that asks.
/// Texture upload time per frame before further uploads wait a frame.
const UPLOAD_BUDGET_MS: f32 = 6.0;

fn sync_decode() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RUSTUDIO_SYNC_DECODE").is_some())
}

/// A texture on the GPU plus its pixels (clothing compositing reads them).
#[derive(Clone)]
pub struct LoadedTexture {
    pub index: u32,
    pub image: Arc<Rgba>,
}

#[derive(Default)]
pub struct TextureCache {
    entries: HashMap<String, Option<LoadedTexture>>,
    /// Why an entry is None (fetch or decode error).
    reasons: HashMap<String, String>,
    /// Composites (clothing): key → GPU index.
    composites: HashMap<String, u32>,
    pub messages: Vec<String>,
    /// Decodes running on worker threads, and finished ones not yet uploaded.
    decoding: std::collections::HashSet<String>,
    decoded: HashMap<String, Result<Rgba, String>>,
    /// Upload time spent this frame (mip chain + GPU copy), and whether a
    /// decoded texture was held back for the next frame by that budget.
    upload_ms: f32,
    deferred: bool,
    decode_chan: Option<(
        std::sync::mpsc::Sender<(String, Result<Rgba, String>)>,
        std::sync::mpsc::Receiver<(String, Result<Rgba, String>)>,
    )>,
    /// EditableImage textures and composites made from them, by key without
    /// generations (runtime/editable.rs): the generation they hold. A newer
    /// one is uploaded into the same GPU slot.
    editable_gens: HashMap<String, u64>,
    editable_composites: HashMap<String, (String, u32)>,
}

impl TextureCache {
    /// An EditableImage's pixels (`rbxtemp://rustudio/editable/img/<n>@<gen>`),
    /// re-uploaded in place when they changed.
    fn editable_image(&mut self, id: &str, upload: &mut TextureUpload) -> Option<LoadedTexture> {
        let (base, gen) = crate::runtime::editable::image_key(id)?;
        let held = self.editable_gens.get(base).copied();
        if held.is_some_and(|h| h >= gen) {
            if let Some(Some(t)) = self.entries.get(base) {
                return Some(t.clone());
            }
        }
        let Some((now, img)) = crate::runtime::editable::image_pixels(id) else {
            return self.entries.get(base).cloned().flatten();
        };
        if let Some(Some(old)) = self.entries.get(base) {
            crate::render::reuse_texture_slot(old.index);
        }
        let index = upload(&img);
        let t = LoadedTexture { index, image: Arc::new(img) };
        self.entries.insert(base.to_string(), Some(t.clone()));
        self.editable_gens.insert(base.to_string(), now.max(gen));
        Some(t)
    }

    pub fn image(
        &mut self,
        id: &str,
        doc_dir: Option<&Path>,
        store: &mut AssetStore,
        upload: &mut TextureUpload,
    ) -> Option<LoadedTexture> {
        if id.starts_with("rbxtemp://rustudio/editable/") {
            return self.editable_image(id, upload);
        }
        if !store.allows(id) {
            return None;
        }
        if let Some(e) = self.entries.get(id) {
            return e.clone();
        }
        if self.decoding.contains(id) {
            return None;
        }
        let decoded = match self.decoded.remove(id) {
            Some(d) => Some(d),
            None => match store.content(id, doc_dir) {
                Fetch::Pending => return None,
                Fetch::Unavailable(why) => {
                    if asset_ref(id).is_none() {
                        self.messages
                            .push(format!("Texture {id} not loaded ({why})"));
                    }
                    self.reasons.insert(id.to_string(), format!("not fetched: {why}"));
                    self.entries.insert(id.to_string(), None);
                    return None;
                }
                // Small images decode here; bigger ones on the decode
                // workers (the part waits and is redone on arrival).
                Fetch::Ready(bytes) if bytes.len() <= 16 * 1024 || sync_decode() => Some(texture::decode(&bytes)),
                Fetch::Ready(bytes) => {
                    let (tx, _) = self.decode_chan.get_or_insert_with(std::sync::mpsc::channel);
                    texture::decode_async(bytes, tx.clone(), id.to_string());
                    self.decoding.insert(id.to_string());
                    return None;
                }
            },
        };
        if decoded.is_some() && self.upload_ms > UPLOAD_BUDGET_MS && !sync_decode() {
            // Over this frame's budget: upload it next frame.
            if let Some(d) = decoded {
                self.decoded.insert(id.to_string(), d);
            }
            self.deferred = true;
            return None;
        }
        let loaded = match decoded {
            None => None,
            Some(result) => match result {
                Ok(img) => {
                    let t0 = std::time::Instant::now();
                    let index = upload(&img);
                    self.upload_ms += t0.elapsed().as_secs_f32() * 1000.0;
                    Some(LoadedTexture {
                        index,
                        image: Arc::new(img),
                    })
                }
                Err(e) => {
                    self.messages.push(format!("Texture {id} not loaded: {e}"));
                    self.reasons.insert(id.to_string(), format!("not decoded: {e}"));
                    None
                }
            },
        };
        self.entries.insert(id.to_string(), loaded.clone());
        loaded
    }

    /// Collects finished background decodes; true when any arrived (the
    /// caller bumps its asset generation so waiting parts are redone).
    pub fn poll_decoded(&mut self) -> bool {
        self.upload_ms = 0.0;
        let mut any = std::mem::take(&mut self.deferred);
        let Some((_, rx)) = &self.decode_chan else { return any };
        while let Ok((id, r)) = rx.try_recv() {
            self.decoding.remove(&id);
            self.decoded.insert(id, r);
            any = true;
        }
        any
    }

    /// Decodes still running.
    pub fn decoding(&self) -> bool {
        !self.decoding.is_empty()
    }

    /// Loaded (Ok), failed (Err with why), or never asked for (None).
    pub fn state(&self, id: &str) -> Option<Result<(), String>> {
        match self.entries.get(id)? {
            Some(_) => Some(Ok(())),
            None => Some(Err(self.reasons.get(id).cloned().unwrap_or_else(|| "failed".into()))),
        }
    }

    /// A cached composite, building it the first time.
    pub fn composite(
        &mut self,
        key: &str,
        build: impl FnOnce() -> Option<Rgba>,
        upload: &mut TextureUpload,
    ) -> Option<u32> {
        if let Some(i) = self.composites.get(key) {
            return Some(*i);
        }
        // Made from an EditableImage: one GPU slot per composite, whatever
        // the generation (the old one is replaced, not leaked).
        let stripped = crate::runtime::editable::strip_generations(key);
        let mut reuse = None;
        if let Some(base) = &stripped {
            if let Some((full, i)) = self.editable_composites.get(base) {
                if full == key {
                    return Some(*i);
                }
                reuse = Some(*i);
            }
        }
        let img = build()?;
        if let Some(i) = reuse {
            crate::render::reuse_texture_slot(i);
        }
        let i = upload(&img);
        match stripped {
            Some(base) => {
                self.editable_composites.insert(base, (key.to_string(), i));
            }
            None => {
                self.composites.insert(key.to_string(), i);
            }
        }
        Some(i)
    }

    pub fn retry(&mut self, done: &[AssetRef]) {
        self.entries
            .retain(|id, _| asset_ref(id).is_none_or(|a| !done.contains(&a)));
        if !done.is_empty() {
            self.composites.clear();
        }
    }
}

// ------------------------------------------------------------------ clothing

/// The classic clothing template is 585×559; regions are (x, y, w, h).
pub const TEMPLATE_W: f32 = 585.0;
pub const TEMPLATE_H: f32 = 559.0;

/// Template regions for one box: front (−Z), back (+Z), +X side, −X side, top, bottom.
#[derive(Debug, Clone, Copy)]
pub struct BoxRegions {
    pub front: [f32; 4],
    pub back: [f32; 4],
    pub px: [f32; 4],
    pub nx: [f32; 4],
    pub top: [f32; 4],
    pub bottom: [f32; 4],
}

pub const TORSO: BoxRegions = BoxRegions {
    front: [231.0, 74.0, 128.0, 128.0],
    back: [427.0, 74.0, 128.0, 128.0],
    px: [165.0, 74.0, 64.0, 128.0],
    nx: [361.0, 74.0, 64.0, 128.0],
    top: [231.0, 8.0, 128.0, 64.0],
    bottom: [231.0, 204.0, 128.0, 64.0],
};

/// The character's right limb (+X side): right arm on shirts, right leg on pants.
pub const RIGHT_LIMB: BoxRegions = BoxRegions {
    front: [217.0, 355.0, 64.0, 128.0],
    back: [85.0, 355.0, 64.0, 128.0],
    px: [151.0, 355.0, 64.0, 128.0],
    nx: [19.0, 355.0, 64.0, 128.0],
    top: [217.0, 289.0, 64.0, 64.0],
    bottom: [217.0, 485.0, 64.0, 64.0],
};

pub const LEFT_LIMB: BoxRegions = BoxRegions {
    front: [308.0, 355.0, 64.0, 128.0],
    back: [440.0, 355.0, 64.0, 128.0],
    px: [506.0, 355.0, 64.0, 128.0],
    nx: [374.0, 355.0, 64.0, 128.0],
    top: [308.0, 289.0, 64.0, 64.0],
    bottom: [308.0, 485.0, 64.0, 64.0],
};

/// Which clothing image a body part uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClothingLayer {
    /// Pants under the shirt (torso parts).
    Torso,
    Shirt,
    Pants,
}

/// A body part's template mapping: regions, the vertical slice [t0, t1] of
/// the side regions (R15 parts split a limb), and the image to use.
#[derive(Debug, Clone, Copy)]
pub struct ClothingRole {
    pub regions: BoxRegions,
    pub t0: f32,
    pub t1: f32,
    pub layer: ClothingLayer,
}

/// The template role of a body part by its R6 / R15 name.
pub fn clothing_role(part: &str) -> Option<ClothingRole> {
    use ClothingLayer::*;
    let r = |regions, t0, t1, layer| {
        Some(ClothingRole {
            regions,
            t0,
            t1,
            layer,
        })
    };
    match part {
        "Torso" => r(TORSO, 0.0, 1.0, Torso),
        "UpperTorso" => r(TORSO, 0.0, 0.8, Torso),
        "LowerTorso" => r(TORSO, 0.8, 1.0, Torso),
        "Right Arm" => r(RIGHT_LIMB, 0.0, 1.0, Shirt),
        "Left Arm" => r(LEFT_LIMB, 0.0, 1.0, Shirt),
        "Right Leg" => r(RIGHT_LIMB, 0.0, 1.0, Pants),
        "Left Leg" => r(LEFT_LIMB, 0.0, 1.0, Pants),
        "RightUpperArm" => r(RIGHT_LIMB, 0.0, 0.4, Shirt),
        "RightLowerArm" => r(RIGHT_LIMB, 0.4, 0.85, Shirt),
        "RightHand" => r(RIGHT_LIMB, 0.85, 1.0, Shirt),
        "LeftUpperArm" => r(LEFT_LIMB, 0.0, 0.4, Shirt),
        "LeftLowerArm" => r(LEFT_LIMB, 0.4, 0.85, Shirt),
        "LeftHand" => r(LEFT_LIMB, 0.85, 1.0, Shirt),
        "RightUpperLeg" => r(RIGHT_LIMB, 0.0, 0.4, Pants),
        "RightLowerLeg" => r(RIGHT_LIMB, 0.4, 0.85, Pants),
        "RightFoot" => r(RIGHT_LIMB, 0.85, 1.0, Pants),
        "LeftUpperLeg" => r(LEFT_LIMB, 0.0, 0.4, Pants),
        "LeftLowerLeg" => r(LEFT_LIMB, 0.4, 0.85, Pants),
        "LeftFoot" => r(LEFT_LIMB, 0.85, 1.0, Pants),
        _ => None,
    }
}

/// A unit box whose faces map into the clothing template regions.
pub fn template_box(role: &ClothingRole) -> (Vec<Vec3>, Vec<Vec3>, Vec<Vec2>) {
    let h = 0.5;
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    let mut uvs = Vec::new();
    let to_uv = |rect: [f32; 4], s: f32, t: f32| {
        Vec2::new(
            (rect[0] + s * rect[2]) / TEMPLATE_W,
            (rect[1] + t * rect[3]) / TEMPLATE_H,
        )
    };
    let side_t = |t: f32| role.t0 + t * (role.t1 - role.t0);
    // (normal, region, s(p), t(p)) per face, same orientation as the renderer's box UVs.
    type Face = (Vec3, [f32; 4], fn(Vec3) -> f32, fn(Vec3) -> f32, bool);
    let faces: [Face; 6] = [
        (
            Vec3::NEG_Z,
            role.regions.front,
            |p| 0.5 - p.x,
            |p| 0.5 - p.y,
            true,
        ),
        (
            Vec3::Z,
            role.regions.back,
            |p| p.x + 0.5,
            |p| 0.5 - p.y,
            true,
        ),
        (Vec3::X, role.regions.px, |p| 0.5 - p.z, |p| 0.5 - p.y, true),
        (
            Vec3::NEG_X,
            role.regions.nx,
            |p| p.z + 0.5,
            |p| 0.5 - p.y,
            true,
        ),
        (
            Vec3::Y,
            role.regions.top,
            |p| 0.5 - p.x,
            |p| 0.5 - p.z,
            false,
        ),
        (
            Vec3::NEG_Y,
            role.regions.bottom,
            |p| 0.5 - p.x,
            |p| p.z + 0.5,
            false,
        ),
    ];
    for (n, rect, fs, ft, side) in faces {
        // Two axes spanning the face.
        let (u, v) = if n.x.abs() > 0.5 {
            (Vec3::Z, Vec3::Y)
        } else if n.y.abs() > 0.5 {
            (Vec3::X, Vec3::Z)
        } else {
            (Vec3::X, Vec3::Y)
        };
        let c = n * h;
        let corners = [
            c - u * h - v * h,
            c + u * h - v * h,
            c + u * h + v * h,
            c - u * h + v * h,
        ];
        // Outward winding.
        let (a, b, cc, d) = if (corners[1] - corners[0])
            .cross(corners[2] - corners[0])
            .dot(n)
            > 0.0
        {
            (corners[0], corners[1], corners[2], corners[3])
        } else {
            (corners[0], corners[3], corners[2], corners[1])
        };
        for p in [a, b, cc, a, cc, d] {
            let t = ft(p);
            let t = if side { side_t(t) } else { t };
            pos.push(p);
            nrm.push(n);
            uvs.push(to_uv(rect, fs(p), t));
        }
    }
    (pos, nrm, uvs)
}

/// Where a point on a body part's unit box (-0.5..0.5) with outward normal
/// `n` lands in the clothing template (the same faces as `template_box`).
pub fn template_uv(role: &ClothingRole, n: Vec3, p: Vec3) -> Vec2 {
    let p = p.clamp(Vec3::splat(-0.5), Vec3::splat(0.5));
    let a = n.abs();
    let r = &role.regions;
    let (rect, s, t, side) = if a.x >= a.y && a.x >= a.z {
        if n.x > 0.0 {
            (r.px, 0.5 - p.z, 0.5 - p.y, true)
        } else {
            (r.nx, p.z + 0.5, 0.5 - p.y, true)
        }
    } else if a.y >= a.z {
        if n.y > 0.0 {
            (r.top, 0.5 - p.x, 0.5 - p.z, false)
        } else {
            (r.bottom, 0.5 - p.x, p.z + 0.5, false)
        }
    } else if n.z < 0.0 {
        (r.front, 0.5 - p.x, 0.5 - p.y, true)
    } else {
        (r.back, p.x + 0.5, 0.5 - p.y, true)
    };
    let t = if side { role.t0 + t * (role.t1 - role.t0) } else { t };
    Vec2::new(
        (rect[0] + s * rect[2]) / TEMPLATE_W,
        (rect[1] + t * rect[3]) / TEMPLATE_H,
    )
}

/// Body part meshes (R6: Studio's shipped ones and package CharacterMeshes;
/// R15 MeshParts) are UV-mapped to Roblox's composited clothing atlases,
/// not to the 585×559 template; those atlases are the template's box faces
/// laid out again. So each triangle of such a mesh (positions in its unit
/// box) is box-projected onto the template by its own face normal: all
/// three corners land in one template region, never smeared across two.
pub fn body_template_uvs(part: &str, positions: &[Vec3]) -> Option<Vec<Vec2>> {
    let role = clothing_role(part)?;
    let mut out = Vec::with_capacity(positions.len());
    for tri in positions.chunks(3) {
        if tri.len() < 3 {
            out.extend(tri.iter().map(|_| Vec2::ZERO));
            continue;
        }
        // Outward: the positions are centred, so the face points away
        // from the middle of the box.
        let mut n = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
        if n.dot(tri[0] + tri[1] + tri[2]) < 0.0 {
            n = -n;
        }
        out.extend(tri.iter().map(|p| template_uv(&role, n, *p)));
    }
    Some(out)
}

/// A mesh's positions / normals / UVs as uploaded (normalized to its unit box).
pub fn mesh_for_upload(bytes: &[u8]) -> Option<(Vec<Vec3>, Vec<Vec3>, Vec<Vec2>)> {
    let (p, n, uv, _, _, _) = load_mesh_projected(bytes).ok()?;
    Some((p, n, uv))
}

/// Shirt over pants (alpha "over"), for torso parts.
pub fn composite_over(under: Option<&Rgba>, over: Option<&Rgba>) -> Option<Rgba> {
    match (under, over) {
        (None, None) => None,
        (Some(u), None) => Some(u.clone()),
        (None, Some(o)) => Some(o.clone()),
        (Some(u), Some(o)) => {
            let (w, h) = (o.width, o.height);
            let mut out = o.clone();
            for y in 0..h {
                for x in 0..w {
                    // Sample the under-image at the same relative position.
                    let ux = (x as u64 * u.width as u64 / w.max(1) as u64) as u32;
                    let uy = (y as u64 * u.height as u64 / h.max(1) as u64) as u32;
                    let ui = ((uy * u.width + ux) * 4) as usize;
                    let oi = ((y * w + x) * 4) as usize;
                    let (Some(us), Some(os)) =
                        (u.pixels.get(ui..ui + 4), out.pixels.get(oi..oi + 4))
                    else {
                        continue;
                    };
                    let (us, os) = ([us[0], us[1], us[2], us[3]], [os[0], os[1], os[2], os[3]]);
                    let oa = os[3] as f32 / 255.0;
                    let ua = us[3] as f32 / 255.0;
                    let a = oa + ua * (1.0 - oa);
                    let mut px = [0u8; 4];
                    for c in 0..3 {
                        let v = if a > 0.0 {
                            (os[c] as f32 * oa + us[c] as f32 * ua * (1.0 - oa)) / a
                        } else {
                            0.0
                        };
                        px[c] = v.round().clamp(0.0, 255.0) as u8;
                    }
                    px[3] = (a * 255.0).round() as u8;
                    out.pixels[oi..oi + 4].copy_from_slice(&px);
                }
            }
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_id_forms() {
        assert_eq!(
            asset_ref("rbxassetid://12345"),
            Some(AssetRef {
                id: 12345,
                version: None
            })
        );
        assert_eq!(
            asset_ref("http://www.roblox.com/asset/?id=987&version=3"),
            Some(AssetRef {
                id: 987,
                version: Some(3)
            })
        );
        assert_eq!(
            asset_ref("42"),
            Some(AssetRef {
                id: 42,
                version: None
            })
        );
        assert_eq!(asset_ref("rbxasset://fonts/x.mesh"), None);
    }

    #[test]
    fn legacy_content_maps_to_asset_ids() {
        // Studio's LegacyContentTable: these files left content/ long ago.
        let id = |s: &str| asset_ref(s).map(|a| a.id);
        assert_eq!(id("rbxasset://fonts/sword.mesh"), Some(12221720));
        assert_eq!(id("rbxasset://Fonts/Torso.mesh"), Some(12221758));
        assert_eq!(id("rbxasset://textures/SwordTexture.png"), Some(12224218));
        assert_eq!(id("rbxasset://textures/rocketlaunchertex.png"), Some(12224070));
        assert_eq!(id("rbxasset://fonts/clonewand.mesh"), Some(12221344));
        // Old models spell the query `?ID=`.
        assert_eq!(id("http://www.roblox.com/asset/?ID=94746105"), Some(94746105));
    }

    /// #16: while scripts run, a place's images and meshes come only from
    /// Roblox content (a local file rendered could be captured and read
    /// back), even when Edit already cached them; Roblox ids, the content
    /// folder and generated images still load, and Edit keeps local files.
    #[test]
    fn local_place_content_is_not_loaded_while_scripts_run() {
        let dir = std::env::temp_dir().join(format!("rustudio-play-content-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("private.png");
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(1, 1, image::Rgba([12, 34, 56, 255])))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let bytes = png.into_inner();
        std::fs::write(&path, &bytes).unwrap();
        let locals = [format!("file://{}", path.display()), path.display().to_string(), "private.png".to_string()];
        let mut store = AssetStore::default();
        let mut textures = TextureCache::default();
        let mut up = |_: &Rgba| 7u32;
        // Edit: local files render (and land in the texture cache).
        for id in &locals {
            assert!(matches!(store.content(id, Some(&dir)), Fetch::Ready(b) if *b == bytes), "{id}");
            assert!(textures.image(id, Some(&dir), &mut store, &mut up).is_some(), "{id}");
        }
        // Play: neither a fresh load nor the cache gives them.
        store.scripts_running = true;
        for id in &locals {
            assert!(matches!(store.content(id, Some(&dir)), Fetch::Unavailable(_)), "{id}");
            assert!(textures.image(id, Some(&dir), &mut store, &mut up).is_none(), "{id}");
            assert!(!store.allows(id), "{id}");
        }
        // Generated (capture) images and Roblox ids still do.
        let id = "rbxtemp://rustudio/test-play-content.png";
        register_generated(id, bytes.clone());
        let generated_ok = matches!(store.content(id, None), Fetch::Ready(b) if *b == bytes)
            && textures.image(id, None, &mut store, &mut up).is_some();
        unregister_generated(id);
        assert!(generated_ok);
        for id in ["rbxassetid://12345", "rbxasset://textures/white.png", "http://www.roblox.com/asset/?id=5", "12345"] {
            assert!(store.allows(id), "{id}");
        }
        // Back in Edit: the cached texture is there again.
        store.scripts_running = false;
        assert!(textures.image(&locals[0], Some(&dir), &mut store, &mut up).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn place_content_policy() {
        for id in ["file:///tmp/a.png", "/tmp/a.png", "a.png", "meshes/a.obj", " /x.wav "] {
            assert!(is_local_path(id), "{id}");
            assert!(place_content_allowed(id, false), "{id}");
            assert!(!place_content_allowed(id, true) || local_content_opt_in(), "{id}");
        }
        for id in ["rbxassetid://1", "rbxasset://sounds/x.wav", "rbxtemp://rustudio/x", "1234", "rbxthumb://type=Asset&id=1&w=150&h=150", ""] {
            assert!(!is_local_path(id), "{id}");
            assert!(place_content_allowed(id, true), "{id}");
        }
    }

    #[test]
    fn resolves_local_ids_only() {
        assert!(resolve("rbxassetid://12345", None).is_none());
        assert!(resolve("http://example.com/x.mesh", None).is_none());
        assert_eq!(
            resolve("file:///tmp/a.obj", None),
            Some(PathBuf::from("/tmp/a.obj"))
        );
        assert!(resolve("rbxasset://fonts/x.mesh", None)
            .unwrap()
            .ends_with("fonts/x.mesh"));
        assert_eq!(
            resolve("meshes/a.obj", Some(Path::new("/w"))),
            Some(PathBuf::from("/w/meshes/a.obj"))
        );
    }

    #[test]
    fn rbxasset_ids_stay_in_the_content_folder() {
        for id in [
            "rbxasset://../../secret.png",
            "rbxasset:///../secret.png",
            "rbxasset://textures/../../secret.png",
        ] {
            assert_eq!(resolve(id, None), None, "{id}");
        }
        assert!(resolve("rbxasset://textures/./a.png", None).is_some());
    }

    #[test]
    fn loads_obj_into_unit_box() {
        let (pos, _, uvs, extent, _) =
            load_mesh(b"v 0 0 0\nv 4 0 0\nv 0 2 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n")
                .unwrap();
        assert_eq!(pos.len(), 3);
        assert_eq!(uvs.len(), 3);
        assert!((extent - Vec3::new(4.0, 2.0, 1e-4)).length() < 1e-3);
        assert!(pos
            .iter()
            .all(|v| v.x.abs() <= 0.5001 && v.y.abs() <= 0.5001));
    }

    #[test]
    fn template_box_front_maps_into_torso_front() {
        let role = clothing_role("Torso").unwrap();
        let (pos, _, uvs) = template_box(&role);
        assert_eq!(pos.len(), 36);
        // The front face's top-left texel (as seen from the front) is the region's corner.
        let i = pos
            .iter()
            .position(|p| {
                (p.z + 0.5).abs() < 1e-6 && (p.x - 0.5).abs() < 1e-6 && (p.y - 0.5).abs() < 1e-6
            })
            .unwrap();
        assert!((uvs[i] - Vec2::new(231.0 / TEMPLATE_W, 74.0 / TEMPLATE_H)).length() < 1e-5);
    }
}

/// A mesh's triangles in its unit box (-0.5..0.5, stretched to a part's
/// Size), read straight from local files (generated, local store / ledger,
/// disk cache, content folder) for ray tests; None when it isn't here.
pub fn mesh_triangles(id: &str) -> Option<Arc<Vec<[Vec3; 3]>>> {
    static CACHE: Mutex<Option<HashMap<String, Option<Arc<Vec<[Vec3; 3]>>>>>> = Mutex::new(None);
    if !place_content_ok(id) {
        return None;
    }
    if let Some(hit) = CACHE.lock().ok()?.get_or_insert_with(HashMap::new).get(id) {
        return hit.clone();
    }
    let bytes: Option<Vec<u8>> =
        generated(id)
            .map(|b| b.to_vec())
            .or_else(|| match asset_ref(id) {
                Some(a) => local_asset(a.id).or_else(|| std::fs::read(cache_path(a)).ok()),
                None => resolve(id, None).and_then(|p| std::fs::read(p).ok()),
            });
    let tris = bytes.and_then(|b| mesh::parse(&b).ok()).map(|m| {
        let ext = (m.max - m.min).max(Vec3::splat(1e-4));
        let c = (m.max + m.min) * 0.5;
        Arc::new(
            m.positions
                .chunks_exact(3)
                .map(|t| [(t[0] - c) / ext, (t[1] - c) / ext, (t[2] - c) / ext])
                .collect::<Vec<_>>(),
        )
    });
    // A mesh can still be downloading when decals first ask for geometry.
    // Retain successful parses only, so the overlay can follow the mesh once
    // its bytes arrive instead of permanently keeping the box fallback.
    if tris.is_some() {
        if let Ok(mut g) = CACHE.lock() {
            g.get_or_insert_with(HashMap::new)
                .insert(id.to_string(), tris.clone());
        }
    }
    tris
}

/// Nearest hit of a ray (origin, dir; dir's length is the unit) with
/// triangles: the ray parameter t.
/// Nearest triangle hit along a ray: distance and the triangle's normal,
/// turned to face the ray (Roblox reports the surface normal of the hit).
pub fn ray_triangles_normal(origin: Vec3, dir: Vec3, tris: &[[Vec3; 3]]) -> Option<(f32, Vec3)> {
    let mut best: Option<(f32, Vec3)> = None;
    for t in tris {
        let e1 = t[1] - t[0];
        let e2 = t[2] - t[0];
        let p = dir.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-12 {
            continue;
        }
        let inv = 1.0 / det;
        let s = origin - t[0];
        let u = s.dot(p) * inv;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = dir.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let tt = e2.dot(q) * inv;
        if tt > 0.0 && best.is_none_or(|(b, _)| tt < b) {
            let n = e1.cross(e2).normalize_or_zero();
            best = Some((tt, if n.dot(dir) > 0.0 { -n } else { n }));
        }
    }
    best
}
