//! Places stored on Roblox (docs/MECHANICS.md, round 27), the way Studio opens
//! them: File > Open from Roblox (Recent, My Experiences, Group Experiences,
//! Shared with Me, a place ID or link), a version picker, the download
//! cache that reopens a place offline, the document's cloud origin, Save to
//! Roblox / Publish with the version check against the server, and the
//! Team Create status. Network work runs on threads (as account_ui.rs);
//! results come back over a channel and are polled each frame.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use egui::RichText;
use rustudio_account::cloud::{CloudPlace, GroupInfo, RecentPlace, VersionInfo};
use rustudio_account::{
    open_mode, parse_place_ref, CloudApi, CloudOrigin, OpenMode, Endpoints, Experience, ExperienceSource, PlaceCache, PublishKeyStore,
    SaveOutcome, TeamCreateStatus, VersionType,
};

use crate::app::{Editor, Level, PendingAction};
use crate::doc::Document;
use crate::theme;

type Listing = Option<Result<Vec<Experience>, String>>;

/// An experience list that loads a page at a time.
#[derive(Default)]
pub(crate) struct Paged {
    pub list: Listing,
    pub page: u32,
    pub more: bool,
    pub loading: bool,
}

impl Paged {
    pub fn fresh() -> Paged {
        Paged { list: Some(Ok(Vec::new())), more: true, ..Default::default() }
    }

    /// The page to ask for next, marking it as on its way; None while one
    /// is on its way or when there are no more.
    pub fn next_page(&mut self) -> Option<u32> {
        if self.loading || !self.more {
            return None;
        }
        self.loading = true;
        Some(self.page)
    }

    /// Adds an arrived page. One from before a Refresh has the wrong number
    /// and is dropped.
    pub fn arrived(&mut self, page: u32, r: Result<(Vec<Experience>, bool), String>) {
        if page != self.page {
            return;
        }
        self.loading = false;
        match r {
            Ok((rows, more)) => {
                if let Some(Ok(list)) = &mut self.list {
                    list.extend(rows);
                }
                self.page += 1;
                self.more = more;
            }
            Err(e) => {
                self.list = Some(Err(e));
                self.more = false;
            }
        }
    }
}

/// Builds the API for a job (the self-test swaps in a mock server's).
pub type ApiFactory = Arc<dyn Fn() -> Option<CloudApi> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerTab {
    Recent,
    Mine,
    Groups,
    Shared,
}

impl PickerTab {
    const ALL: [PickerTab; 4] = [PickerTab::Recent, PickerTab::Mine, PickerTab::Groups, PickerTab::Shared];

    pub fn parse(s: &str) -> Option<PickerTab> {
        Some(match s.to_ascii_lowercase().as_str() {
            "recent" => PickerTab::Recent,
            "mine" | "my" => PickerTab::Mine,
            "groups" | "group" => PickerTab::Groups,
            "shared" => PickerTab::Shared,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            PickerTab::Recent => "Recent",
            PickerTab::Mine => "My Experiences",
            PickerTab::Groups => "Group Experiences",
            PickerTab::Shared => "Shared with Me",
        }
    }
}

/// A place downloaded (or read from the cache), ready to become the document.
pub struct Opened {
    pub origin: CloudOrigin,
    pub bytes: Vec<u8>,
    /// Why the cached copy was used instead of Roblox's, if it was.
    pub offline: Option<String>,
    pub cache_file: Option<std::path::PathBuf>,
}

enum Msg {
    Log(Level, String),
    Page(ExperienceSource, u32, Result<(Vec<Experience>, bool), String>),
    Groups(Result<Vec<GroupInfo>, String>),
    Places(u64, Result<Vec<CloudPlace>, String>),
    Icon(u64, egui::ColorImage),
    Versions(u64, Result<Vec<VersionInfo>, String>),
    Opened(Box<Opened>),
    OpenFailed(String),
    /// Team Create is on: Studio would join, not download.
    TeamCreateOn(Box<CloudOrigin>),
    Status(u64, TeamCreateStatus),
    Saved { version: u64, revision: u64, vt: VersionType, bytes: Vec<u8> },
    Conflict { opened: u64, server: u64, vt: VersionType },
    SaveFailed(String),
    Done,
}

/// The newer server version that stopped a save, and what was being done.
#[derive(Debug, Clone, Copy)]
pub struct Conflict {
    pub opened: u64,
    pub server: u64,
    pub vt: VersionType,
}

struct LiveSave {
    revision: u64,
    vt: VersionType,
    sent: Option<std::time::Instant>,
}

struct LiveView {
    session: rustudio_net::live::LiveSession,
    model: rustudio_net::model::LiveModel,
    origin: Option<CloudOrigin>,
    place: u64,
    universe: u64,
    document_id: Option<u64>,
    requested_document_id: u64,
    owner_user: Option<u64>,
    own_player: Option<u64>,
    publish_service: Option<u64>,
    save_pending: Option<LiveSave>,
    sync_failed: bool,
    drafts: crate::live_drafts::DraftStore,
    script_service: Option<u64>,
    script_configuration_sent: bool,
    next_script_request: i64,
    script_commits: HashMap<i64,(u64,std::time::Instant,String)>,
    last_presence: std::time::Instant,
    last_camera: Option<[f32; 5]>,
    locally_deleted: HashSet<rbx_types::Ref>,
    collaborators: HashSet<rbx_types::Ref>,
    players: HashMap<u64,u64>,
    selections: HashMap<u64,Vec<u64>>,
    last_selection: Option<Vec<rbx_types::Ref>>,
    last_activity: std::time::Instant,
    last_active: Option<bool>,
}

pub struct CloudUi {
    /// File > Open from Roblox.
    pub open: bool,
    pub live_collaborators_open: bool,
    /// File > Experience Settings (the place's info).
    pub settings_open: bool,
    pub tab: PickerTab,
    pub cache: PlaceCache,
    pub endpoints: Endpoints,
    /// Set by the self-test: jobs use this API instead of the session's.
    pub api_override: Option<ApiFactory>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    jobs: usize,
    lists: HashMap<ExperienceSource, Paged>,
    groups: Option<Result<Vec<GroupInfo>, String>>,
    group_sel: Option<u64>,
    places: HashMap<u64, Result<Vec<CloudPlace>, String>>,
    icons: HashMap<u64, egui::TextureHandle>,
    icon_images: Vec<(u64, egui::ColorImage)>,
    icons_asked: HashSet<u64>,
    expanded: HashSet<u64>,
    /// The place whose versions are shown: (place id, name).
    versions_of: Option<(u64, String)>,
    versions: HashMap<u64, Result<Vec<VersionInfo>, String>>,
    version_input: String,
    pub url_input: String,
    pub url_error: Option<String>,
    opened: Option<Opened>,
    /// The last open that failed (shown in the picker; the self-test reads it).
    pub open_error: Option<String>,
    live: Option<LiveView>,
    /// The opening / joining dialog (src/join_ui.rs).
    pub join: Option<crate::join_ui::JoinProgress>,
    /// Opens cancelled from the dialog whose worker hasn't answered yet.
    open_cancelled: u32,
    live_origin: Option<CloudOrigin>,
    /// Team Create and edit access of the open document's universe.
    pub status: Option<(u64, TeamCreateStatus)>,
    pub conflict: Option<Conflict>,
    pub save_error: Option<String>,
    /// The last save that went through: (version, Saved/Published).
    pub last_saved: Option<(u64, VersionType)>,
    window_title: String,
    /// `--open-roblox` / `--roblox-picker`: done once the session is checked.
    pub open_when_ready: Option<(u64, Option<u64>, bool)>,
    /// Save to Roblox / Publish asked for a local copy: confirm first.
    pub confirm_copy_save: Option<VersionType>,
    pub picker_when_ready: Option<PickerTab>,
    /// Opens of a Roblox place still downloading (Opened / OpenFailed /
    /// TeamCreateOn ends one).
    opening: u32,
    /// Play pressed while the opened place was still loading: it starts once
    /// the place (and so its PlaceId / GameId) is in, as Studio only lets Play
    /// run on a loaded DataModel.
    play_when_loaded: Option<crate::runtime::RunMode>,
}

fn live_source_text(value:&rustudio_net::replicator::PropValue)->Option<String> {
    use rustudio_net::replicator::PropValue as P;
    match value {P::String(text)=>Some(text.clone()),P::SharedString(bytes)|P::CachedSharedString{bytes,..}=>String::from_utf8(bytes.clone()).ok(),_=>None}
}

fn live_value(value: &rbx_types::Variant, model: &rustudio_net::model::LiveModel) -> Option<rustudio_net::replicator::PropValue> {
    use rbx_types::Variant as V;
    use rustudio_net::replicator::PropValue as P;
    Some(match value {
        V::Bool(v) => P::Bool(*v), V::Int32(v) => P::Int(*v as i64), V::Int64(v) => P::Int(*v),
        V::Float32(v) => P::Float(*v as f64), V::Float64(v) => P::Float(*v),
        V::Enum(v) => P::Enum(v.to_u32()), V::String(v) => P::String(v.clone()),
        V::BinaryString(v)=>P::BinaryString(v.clone().into_vec()),
        V::Attributes(v)=>{let mut bytes=Vec::new();v.to_writer(&mut bytes).ok()?;P::BinaryString(bytes)},
        V::SharedString(v) => P::SharedString(v.data().to_vec()),
        V::Color3(v) => P::Color3([v.r, v.g, v.b]), V::Color3uint8(v) => P::Color3uint8([v.r, v.g, v.b]),
        V::Vector2(v) => P::Vector2([v.x, v.y]), V::Vector3(v) => P::Vector3([v.x, v.y, v.z]),
        V::CFrame(v) => P::CFrame { pos: [v.position.x, v.position.y, v.position.z], rot: [v.orientation.x.x, v.orientation.x.y, v.orientation.x.z, v.orientation.y.x, v.orientation.y.y, v.orientation.y.z, v.orientation.z.x, v.orientation.z.y, v.orientation.z.z] },
        V::Ref(v) if v.is_none() => P::Ref(None), V::Ref(v) => P::Ref(Some(model.server_id(*v)?)),
        other => rustudio_net::model::typed_value(other)?,
    })
}

impl CloudUi {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        CloudUi {
            open: false,
            live_collaborators_open: false,
            settings_open: false,
            tab: PickerTab::Recent,
            cache: PlaceCache::default(),
            endpoints: Endpoints::default(),
            api_override: None,
            tx,
            rx,
            jobs: 0,
            lists: HashMap::new(),
            groups: None,
            group_sel: None,
            places: HashMap::new(),
            icons: HashMap::new(),
            icon_images: Vec::new(),
            icons_asked: HashSet::new(),
            expanded: HashSet::new(),
            versions_of: None,
            versions: HashMap::new(),
            version_input: String::new(),
            url_input: String::new(),
            url_error: None,
            opened: None,
            open_error: None,
            live: None,
            join: None,
            open_cancelled: 0,
            live_origin: None,
            status: None,
            conflict: None,
            save_error: None,
            last_saved: None,
            window_title: String::new(),
            open_when_ready: None,
            confirm_copy_save: None,
            picker_when_ready: None,
            opening: 0,
            play_when_loaded: None,
        }
    }

    pub fn busy(&self) -> bool {
        self.jobs > 0 || self.live.as_ref().is_some_and(|v| v.document_id.is_none())
    }

    /// The API for a job: the session's (a downloader copy, so the account
    /// lock isn't held), or the self-test's mock.
    fn factory(&self, client: Option<crate::assets::SharedClient>) -> ApiFactory {
        if let Some(f) = &self.api_override {
            return f.clone();
        }
        let ep = self.endpoints.clone();
        Arc::new(move || {
            // The account lock only for the copy; the keyring read after.
            let session = {
                let g = client.as_ref()?.lock().ok()?;
                g.as_ref()?.session.downloader()
            };
            let key = PublishKeyStore::load().ok().flatten();
            Some(CloudApi::new(session, ep.clone(), key))
        })
    }

    fn spawn(
        &mut self,
        api: ApiFactory,
        f: impl FnOnce(Option<CloudApi>, &Sender<Msg>) + Send + 'static,
    ) {
        self.jobs += 1;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            f(api(), &tx);
            let _ = tx.send(Msg::Done);
        });
    }

    fn signed_out() -> String {
        "signed out: sign in to list experiences (Recent places cached on this computer still open)".into()
    }

    /// Drops the lists of the previous account (account switch).
    pub fn forget_lists(&mut self) {
        self.lists.clear();
        self.groups = None;
        self.group_sel = None;
        self.places.clear();
        self.versions.clear();
        self.versions_of = None;
        self.expanded.clear();
    }

    /// Fills the open tab's list (and Recent's icons).
    pub fn refresh(&mut self, client: Option<crate::assets::SharedClient>, tab: PickerTab) {
        let api = self.factory(client);
        match tab {
            PickerTab::Recent => {
                let ids: Vec<u64> = self.cache.recent().iter().map(|r| r.origin.universe_id).collect();
                self.request_icons(api, ids);
            }
            PickerTab::Mine => self.reset_list(api, ExperienceSource::Mine),
            PickerTab::Shared => self.reset_list(api, ExperienceSource::Shared),
            PickerTab::Groups => {
                self.groups = Some(Ok(Vec::new()));
                self.spawn(api, |a, tx| {
                    let r = a.ok_or_else(Self::signed_out).and_then(|a| a.groups().map_err(|e| e.0));
                    let _ = tx.send(Msg::Groups(r));
                });
            }
        }
    }

    fn load_group(&mut self, client: Option<crate::assets::SharedClient>, group: u64) {
        let api = self.factory(client);
        self.reset_list(api, ExperienceSource::Group(group));
    }

    /// Starts a list again from its first page.
    fn reset_list(&mut self, api: ApiFactory, src: ExperienceSource) {
        self.lists.insert(src, Paged::fresh());
        self.load_page(api, src);
    }

    /// Asks for a list's next page, unless one is on its way or there are no more.
    fn load_page(&mut self, api: ApiFactory, src: ExperienceSource) {
        let Some(page) = self.lists.get_mut(&src).and_then(Paged::next_page) else { return };
        self.spawn(api, move |a, tx| {
            let r = a
                .ok_or_else(Self::signed_out)
                .and_then(|a| a.experiences_page(src, page, 25).map_err(|e| e.0));
            let _ = tx.send(Msg::Page(src, page, r));
        });
    }

    fn load_places(&mut self, client: Option<crate::assets::SharedClient>, universe: u64) {
        self.places.insert(universe, Ok(Vec::new()));
        self.spawn(self.factory(client), move |a, tx| {
            let r = a.ok_or_else(Self::signed_out).and_then(|a| a.places(universe).map_err(|e| e.0));
            let _ = tx.send(Msg::Places(universe, r));
        });
    }

    fn load_versions(&mut self, client: Option<crate::assets::SharedClient>, place: u64) {
        self.versions.insert(place, Ok(Vec::new()));
        self.spawn(self.factory(client), move |a, tx| {
            let r = a.ok_or_else(Self::signed_out).and_then(|a| a.versions(place).map_err(|e| e.0));
            let _ = tx.send(Msg::Versions(place, r));
        });
    }

    /// A loaded experience icon.
    pub fn icon(&self, universe: u64) -> Option<egui::TextureHandle> {
        self.icons.get(&universe).cloned()
    }

    /// Ends the live session (its worker stops).
    pub fn drop_live(&mut self) {
        self.live = None;
    }

    /// Opens still waiting on their worker are ignored when they answer.
    pub fn cancel_open(&mut self) {
        self.open_cancelled = self.opening;
    }

    /// Experience icons from thumbnails.roblox.com, each once.
    fn request_icons(&mut self, api: ApiFactory, ids: Vec<u64>) {
        let ids: Vec<u64> = ids.into_iter().filter(|i| self.icons_asked.insert(*i)).collect();
        if ids.is_empty() {
            return;
        }
        self.spawn(api, move |a, tx| {
            let Some(a) = a else { return };
            let Ok(urls) = a.icon_urls(&ids) else { return };
            for (id, url) in urls {
                let Ok(bytes) = a.image(&url) else { continue };
                let Ok(img) = image::load_from_memory(&bytes) else { continue };
                let img = img.thumbnail(64, 64).to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let _ = tx.send(Msg::Icon(id, egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw())));
            }
        });
    }

    /// Opens a place as Studio does: its universe's Team Create status
    /// first; Team Create on means joining the live session
    /// ([`Editor::open_team_create`]) unless `as_copy` asks for a local copy
    /// of the last saved version. Otherwise the latest version (or
    /// `version`) downloads; the cached copy when Roblox can't be asked.
    pub fn start_open(
        &mut self,
        client: Option<crate::assets::SharedClient>,
        place: u64,
        version: Option<u64>,
        as_copy: bool,
    ) {
        self.open_error = None;
        self.opening += 1;
        self.join = Some(crate::join_ui::JoinProgress::new(place));
        let cache = self.cache.clone();
        self.spawn(self.factory(client), move |a, tx| {
            let what = match version {
                Some(v) => format!("place {place} version {v}"),
                None => format!("place {place}"),
            };
            let _ = tx.send(Msg::Log(Level::Info, format!("Opening {what} from Roblox…")));
            // A cached Team Create place opens as a copy: the live session
            // can't be what's on disk.
            let from_cache = |why: String| -> Option<Opened> {
                let (mut origin, bytes, path) = cache.load(place, version)?;
                origin.local_copy = as_copy || origin.team_create == Some(true);
                Some(Opened {
                    origin,
                    bytes,
                    offline: Some(why),
                    cache_file: Some(path),
                })
            };
            let Some(a) = a else {
                let msg = match from_cache("signed out".into()) {
                    Some(o) => Msg::Opened(Box::new(o)),
                    None => Msg::OpenFailed(format!("Could not open {what}: {}", Self::signed_out())),
                };
                let _ = tx.send(msg);
                return;
            };
            let refused = |e: &str| e.contains("won't give") || e.contains("has no place");
            let mut origin = match a.resolve(place) {
                Ok(o) => o,
                Err(e) => {
                    let msg = match (!refused(&e.0)).then(|| from_cache(e.0.clone())).flatten() {
                        Some(o) => Msg::Opened(Box::new(o)),
                        None => Msg::OpenFailed(format!("Could not open {what}: {e}")),
                    };
                    let _ = tx.send(msg);
                    return;
                }
            };
            let st = a.team_create(origin.universe_id);
            origin.team_create = st.enabled;
            let _ = tx.send(Msg::Status(origin.universe_id, st.clone()));
            if open_mode(st.enabled, as_copy) == OpenMode::JoinTeamCreate {
                // Live Team Create sessions count as opened too.
                let _ = cache.remember(&origin);
                let _ = tx.send(Msg::TeamCreateOn(Box::new(origin)));
                return;
            }
            origin.local_copy = as_copy && st.enabled == Some(true);
            let opened = match a.download(place, version) {
                Ok((bytes, v)) => {
                    origin.version = v;
                    let cache_file = cache.store(&origin, &bytes).ok();
                    Opened {
                        origin,
                        bytes,
                        offline: None,
                        cache_file,
                    }
                }
                Err(e) => {
                    // Roblox refused (no access, no such place): say so. A
                    // network failure falls back to the cache.
                    match (!refused(&e.0)).then(|| from_cache(e.0.clone())).flatten() {
                        Some(o) => o,
                        None => {
                            let _ = tx.send(Msg::OpenFailed(format!("Could not open {what}: {e}")));
                            return;
                        }
                    }
                }
            };
            let _ = cache.remember(&opened.origin);
            let _ = tx.send(Msg::Opened(Box::new(opened)));
        });
    }

    /// Saves the open cloud document as a new version (Saved or
    /// Published), after the version check; `overwrite` skips the check.
    pub fn start_save(
        &mut self,
        client: Option<crate::assets::SharedClient>,
        origin: CloudOrigin,
        bytes: Vec<u8>,
        revision: u64,
        vt: VersionType,
        overwrite: bool,
    ) {
        self.save_error = None;
        self.conflict = None;
        self.spawn(self.factory(client), move |a, tx| {
            let Some(a) = a else {
                let _ = tx.send(Msg::SaveFailed(format!(
                    "Save to Roblox needs a session: {}",
                    Self::signed_out()
                )));
                return;
            };
            let _ = tx.send(Msg::Log(
                Level::Info,
                format!(
                    "{} place {} (version {} opened; checking Roblox's version first)…",
                    if vt == VersionType::Published { "Publishing" } else { "Saving" },
                    origin.place_id,
                    origin.version
                ),
            ));
            match a.save_place(&origin, &bytes, false, vt, overwrite) {
                Ok(SaveOutcome::Saved { version }) => {
                    let _ = tx.send(Msg::Saved { version, revision, vt, bytes });
                }
                Ok(SaveOutcome::Conflict { opened, server }) => {
                    let _ = tx.send(Msg::Conflict { opened, server, vt });
                }
                Err(e) => {
                    let _ = tx.send(Msg::SaveFailed(e.0));
                }
            }
        });
    }

    pub fn take_opened(&mut self) -> Option<Opened> {
        self.opened.take()
    }
}

impl Editor {
    /// The editor's side of the cloud jobs, once a frame.
    pub fn cloud_poll(&mut self, ctx: &egui::Context) {
        let checked = !matches!(self.account.state, crate::account_ui::SessionState::Checking(_));
        if checked {
            if let Some((p, v, copy)) = self.cloud.open_when_ready.take() {
                self.perform_open_cloud(p, v, copy);
            }
            if let Some(tab) = self.cloud.picker_when_ready.take() {
                self.cloud.open = true;
                self.cloud.tab = tab;
                let client = self.account.shared_client();
                self.cloud.refresh(client, tab);
            }
        }
        while let Ok(m) = self.cloud.rx.try_recv() {
            match m {
                Msg::Log(l, t) => self.push_log(l, t),
                Msg::Page(src, page, r) => {
                    if let Some(p) = self.cloud.lists.get_mut(&src) {
                        p.arrived(page, r);
                    }
                }
                Msg::Groups(r) => {
                    if let Ok(g) = &r {
                        if self.cloud.group_sel.is_none() {
                            self.cloud.group_sel = g.first().map(|g| g.id);
                        }
                    }
                    self.cloud.groups = Some(r);
                }
                Msg::Places(u, r) => {
                    self.cloud.places.insert(u, r);
                }
                Msg::Icon(u, img) => self.cloud.icon_images.push((u, img)),
                Msg::Versions(p, r) => {
                    self.cloud.versions.insert(p, r);
                }
                Msg::Opened(_) | Msg::TeamCreateOn(_) | Msg::OpenFailed(_) if self.cloud.open_cancelled > 0 => {
                    self.cloud.open_cancelled -= 1;
                    self.cloud.opening = self.cloud.opening.saturating_sub(1);
                }
                Msg::Opened(o) => {
                    self.cloud.opening = self.cloud.opening.saturating_sub(1);
                    self.cloud.opened = Some(*o);
                    self.cloud.join = None;
                }
                Msg::TeamCreateOn(o) => {
                    // As in Studio: a Team Create place opens by joining
                    // its live session, without asking.
                    self.cloud.opening = self.cloud.opening.saturating_sub(1);
                    let j = self.cloud.join.get_or_insert_with(|| crate::join_ui::JoinProgress::new(o.place_id));
                    j.found(o.universe_id, &o.name, &o.experience);
                    let api = self.cloud.factory(self.account.shared_client());
                    self.cloud.request_icons(api, vec![o.universe_id]);
                    self.info(format!(
                        "Team Create is on for {} (universe {}): joining the live session",
                        o.experience, o.universe_id
                    ));
                    self.cloud.live_origin = Some((*o).clone());
                    self.open_team_create(o.universe_id, o.place_id);
                }
                Msg::OpenFailed(e) => {
                    self.cloud.opening = self.cloud.opening.saturating_sub(1);
                    if let Some(j) = self.cloud.join.as_mut() {
                        j.fail(e.clone());
                    }
                    self.error(e.clone());
                    self.cloud.open_error = Some(e);
                }
                Msg::Status(u, st) => {
                    if self.doc.cloud.as_ref().is_some_and(|o| o.universe_id == u) {
                        let line = st.describe();
                        if st.enabled == Some(true) || st.can_edit() == Some(false) {
                            self.warn(line);
                        } else {
                            self.info(line);
                        }
                    }
                    self.cloud.status = Some((u, st));
                }
                Msg::Saved { version, revision, vt, bytes } => {
                    if let Some(o) = self.doc.cloud.as_mut() {
                        o.version = version;
                        let o = o.clone();
                        let _ = self.cloud.cache.store(&o, &bytes);
                        let _ = self.cloud.cache.remember(&o);
                        self.doc.mark_saved_at(revision);
                        self.info(format!(
                            "{} to Roblox: version {version} of {} (place {})",
                            if vt == VersionType::Published { "Published" } else { "Saved" },
                            o.name,
                            o.place_id
                        ));
                    }
                    self.cloud.last_saved = Some((version, vt));
                }
                Msg::Conflict { opened, server, vt } => {
                    self.warn(format!(
                        "Not saved: someone saved this place on Roblox since it was opened (opened version {opened}, Roblox has version {server})"
                    ));
                    self.cloud.conflict = Some(Conflict { opened, server, vt });
                }
                Msg::SaveFailed(e) => {
                    self.error(format!("Save to Roblox failed: {e}"));
                    self.cloud.save_error = Some(e);
                }
                Msg::Done => self.cloud.jobs = self.cloud.jobs.saturating_sub(1),
            }
        }
        for (u, img) in std::mem::take(&mut self.cloud.icon_images) {
            let tex = ctx.load_texture(format!("experience-icon-{u}"), img, egui::TextureOptions::LINEAR);
            self.cloud.icons.insert(u, tex);
        }
        if let Some(o) = self.cloud.take_opened() {
            self.set_cloud_document(o);
        }
        self.poll_live_team_create(ctx);
        if self.cloud.jobs == 0 {
            self.cloud.opening = 0;
        }
        if !self.place_loading() {
            if let Some(mode) = self.cloud.play_when_loaded.take() {
                self.start_play(mode);
            }
        }
        // The window title, as Studio's: the place's name.
        let dirty = if self.doc.dirty() { " •" } else { "" };
        let title = format!("{}{dirty} - rustudio", if self.doc.title.is_empty() { "Untitled" } else { &self.doc.title });
        if title != self.cloud.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.cloud.window_title = title;
        }
    }

    fn set_cloud_document(&mut self, o: Opened) {
        let Opened { origin, bytes, offline, cache_file } = o;
        let title = if origin.local_copy {
            format!("{} (local copy)", origin.name)
        } else {
            origin.name.clone()
        };
        match Document::from_place_bytes(&bytes, title) {
            Ok(mut d) => {
                d.cloud = Some(origin.clone());
                let n = d.dom.descendants().count();
                self.set_document(d);
                self.cloud.open = false;
                if self.cloud.status.as_ref().is_some_and(|(u, _)| *u != origin.universe_id) {
                    self.cloud.status = None;
                }
                self.info(format!(
                    "Opened {} from Roblox: place {} version {} of {} (universe {}, by {}), {n} instances",
                    origin.name,
                    origin.place_id,
                    origin.version,
                    origin.experience,
                    origin.universe_id,
                    origin.creator_label()
                ));
                if origin.local_copy {
                    self.warn(format!(
                        "This is a local copy of {}'s last saved version (Team Create is on; others may be editing it live). Ctrl+S saves it to a file; File > Save to Roblox asks first",
                        origin.experience
                    ));
                }
                if let Some(why) = offline {
                    self.warn(format!(
                        "Roblox couldn't be asked ({why}): this is the copy cached at {}; saving checks Roblox's version first",
                        cache_file.map(|p| p.display().to_string()).unwrap_or_default()
                    ));
                }
            }
            Err(e) => {
                let msg = format!("Place {} was downloaded but could not be read: {e}", origin.place_id);
                self.error(msg.clone());
                self.cloud.open_error = Some(msg);
            }
        }
    }

    /// Opens a place from Roblox (after the unsaved-changes question);
    /// `as_copy`: a local copy of a Team Create place.
    pub fn open_cloud(&mut self, place: u64, version: Option<u64>, as_copy: bool) {
        rustudio_account::session::set_place_context(place);
        self.request(PendingAction::OpenCloud(place, version, as_copy));
    }

    pub fn perform_open_cloud(&mut self, place: u64, version: Option<u64>, as_copy: bool) {
        let client = self.account.shared_client();
        self.cloud.start_open(client, place, version, as_copy);
    }

    /// Join the server's live DataModel, with network work on its own thread.
    pub fn open_team_create(&mut self, universe: u64, place: u64) {
        rustudio_account::session::set_place_context(place);
        if self.cloud.join.is_none() {
            let mut j = crate::join_ui::JoinProgress::new(place);
            j.found(universe, "", "");
            self.cloud.join = Some(j);
        }
        match rustudio_net::live::LiveSession::connect(universe, place) {
            Ok(session) => {
                self.cloud.open_error = None;
                self.cloud.live = Some(LiveView {
                    session, model: rustudio_net::model::LiveModel::new(),
                    origin: self.cloud.live_origin.take(), place, universe, document_id: None, requested_document_id: self.doc.id,
                    owner_user: None, own_player: None, publish_service: None, save_pending: None, sync_failed: false, drafts: crate::live_drafts::DraftStore::for_place(place), script_service: None, script_configuration_sent: false, next_script_request: 1, script_commits: HashMap::new(),
                    last_presence: std::time::Instant::now(), last_camera: None, locally_deleted: HashSet::new(), collaborators: HashSet::new(), players: HashMap::new(), selections: HashMap::new(), last_selection: None, last_activity: std::time::Instant::now(), last_active: None,
                });
                self.info(format!("Joining live Team Create place {place}…"));
            }
            Err(e) => {
                if let Some(j) = self.cloud.join.as_mut() { j.fail(e.to_string()); }
                self.error(e.to_string()); self.cloud.open_error = Some(e.to_string());
            }
        }
    }

    /// (universe, place) of the live Team Create session this document is.
    /// A Team Create join is under way and its DataModel isn't loaded yet.
    pub fn live_join_pending(&self)->bool { self.cloud.live.as_ref().is_some_and(|l| l.document_id.is_none()) }
    /// A place being opened isn't the document yet: an open waiting for the
    /// session, downloading, unread, or
    /// a live join before its DataModel is in. Its PlaceId / GameId (set at
    /// Play from the document's origin) aren't known until then.
    pub fn place_loading(&self)->bool {
        self.cloud.open_when_ready.is_some()
            || self.cloud.opening > 0
            || self.cloud.opened.is_some()
            || self.live_join_pending()
    }
    /// Play asked for while [`Self::place_loading`]: held until the place is in.
    /// True when it was held.
    pub fn defer_play_until_loaded(&mut self, mode: crate::runtime::RunMode)->bool {
        if !self.place_loading() { return false; }
        if self.cloud.play_when_loaded.replace(mode).is_none() {
            self.info("Play starts once the place has finished loading".to_string());
        }
        true
    }
    pub fn live_team_create_ids(&self)->Option<(u64,u64)> {
        self.cloud.live.as_ref().filter(|l|l.document_id==Some(self.doc.id)).map(|l|(l.universe,l.place))
    }

    pub fn live_team_create_place(&self)->Option<u64> {
        self.cloud.live.as_ref().filter(|l|l.document_id==Some(self.doc.id)).map(|l|l.place)
    }

    pub fn live_script_has_draft(&self,reference:rbx_types::Ref)->Option<bool> {
        let live=self.cloud.live.as_ref().filter(|l|l.document_id==Some(self.doc.id))?;
        let id=live.model.server_id(reference)?;
        Some(live.drafts.get(id).is_some())
    }

    pub fn live_draft_toolbar(&mut self,ui:&mut egui::Ui,reference:rbx_types::Ref) {
        let Some(live)=self.cloud.live.as_ref().filter(|l|l.document_id==Some(self.doc.id)) else {return;};
        let Some(id)=live.model.server_id(reference) else {return;};
        let Some(draft)=live.drafts.get(id).cloned() else {
            ui.label(egui::RichText::new("Live script edits are kept as private drafts until you commit them").small().color(theme::TEXT_DIM));
            return;
        };
        let conflict=live.drafts.conflict(id);
        let committing=live.script_commits.values().any(|(script,_,_)| *script==id);
        let server=live.drafts.source(id).unwrap_or("").to_string();
        let mut action=0;
        ui.horizontal(|ui| {
            ui.label(if conflict {"Draft conflicts with a newer server version"} else if draft.submitted {"Draft sent to server"} else {"Local draft"});
            if !draft.submitted && ui.add_enabled(!committing,egui::Button::new(if conflict {"Replace server with my draft"} else {"Commit draft"})).clicked() {action=1;}
            if ui.add_enabled(!committing,egui::Button::new("Use server version")).clicked() {action=2;}
        });
        if conflict {egui::CollapsingHeader::new("Compare server and draft").show(ui,|ui| {
            ui.columns(2,|cols| {
                cols[0].label("Server");egui::ScrollArea::both().id_salt(("server_source_diff",id)).max_height(180.0).show(&mut cols[0],|ui|{ui.monospace(&server);});
                cols[1].label("My draft");egui::ScrollArea::both().id_salt(("draft_source_diff",id)).max_height(180.0).show(&mut cols[1],|ui|{ui.monospace(&draft.text);});
            });
        });}
        if action==1 {
            let live=self.cloud.live.as_mut().unwrap();
            let result=live.drafts.commit_text(id,conflict).and_then(|text| {
                let inst=self.doc.dom.get_by_ref(reference).ok_or("The script was removed")?;
                let guid=inst.properties.get(&"ScriptGuid".into()).and_then(|v| if let rbx_types::Variant::String(g)=v {Some(g.clone())} else {None});
                if let (Some(service),Some(guid))=(live.script_service,guid) {
                    if !live.script_configuration_sent {
                        live.session.send_event(service,"LiveScriptingService".into(),"LiveEditingClientConfiguration".into(),vec![rustudio_net::scripts::client_configuration()]).map_err(|e|e.to_string())?;
                        live.script_configuration_sent=true;
                    }
                    let request=live.next_script_request;live.next_script_request=live.next_script_request.checked_add(1).ok_or("Script request counter exhausted")?;
                    let base=live.drafts.source(id).ok_or("The server source is unavailable")?;
                    let args=rustudio_net::scripts::replace(request,&guid,base,&text);
                    live.session.send_event(service,"LiveScriptingService".into(),"SendAtomicOperation".into(),args).map_err(|e|e.to_string())?;
                    live.script_commits.insert(request,(id,std::time::Instant::now(),text));
                    Ok(true)
                } else {
                    live.session.send_property(id,inst.class.to_string(),"Source".into(),rustudio_net::replicator::PropValue::String(text)).map_err(|e|e.to_string())?;
                    live.drafts.submitted(id);Ok(false)
                }
            });
            match result {Ok(true)=>self.info("Waiting for the server to confirm the script commit"),Ok(false)=>self.info("Script draft sent; its recovery copy is retained"),Err(error)=>self.error(error)}
        } else if action==2 {self.discard_live_draft(reference);}
    }

    pub fn discard_live_draft(&mut self,reference:rbx_types::Ref) {
        let Some(live)=self.cloud.live.as_mut() else {return;};
        let Some(id)=live.model.server_id(reference) else {return;};
        if let Some(source)=live.drafts.discard(id) {
            self.doc.set_props("Discard script draft",vec![(reference,"Source".into(),rbx_types::Variant::String(source.clone()))],None);
            if let Some(buf)=self.scripts.bufs.get_mut(&reference) {buf.text=source;}
        }
    }

    fn sync_live_tree(&mut self, live: &mut LiveView, reference: rbx_types::Ref) -> Result<(), String> {
        let Some(inst) = self.doc.dom.get_by_ref(reference) else {
            if !live.locally_deleted.insert(reference) { return Ok(()); }
            if let Some(id) = live.model.server_id(reference) {
                live.session.send_delete(id).map_err(|e| e.to_string())?;
            }
            return Ok(());
        };
        let parent = live.model.server_id(inst.parent()).ok_or("The live parent has not synchronized yet")?;
        if !live.locally_deleted.remove(&reference) {
            if let Some(id) = live.model.server_id(reference) {
                return live.session.send_reparent(id, parent).map_err(|e| e.to_string());
            }
        }
        // Allocate the entire subtree first so references between its nodes resolve.
        let mut nodes = vec![reference];
        let mut index = 0;
        while index < nodes.len() {
            let r = nodes[index]; index += 1;
            if let Some(i) = self.doc.dom.get_by_ref(r) { nodes.extend_from_slice(i.children()); }
            let id = live.session.allocate_id().map_err(|e| e.to_string())?;
            live.model.bind_local(id, r);
        }
        for r in nodes {
            let inst = self.doc.dom.get_by_ref(r).ok_or("A new live instance disappeared")?;
            let id = live.model.server_id(r).unwrap();
            let parent = live.model.server_id(inst.parent()).ok_or("A new live parent is unavailable")?;
            let mut props = vec![("Name".into(), rustudio_net::replicator::PropValue::String(inst.name.clone()))];
            for (name, value) in &inst.properties {
                let value=live_value(value,&live.model).ok_or_else(||format!("Team Create cannot synchronize {}.{name} yet",inst.class))?;
                props.push((name.to_string(),value));
            }
            live.session.send_create(id, inst.class.to_string(), parent, props).map_err(|e| e.to_string())?;
            if let Some(rbx_types::Variant::String(source))=inst.properties.get(&"Source".into()) {
                let guid=inst.properties.get(&"ScriptGuid".into()).and_then(|v| if let rbx_types::Variant::String(g)=v {Some(g.as_str())} else {None});
                live.drafts.register(id,guid,source.clone())?;
            }
        }
        Ok(())
    }

    fn poll_live_team_create(&mut self, ctx: &egui::Context) {
        use rustudio_net::live::LiveEvent;
        let Some(mut live) = self.cloud.live.take() else { return; };
        if live.document_id.unwrap_or(live.requested_document_id) != self.doc.id { return; }
        if live.document_id.is_some() {
            if let crate::doc::Marks::Some(mut marks) = self.doc.marks_for(crate::doc::Reader::TeamCreate) {
                marks.sort_by_key(|mark| matches!(mark, crate::doc::Mark::Tree(_)));
                let mut sent = HashSet::new();
                let mut sent_trees = HashSet::new();
                for mark in marks {
                    match mark {
                        crate::doc::Mark::Prop(reference, name) if sent.insert((reference, name)) => {
                            let Some(id) = live.model.server_id(reference) else { continue; };
                            let Some(inst) = self.doc.dom.get_by_ref(reference) else { continue; };
                            if name.as_str()=="Source" {
                                if let Some(rbx_types::Variant::String(text))=self.doc.prop(reference,"Source") {
                                    if let Err(error)=live.drafts.edit(id,text) {self.warn(error);}
                                }
                                continue;
                            }
                            let value = if name.as_str() == "Name" { Some(rbx_types::Variant::String(inst.name.clone())) }
                                else { self.doc.prop(reference, name.as_str()) };
                            if let Some(value) = value.and_then(|v| live_value(&v, &live.model)) {
                                if let Err(e) = live.session.send_property(id, inst.class.to_string(), name.to_string(), value) { live.sync_failed=true;self.warn(e.to_string()); }
                            } else { live.sync_failed=true;self.warn(format!("Team Create cannot synchronize {name} yet")); }
                        }
                        crate::doc::Mark::Tree(reference) if sent_trees.insert(reference) => {
                            if let Err(error) = self.sync_live_tree(&mut live, reference) { live.sync_failed=true;self.warn(error); }
                        }
                        _ => {}
                    }
                }
            }
        }
        if live.sync_failed && live.save_pending.take().is_some() {
            self.error("Save blocked because some local edits have not synchronized; retain a local copy and reopen the live place");
        }
        if let Some(save) = live.save_pending.as_mut() {
            if save.sent.is_none() {
                if let Some(service) = live.publish_service {
                    use rustudio_net::replicator::PropValue as P;
                    let options=P::Dictionary(vec![("publish".into(),P::Bool(save.vt==VersionType::Published)),("upload_to_edge".into(),P::Bool(false))]);
                    match live.session.send_event(service,"TeamCreatePublishService".into(),"TeamCreatePlaceUploadRequest".into(),vec![options]) {
                        Ok(()) => save.sent=Some(std::time::Instant::now()),
                        Err(error) => {self.error(error.to_string());live.save_pending=None;}
                    }
                } else {self.error("The live server does not expose saving yet");live.save_pending=None;}
            } else if save.sent.is_some_and(|at| at.elapsed() > std::time::Duration::from_secs(120)) {
                self.error("The Team Create server has not confirmed the save; check Version History before retrying");
                live.save_pending=None;
            }
        }
        let mut keep = true;
        let mut changed = false;
        // What the server's ops touched, so the scene, physics and effects
        // update only that instead of rebuilding everything (a full rebuild
        // per batch reset every particle and hitched ~twice a second, when a
        // collaborator's camera presence arrived).
        let mut live_marks: Vec<crate::doc::Mark> = Vec::new();
        let mut live_rebuild = false;
        let was_dirty = self.doc.dirty();
        // Limit UI work per frame while retaining the stream's batch ordering.
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_millis(12) {
            let event = match live.session.events.try_recv() {
                Ok(event) => event,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.warn("Team Create connection ended"); keep = false; break;
                }
            };
            match event {
                LiveEvent::Stage(stage) => {
                    if live.document_id.is_none() {
                        if let Some(j) = self.cloud.join.as_mut() { j.stage(stage); }
                    }
                }
                LiveEvent::Waiting(attempt, message) => {
                    if let Some(j) = self.cloud.join.as_mut() { j.waiting(attempt, message); }
                }
                LiveEvent::Batch(ops) => {
                    let joining = live.document_id.is_none();
                    if live.document_id.is_some() {
                        std::mem::swap(live.model.dom_mut(), &mut self.doc.dom);
                    }
                    for op in ops {
                        if let rustudio_net::replicator::RepOp::Event {id,name,args} = &op {
                            if Some(*id)==live.script_service && name=="LiveScriptingService.SendAtomicUpdateResponse" {
                                use rustudio_net::replicator::PropValue as P;
                                if let [P::Int(request),P::Enum(status)]=args.as_slice() {
                                    if let Some((script,_,text))=live.script_commits.remove(request) {
                                        match status {
                                            0=>{live.drafts.confirmed(script,text);self.info("Script draft committed by the Team Create server");},
                                            2=>{live.drafts.rejected(script);self.warn("The script changed on the server; your draft was retained for comparison");},
                                            _=>self.error("The server rejected the script commit; your draft was retained"),
                                        }
                                    }
                                }
                            }

                            if name=="Player.CloudEditSelectionChanged" {
                                if let Some(rustudio_net::replicator::PropValue::Array(values))=args.first() {
                                    live.selections.insert(*id,values.iter().filter_map(|v| if let rustudio_net::replicator::PropValue::Ref(Some(id))=v {Some(*id)} else {None}).take(512).collect());
                                }
                            }
                            if Some(*id)==live.publish_service && name=="TeamCreatePublishService.TeamCreatePlaceUploadResponse" {
                                if let Some(save)=live.save_pending.take() {
                                    if matches!(args.first(),Some(rustudio_net::replicator::PropValue::Bool(true))) {
                                        if let Some(rustudio_net::replicator::PropValue::Dictionary(metadata))=args.get(1) {
                                            if let Some(version)=metadata.iter().find_map(|(key,value)| match (key.as_str(),value) {
                                                ("version_number",rustudio_net::replicator::PropValue::Int(v)) if *v>0 => Some(*v as u64),
                                                _=>None,
                                            }) {if let Some(origin)=self.doc.cloud.as_mut() {origin.version=version;}}
                                        }
                                        self.doc.mark_saved_at(save.revision);
                                        self.info(if save.vt==VersionType::Published {"Team Create server confirmed publication"} else {"Team Create server confirmed the save"});
                                    } else {self.error("The Team Create server could not save the place");}
                                }
                            }
                            if Some(*id)==live.publish_service && name=="TeamCreatePublishService.TeamCreateErrorStatus" {
                                use rustudio_net::replicator::PropValue as P;
                                match args.first() {
                                    Some(P::Enum(1))=>self.warn("The Team Create place is approaching the save size limit"),
                                    Some(P::Enum(0 | 2))=>{if live.save_pending.take().is_some() {self.error("The Team Create server reported a save error");}},
                                    _=>{},
                                }
                            }
                        }
                        if let rustudio_net::replicator::RepOp::NewInstance { id, class, props, .. } = &op {
                            if class=="Player" {
                                if let Some(user)=props.iter().find_map(|(k,v)| if k=="UserId" {if let rustudio_net::replicator::PropValue::Int(user)=v {Some(*user as u64)} else {None}} else {None}) {live.players.insert(*id,user);}
                            }
                            if class == "Player" && props.iter().any(|(k, v)| k == "UserId" && matches!(v, rustudio_net::replicator::PropValue::Int(uid) if Some(*uid as u64) == live.owner_user)) { live.own_player = Some(*id); }
                        }
                        let source_id=match &op {
                            rustudio_net::replicator::RepOp::NewInstance {id,props,..} => {
                                let source=props.iter().find_map(|(k,v)| (k=="Source").then(|| live_source_text(v)).flatten());
                                if let Some(source)=source {
                                    let guid=props.iter().find_map(|(k,v)| if k=="ScriptGuid" {if let rustudio_net::replicator::PropValue::String(g)=v {Some(g.as_str())} else {None}} else {None});
                                    if let Err(error)=live.drafts.register(*id,guid,source) {self.warn(error);}
                                    Some(*id)
                                } else {None}
                            }
                            rustudio_net::replicator::RepOp::ChangeProperty {id,name,value} if name=="Source" => {
                                if let Some(source)=live_source_text(value) {live.drafts.receive(*id,source);}
                                Some(*id)
                            }
                            _=>None,
                        };
                        let collaborator = match &op { rustudio_net::replicator::RepOp::NewInstance {id, class, ..} if class=="Collaborator" => Some(*id), _ => None };
                        use rustudio_net::replicator::RepOp as Op;
                        let touched = match &op {
                            Op::NewInstance { id, .. } | Op::Reparent { id, .. } => Some((*id, None)),
                            Op::ChangeProperty { id, name, .. } => Some((*id, Some(name.clone()))),
                            Op::DeleteInstance { .. } => { live_rebuild = true; None }
                            Op::JoinDataBegin => { live_rebuild = true; None }
                            Op::Presence { .. } | Op::Event { .. } | Op::Unhandled { .. } | Op::NetAsset { .. } => None,
                        };
                        live.model.apply(op);
                        // Unions whose geometry just arrived on the 0xa5 channel.
                        for r in live.model.take_resolved() {
                            live_marks.push(crate::doc::Mark::Prop(r, rbx_dom_weak::ustr("SolidMeshHolder")));
                        }
                        // The time of day as the server sends it (2026-11-05: rustudio
                        // joined Place B at ~02:xx while the place saves 14:00).
                        if let Some((id, Some(prop))) = &touched {
                            if prop == "TimeOfDay" || prop == "ClockTime" {
                                if let Some(inst) = live.model.reference(*id).and_then(|r| live.model.dom_mut().get_by_ref(r)) {
                                    let value = inst.properties.get(&rbx_dom_weak::ustr(prop)).map(|v| format!("{v:?}")).unwrap_or_default();
                                    let line = format!("Team Create: {} {:?}.{prop} = {value}", inst.class, inst.name);
                                    self.info(line);
                                }
                            }
                        }
                        if let Some((id, prop)) = touched {
                            match live.model.reference(id).filter(|r| live.model.dom_mut().get_by_ref(*r).is_some()) {
                                Some(r) => live_marks.push(match prop {
                                    Some(name) => crate::doc::Mark::Prop(r, rbx_dom_weak::ustr(&name)),
                                    None => crate::doc::Mark::Tree(r),
                                }),
                                // A NewInstance parked until its parent arrives
                                // touches nothing yet; a pending drain later
                                // shows up as its parent's own NewInstance.
                                None => {}
                            }
                        }
                        if let Some(id)=source_id {
                            if let Some(draft)=live.drafts.get(id).filter(|d| !d.submitted) {
                                let text=draft.text.clone();
                                if let Some(reference)=live.model.reference(id) {
                                    if let Some(inst)=live.model.dom_mut().get_by_ref_mut(reference) {inst.properties.insert("Source".into(),rbx_types::Variant::String(text));}
                                }
                            }
                        }
                        if let Some(reference) = collaborator.and_then(|id| live.model.reference(id)) { live.collaborators.insert(reference); }
                    }
                    if live.document_id.is_some() {
                        std::mem::swap(live.model.dom_mut(), &mut self.doc.dom);
                        changed = true;
                    }
                    if joining {
                        let n = live.model.instance_count();
                        if let Some(j) = self.cloud.join.as_mut() { j.data(n); }
                    }
                }
                LiveEvent::Ready => {
                    self.cloud.join = None;
                    if live.document_id.is_none() {
                        let dom = std::mem::replace(live.model.dom_mut(), rbx_dom_weak::WeakDom::new(
                            rbx_dom_weak::InstanceBuilder::new("DataModel")));
                        let title = live.origin.as_ref().map(|o| o.name.clone())
                            .unwrap_or_else(|| format!("Place {}", live.place));
                        let mut document = Document::from_play_dom(dom, format!("{title} (Live)"));
                        document.cloud = live.origin.clone();
                        self.set_document(document);
                        live.document_id = Some(self.doc.id);
                        self.doc.marks_for(crate::doc::Reader::TeamCreate);
                        live.collaborators = self.doc.dom.descendants().filter(|inst| inst.class.as_str()=="Collaborator").map(|inst| inst.referent()).collect();
                        live.publish_service = self.doc.dom.descendants().find(|i| i.class.as_str()=="TeamCreatePublishService").and_then(|i| live.model.server_id(i.referent()));
                        live.script_service=self.doc.dom.descendants().find(|i|i.class.as_str()=="LiveScriptingService").and_then(|i|live.model.server_id(i.referent()));
                        self.cloud.open = false;
                        self.info(format!("Joined live Team Create place {}: {} instances", live.place, live.model.instance_count()));
                    }
                }
                LiveEvent::Identity(user) => live.owner_user = Some(user),
                LiveEvent::EditRejected(message) => {live.sync_failed=true;live.save_pending=None;self.error(format!("Team Create: {message}"));},
                LiveEvent::Notice(message) => self.warn(format!("Team Create: {message}")),
                LiveEvent::Error(e) => {
                    if live.document_id.is_none() { if let Some(j) = self.cloud.join.as_mut() { j.fail(e.clone()); } }
                    self.error(format!("Team Create: {e}")); self.cloud.open_error = Some(e); keep = false; break;
                }
                LiveEvent::Disconnected(e) => {
                    if live.document_id.is_none() { if let Some(j) = self.cloud.join.as_mut() { j.fail(e.clone()); } }
                    self.warn(format!("Team Create disconnected: {e}")); keep = false; break;
                }
            }
        }
        if changed {
            if live_rebuild || live_marks.len() > crate::mech::JOURNAL_READER_LAG / 2 {
                self.doc.mark_all();
            } else {
                for m in live_marks {
                    match m {
                        crate::doc::Mark::Prop(r, name) => self.doc.mark_prop(r, name),
                        crate::doc::Mark::Tree(r) => self.doc.mark_tree(r),
                    }
                }
            }
            self.doc.marks_for(crate::doc::Reader::TeamCreate); // Do not echo server changes.
            if !was_dirty { self.doc.mark_saved_at(self.doc.revision); }
        }
        if ctx.input(|i| !i.raw.events.is_empty()) {live.last_activity=std::time::Instant::now();}
        if live.document_id.is_some() && live.last_presence.elapsed() >= std::time::Duration::from_millis(200) {
            let camera = [self.cam.pos.x, self.cam.pos.y, self.cam.pos.z, self.cam.yaw, self.cam.pitch];
            if live.last_camera != Some(camera) {
                if let Some(id) = live.own_player {
                    let right = self.cam.right(); let up = self.cam.up(); let back = -self.cam.forward();
                    let value = rustudio_net::replicator::PropValue::CFrame { pos: self.cam.pos.to_array(),
                        rot: [right.x, up.x, back.x, right.y, up.y, back.y, right.z, up.z, back.z] };
                    if live.session.send_property(id, "Player".into(), "CloudEditCameraCoordinateFrame".into(), value).is_ok() { live.last_camera = Some(camera); }
                }
            }
            if let Some(player)=live.own_player {
                if live.last_selection.as_ref()!=Some(&self.doc.selection) {
                    let refs=self.doc.selection.iter().filter_map(|r|live.model.server_id(*r)).take(512).map(|id|rustudio_net::replicator::PropValue::Ref(Some(id))).collect();
                    if live.session.send_event(player,"Player".into(),"CloudEditSelectionChanged".into(),vec![rustudio_net::replicator::PropValue::Array(refs)]).is_ok() {live.last_selection=Some(self.doc.selection.clone());}
                }
                let active=live.last_activity.elapsed()<std::time::Duration::from_secs(300);
                if live.last_active!=Some(active) && live.session.send_property(player,"Player".into(),"CloudEditPlayerActive".into(),rustudio_net::replicator::PropValue::Bool(active)).is_ok() {live.last_active=Some(active);}
            }
            live.last_presence = std::time::Instant::now();
        }
        if live.document_id.is_some() && self.hud.viewport_drawn && self.viewport_rect.is_positive() {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("live_collaborator_cameras"))).with_clip_rect(self.viewport_rect);
            for reference in &live.collaborators {
                let Some(inst) = self.doc.dom.get_by_ref(*reference) else { continue; };
                if matches!(inst.properties.get(&"UserId".into()), Some(rbx_types::Variant::Int64(id)) if Some(*id as u64)==live.owner_user) { continue; }
                let color = match inst.properties.get(&"CollaboratorColor3".into()) {
                    Some(rbx_types::Variant::Color3(c)) => egui::Color32::from_rgb((c.r*255.0) as u8,(c.g*255.0) as u8,(c.b*255.0) as u8),
                    _ => egui::Color32::from_rgb(80,190,255),
                };
                let user=inst.properties.get(&"UserId".into()).and_then(|v|if let rbx_types::Variant::Int64(user)=v {Some(*user as u64)} else {None});
                if let Some(player)=live.players.iter().find_map(|(player,u)| (Some(*u)==user).then_some(*player)) {
                    if let Some(selection)=live.selections.get(&player) {
                        for id in selection {
                            if let Some(reference)=live.model.reference(*id) {
                                if let Some(part)=self.scene.get(reference) {self.draw_box(&painter,self.viewport_rect,&part.xf,part.size,egui::Stroke::new(1.5,color));}
                                else {for inst in self.doc.dom.descendants_of(reference).take(512) {if let Some(part)=self.scene.get(inst.referent()) {self.draw_box(&painter,self.viewport_rect,&part.xf,part.size,egui::Stroke::new(1.5,color));}}}
                            }
                        }
                    }
                }
                let Some(rbx_types::Variant::CFrame(frame)) = inst.properties.get(&"CFrame".into()) else { continue; };
                let world = glam::Vec3::new(frame.position.x, frame.position.y, frame.position.z);
                let Some(point) = self.cam.project(self.viewport_rect, world) else { continue; };
                if !self.viewport_rect.contains(point) { continue; }
                let name = match inst.properties.get(&"Username".into()) { Some(rbx_types::Variant::String(s)) => s.as_str(), _ => inst.name.as_str() };
                painter.rect_filled(egui::Rect::from_center_size(point, egui::vec2(12.0,9.0)), 2.0, color);
                painter.text(point+egui::vec2(0.0,11.0), egui::Align2::CENTER_TOP, name, egui::FontId::proportional(12.0), color);
            }
        }
        let timed_out:Vec<_>=live.script_commits.iter().filter_map(|(request,(_,sent,_))| (sent.elapsed()>std::time::Duration::from_secs(60)).then_some(*request)).collect();
        for request in timed_out {live.script_commits.remove(&request);self.warn("The server has not confirmed the script commit; the local draft is retained");}
        if let Err(error)=live.drafts.flush_due() {self.warn(error);}
        if keep { self.cloud.live = Some(live); ctx.request_repaint_after(std::time::Duration::from_millis(16)); }
    }

    /// Ctrl+S / Save to Roblox / Publish for a document opened from Roblox.
    pub fn save_cloud(&mut self, vt: VersionType, overwrite: bool) {
        if self.cloud.live.as_ref().is_some_and(|live| live.document_id==Some(self.doc.id)) {
            if self.play.is_some() {self.warn("Stop the play session before saving to Roblox");return;}
            let live=self.cloud.live.as_mut().unwrap();
            if live.sync_failed {self.error("Some local edits have not synchronized; retain a local copy and reopen the live place before saving");return;}
            if live.save_pending.is_some() {self.warn("A Team Create save is already waiting for the server");return;}
            if live.drafts.has_unsubmitted() {self.warn("Commit or discard your script drafts before saving the live place");return;}
            live.save_pending=Some(LiveSave {revision:self.doc.revision,vt,sent:None});
            self.info("Requesting a save from the Team Create server…");
            return;
        }
        let Some(origin) = self.doc.cloud.clone() else {
            return;
        };
        if self.play.is_some() {
            self.warn("Stop the play session before saving to Roblox");
            return;
        }
        match self.doc.encode_all(false) {
            Ok(bytes) => {
                let client = self.account.shared_client();
                let rev = self.doc.revision;
                self.cloud.start_save(client, origin, bytes, rev, vt, overwrite);
            }
            Err(e) => self.error(format!("Could not encode the place: {e}")),
        }
    }

    pub fn cloud_windows(&mut self, ctx: &egui::Context) {
        self.cloud_poll(ctx);
        self.live_collaborators_window(ctx);
        self.picker_window(ctx);
        self.place_info_window(ctx);
        self.conflict_modal(ctx);
        self.copy_save_modal(ctx);
        self.save_error_modal(ctx);
        self.join_modal(ctx);
    }

    fn live_collaborators_window(&mut self,ctx:&egui::Context) {
        if !self.cloud.live_collaborators_open {return;}
        let mut fly=None;
        let mut open_guid=None;
        let live=self.cloud.live.as_ref();
        let peers:Vec<_>=live.into_iter().flat_map(|live|live.collaborators.iter().filter_map(|r| {
            let inst=self.doc.dom.get_by_ref(*r)?;
            let user=match inst.properties.get(&"UserId".into()) {Some(rbx_types::Variant::Int64(v))=>*v as u64,_=>0};
            let own=Some(user)==live.owner_user;
            let name=match inst.properties.get(&"Username".into()) {Some(rbx_types::Variant::String(v))=>v.clone(),_=>inst.name.clone()};
            let frame=match inst.properties.get(&"CFrame".into()) {Some(rbx_types::Variant::CFrame(v))=>Some(*v),_=>None};
            let idle=matches!(inst.properties.get(&"IsIdle".into()),Some(rbx_types::Variant::Bool(true)));
            let guid=match inst.properties.get(&"CurDocGUID".into()) {Some(rbx_types::Variant::String(v))=>v.clone(),_=>String::new()};
            let line=match inst.properties.get(&"CurScriptLineNumber".into()) {Some(rbx_types::Variant::Int64(v))=>*v,_=>0};
            Some((name,own,frame,idle,guid,line))
        })).collect();
        let universe=live.map(|l|l.universe).or_else(||self.doc.cloud.as_ref().map(|o|o.universe_id));
        egui::Window::new("Live Collaborators").id(egui::Id::new("live_collaborators_window")).open(&mut self.cloud.live_collaborators_open).show(ctx,|ui| {
            if self.cloud.live.is_none() {ui.label("Open a Team Create place to join its live session.");}
            for (name,own,frame,idle,guid,line) in peers {
                ui.horizontal(|ui| {
                    ui.label(format!("{name}{}{}",if own {" (you)"} else {""},if idle {" · idle"} else {""}));
                    if !own && frame.is_some() && ui.small_button("Fly to").clicked() {fly=frame;}
                    if !guid.is_empty() && ui.small_button(format!("Script · line {line}")).clicked() {open_guid=Some(guid);}
                });
            }
            if let Some(universe)=universe {ui.hyperlink_to("Manage collaborators",format!("https://create.roblox.com/dashboard/creations/experiences/{universe}/collaborators"));}
        });
        if let Some(frame)=fly {
            self.cam.pos=glam::Vec3::new(frame.position.x,frame.position.y,frame.position.z);
            let m=frame.orientation;
            self.cam.set_rotation(glam::Mat3::from_cols_array(&[m.x.x,m.y.x,m.z.x,m.x.y,m.y.y,m.z.y,m.x.z,m.y.z,m.z.z]));
            self.cam.focus=self.cam.pos+self.cam.forward()*32.0;
        }
        if let Some(guid)=open_guid {
            if let Some(reference)=self.doc.dom.descendants().find(|i| matches!(i.properties.get(&"ScriptGuid".into()),Some(rbx_types::Variant::String(g)) if *g==guid)).map(|i|i.referent()) {self.open_script.push(reference);}
        }
    }

    fn conflict_modal(&mut self, ctx: &egui::Context) {
        let Some(c) = self.cloud.conflict else { return };
        let mut pick: Option<u8> = None;
        egui::Modal::new(egui::Id::new("cloud_conflict")).show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.label(RichText::new("Someone saved this place since you opened it").strong());
            ui.label(format!(
                "You opened version {}; Roblox now has version {}. Saving now replaces their changes with yours.",
                c.opened, c.server
            ));
            if self.cloud.status.as_ref().is_some_and(|(_, s)| s.enabled == Some(true)) {
                ui.label(
                    RichText::new("Team Create is on for this experience. Reopen the live session to merge current changes.")
                        .small()
                        .color(theme::WARN),
                );
            }
            ui.horizontal(|ui| {
                if ui.button("Overwrite").on_hover_text("Save your version on top of theirs").clicked() {
                    pick = Some(0);
                }
                if ui
                    .button(format!("Download version {}", c.server))
                    .on_hover_text("Open Roblox's newer version (your unsaved changes are discarded)")
                    .clicked()
                {
                    pick = Some(1);
                }
                if ui.button("Cancel").clicked() {
                    pick = Some(2);
                }
            });
        });
        match pick {
            Some(0) => {
                self.cloud.conflict = None;
                self.save_cloud(c.vt, true);
            }
            Some(1) => {
                self.cloud.conflict = None;
                if let Some(p) = self.doc.cloud.as_ref().map(|o| o.place_id) {
                    // The person chose to drop their changes.
                    let copy = self.doc.cloud.as_ref().is_some_and(|o| o.local_copy);
                    self.perform_open_cloud(p, Some(c.server), copy);
                }
            }
            Some(_) => self.cloud.conflict = None,
            None => {}
        }
    }

    /// Save to Roblox for a local copy of a Team Create place: only when
    /// the person says so.
    fn copy_save_modal(&mut self, ctx: &egui::Context) {
        let Some(vt) = self.cloud.confirm_copy_save else { return };
        let exp = self.doc.cloud.as_ref().map(|o| o.experience.clone()).unwrap_or_default();
        let mut pick: Option<bool> = None;
        egui::Modal::new(egui::Id::new("cloud_copy_save")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.label(RichText::new("This is a local copy").strong());
            ui.label(format!(
                "Team Create is on for {exp}: others may be editing it live. {} this copy replaces the server copy with yours (Roblox's version is checked first).",
                if vt == VersionType::Published { "Publishing" } else { "Saving" }
            ));
            ui.horizontal(|ui| {
                let label = if vt == VersionType::Published { "Publish to Roblox anyway" } else { "Save to Roblox anyway" };
                if ui.button(label).clicked() {
                    pick = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    pick = Some(false);
                }
            });
        });
        if let Some(go) = pick {
            self.cloud.confirm_copy_save = None;
            if go {
                self.save_cloud(vt, false);
            }
        }
    }

    fn save_error_modal(&mut self, ctx: &egui::Context) {
        let Some(e) = self.cloud.save_error.clone() else { return };
        let mut close = false;
        let mut key = false;
        let mut save_as = false;
        egui::Modal::new(egui::Id::new("cloud_save_error")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.label(RichText::new("Not saved to Roblox").strong());
            ui.label(&e);
            ui.horizontal(|ui| {
                if e.contains("Open Cloud API key") && ui.button("Publishing key…").clicked() {
                    key = true;
                }
                if ui.button("Save to File As…").clicked() {
                    save_as = true;
                }
                if ui.button("OK").clicked() {
                    close = true;
                }
            });
        });
        if close || key || save_as {
            self.cloud.save_error = None;
        }
        if key {
            self.account.show_publish = true;
        }
        if save_as {
            self.save_as();
        }
    }

    /// File > Experience Settings: what the open place is on Roblox.
    fn place_info_window(&mut self, ctx: &egui::Context) {
        if !self.cloud.settings_open {
            return;
        }
        let mut open = true;
        let origin = self.doc.cloud.clone();
        let mut refresh = false;
        egui::Window::new("Experience Settings")
            .open(&mut open)
            .default_width(420.0)
            .resizable(true)
            .show(ctx, |ui| {
                let Some(o) = origin else {
                    ui.label("This place isn't on Roblox: it was opened from a file or made here.");
                    ui.label(
                        RichText::new("Open one with File > Open from Roblox to see its experience, creator and version here.")
                            .color(theme::TEXT_DIM),
                    );
                    return;
                };
                ui.label(RichText::new("Basic Info").strong());
                egui::Grid::new("place_info").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                    let mut row = |k: &str, v: String| {
                        ui.label(RichText::new(k).color(theme::TEXT_DIM));
                        ui.label(v);
                        ui.end_row();
                    };
                    row("Place", o.name.clone());
                    row("Experience", o.experience.clone());
                    row("Creator", format!("{} ({} {})", o.creator_label(), o.creator_type.as_str(), o.creator_id));
                    row("Place ID", o.place_id.to_string());
                    row("Universe ID", o.universe_id.to_string());
                    row(
                        "Start place",
                        if o.root_place_id == o.place_id { "this place".into() } else { o.root_place_id.to_string() },
                    );
                    row(
                        "Opened as",
                        if o.local_copy {
                            "a local copy of the last saved version (Team Create is on; joining isn't supported yet)".into()
                        } else {
                            "the saved version on Roblox".into()
                        },
                    );
                    row("Version", format!("{}{}", o.version, if self.doc.dirty() { " (unsaved changes)" } else { "" }));
                    row("Cached at", self.cloud.cache.dir(o.place_id).display().to_string());
                });
                ui.separator();
                ui.label(RichText::new("Team Create").strong());
                match self.cloud.status.as_ref().filter(|(u, _)| *u == o.universe_id) {
                    Some((_, st)) => {
                        ui.label(st.describe());
                        let yn = |b: Option<bool>| match b {
                            Some(true) => "yes",
                            Some(false) => "no",
                            None => "unknown",
                        };
                        ui.label(
                            RichText::new(format!(
                                "Can collaborate: {}{} · can manage: {} · can edit in the cloud: {}",
                                yn(st.can_collaborate),
                                st.reason.as_ref().map(|r| format!(" ({r})")).unwrap_or_default(),
                                yn(st.can_manage),
                                yn(st.can_cloud_edit)
                            ))
                            .small()
                            .color(theme::TEXT_DIM),
                        );
                    }
                    None => {
                        ui.label(RichText::new("Not checked yet.").color(theme::TEXT_DIM));
                    }
                }
                if ui.add_enabled(!self.cloud.busy(), egui::Button::new("Check again")).clicked() {
                    refresh = true;
                }
            });
        if refresh {
            if let Some(u) = self.doc.cloud.as_ref().map(|o| o.universe_id) {
                let api = self.cloud.factory(self.account.shared_client());
                self.cloud.spawn(api, move |a, tx| {
                    if let Some(a) = a {
                        let _ = tx.send(Msg::Status(u, a.team_create(u)));
                    }
                });
            }
        }
        if !open {
            self.cloud.settings_open = false;
        }
    }

    fn picker_window(&mut self, ctx: &egui::Context) {
        if !self.cloud.open {
            return;
        }
        self.account.restore();
        let mut open = true;
        let mut todo: Vec<PickerDo> = Vec::new();
        egui::Window::new("Open from Roblox")
            .open(&mut open)
            .default_width(600.0)
            .default_height(520.0)
            .resizable(true)
            .show(ctx, |ui| self.picker_contents(ui, &mut todo));
        if !open {
            self.cloud.open = false;
        }
        let client = self.account.shared_client();
        for t in todo {
            match t {
                PickerDo::Tab(tab) => {
                    self.cloud.tab = tab;
                    let empty = match tab {
                        PickerTab::Recent => true,
                        PickerTab::Mine => !self.cloud.lists.contains_key(&ExperienceSource::Mine),
                        PickerTab::Shared => !self.cloud.lists.contains_key(&ExperienceSource::Shared),
                        PickerTab::Groups => self.cloud.groups.is_none(),
                    };
                    if empty {
                        self.cloud.refresh(client.clone(), tab);
                    }
                }
                PickerDo::Refresh => {
                    let tab = self.cloud.tab;
                    self.cloud.refresh(client.clone(), tab);
                    if tab == PickerTab::Groups {
                        self.cloud.lists.retain(|s, _| !matches!(s, ExperienceSource::Group(_)));
                    }
                }
                PickerDo::Group(g) => {
                    self.cloud.group_sel = Some(g);
                    if !self.cloud.lists.contains_key(&ExperienceSource::Group(g)) {
                        self.cloud.load_group(client.clone(), g);
                    }
                }
                PickerDo::Expand(u) => {
                    if !self.cloud.expanded.remove(&u) {
                        self.cloud.expanded.insert(u);
                        if !self.cloud.places.contains_key(&u) {
                            self.cloud.load_places(client.clone(), u);
                        }
                    }
                }
                PickerDo::Icons(ids) => {
                    let api = self.cloud.factory(client.clone());
                    self.cloud.request_icons(api, ids);
                }
                PickerDo::More(src) => {
                    let api = self.cloud.factory(client.clone());
                    self.cloud.load_page(api, src);
                }
                PickerDo::Versions(p, name) => {
                    self.cloud.versions_of = Some((p, name));
                    self.cloud.version_input.clear();
                    if !self.cloud.versions.contains_key(&p) {
                        self.cloud.load_versions(client.clone(), p);
                    }
                }
                PickerDo::Open(p, v) => self.open_cloud(p, v, false),
                PickerDo::Forget(p) => {
                    let _ = self.cloud.cache.forget(p);
                }
                PickerDo::Account => self.account.open = true,
            }
        }
    }

    fn picker_contents(&mut self, ui: &mut egui::Ui, todo: &mut Vec<PickerDo>) {
        let signed = self.account.signed_in();
        let c = &mut self.cloud;
        ui.horizontal(|ui| {
            for t in PickerTab::ALL {
                if ui.selectable_label(c.tab == t, t.label()).clicked() {
                    todo.push(PickerDo::Tab(t));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!c.busy(), egui::Button::new("Refresh")).clicked() {
                    todo.push(PickerDo::Refresh);
                }
                if c.busy() {
                    ui.spinner();
                }
            });
        });
        if !signed {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{}: only places cached on this computer open.", self.account.status_short()))
                        .color(theme::WARN),
                );
                if ui.small_button("Roblox account…").clicked() {
                    todo.push(PickerDo::Account);
                }
            });
        }
        ui.separator();
        let list_h = (ui.available_height() - 150.0).max(160.0);
        egui::ScrollArea::vertical()
            .id_salt("cloud_picker_list")
            .max_height(list_h)
            .auto_shrink([false, false])
            .show(ui, |ui| match c.tab {
                PickerTab::Recent => recent_list(ui, c, todo),
                PickerTab::Mine => experience_list(ui, c, ExperienceSource::Mine, "This account owns no experiences.", todo),
                PickerTab::Shared => experience_list(
                    ui,
                    c,
                    ExperienceSource::Shared,
                    "No experiences are shared with this account (it isn't a collaborator on any).",
                    todo,
                ),
                PickerTab::Groups => match c.groups.clone() {
                    None => {
                        ui.label(RichText::new("Press Refresh to list your groups.").color(theme::TEXT_DIM));
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).color(theme::WARN));
                    }
                    Some(Ok(groups)) if groups.is_empty() => {
                        let msg = if c.busy() {
                            "Loading…"
                        } else {
                            "This account isn't in any group."
                        };
                        ui.label(RichText::new(msg).color(theme::TEXT_DIM));
                    }
                    Some(Ok(groups)) => {
                        let sel = c.group_sel.and_then(|g| groups.iter().find(|x| x.id == g)).cloned();
                        egui::ComboBox::from_id_salt("cloud_group")
                            .selected_text(sel.as_ref().map(|g| g.name.clone()).unwrap_or_else(|| "Group".into()))
                            .show_ui(ui, |ui| {
                                for g in &groups {
                                    if ui
                                        .selectable_label(sel.as_ref().is_some_and(|s| s.id == g.id), format!("{}  ({}{})", g.name, g.role, if g.can_edit == Some(false) { ", can't edit" } else { "" }))
                                        .clicked()
                                    {
                                        todo.push(PickerDo::Group(g.id));
                                    }
                                }
                            });
                        if let Some(g) = sel {
                            // A role without "Edit group experiences": Roblox
                            // refuses its list, so say so first.
                            if g.can_edit == Some(false) {
                                ui.label(
                                    RichText::new(format!(
                                        "Your role in {} is {}: it can't edit the group's experiences. A group owner can give the role the 'Edit group experiences' permission.",
                                        g.name, g.role
                                    ))
                                    .color(theme::WARN),
                                );
                            }
                            let src = ExperienceSource::Group(g.id);
                            if c.lists.contains_key(&src) {
                                experience_list(ui, c, src, "This group has no experiences.", todo);
                            } else {
                                todo.push(PickerDo::Group(g.id));
                            }
                        }
                    }
                },
            });
        ui.separator();
        // A place's versions.
        if let Some((p, name)) = c.versions_of.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Versions of {name}")).strong());
                let cached = c.cache.cached_versions(p);
                if !cached.is_empty() {
                    let list: Vec<String> = cached.iter().map(|v| v.to_string()).collect();
                    ui.label(RichText::new(format!("(cached here: {})", list.join(", "))).small().color(theme::TEXT_DIM));
                }
            });
            match c.versions.get(&p) {
                Some(Ok(vs)) if vs.is_empty() && c.busy() => {
                    ui.spinner();
                }
                Some(Ok(vs)) => {
                    egui::ScrollArea::vertical().id_salt("cloud_versions").max_height(70.0).show(ui, |ui| {
                        for v in vs {
                            ui.horizontal(|ui| {
                                ui.label(format!(
                                    "Version {}  {}  {}{}",
                                    v.version,
                                    v.created.get(..16).unwrap_or(&v.created).replace('T', " "),
                                    v.kind,
                                    if v.published { "  (published)" } else { "" }
                                ));
                                if ui.small_button("Open").clicked() {
                                    todo.push(PickerDo::Open(p, Some(v.version)));
                                }
                            });
                        }
                    });
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).small().color(theme::TEXT_DIM));
                }
                None => {}
            }
            ui.horizontal(|ui| {
                ui.label("Version number:");
                ui.add(egui::TextEdit::singleline(&mut c.version_input).desired_width(70.0));
                let v = c.version_input.trim().parse::<u64>().ok().filter(|v| *v > 0);
                if ui.add_enabled(v.is_some(), egui::Button::new("Open this version")).clicked() {
                    todo.push(PickerDo::Open(p, v));
                }
            });
            ui.separator();
        }
        ui.horizontal(|ui| {
            ui.label("Open by place ID or URL:");
            let r = ui.add(
                egui::TextEdit::singleline(&mut c.url_input)
                    .hint_text("roblox.com/games/… or a place ID")
                    .desired_width(280.0),
            );
            let go = ui.add_enabled(!c.url_input.trim().is_empty(), egui::Button::new("Open")).clicked()
                || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if go {
                match parse_place_ref(&c.url_input) {
                    Ok(p) => {
                        c.url_error = None;
                        todo.push(PickerDo::Open(p.place_id, None));
                    }
                    Err(e) => c.url_error = Some(e),
                }
            }
        });
        if let Some(e) = c.url_error.as_ref().or(c.open_error.as_ref()) {
            ui.label(RichText::new(e).color(theme::WARN));
        }
    }
}

enum PickerDo {
    Tab(PickerTab),
    Refresh,
    Group(u64),
    Expand(u64),
    Icons(Vec<u64>),
    Versions(u64, String),
    Open(u64, Option<u64>),
    Forget(u64),
    Account,
    More(ExperienceSource),
}

const ICON: f32 = 40.0;

fn icon(ui: &mut egui::Ui, c: &CloudUi, universe: u64) {
    match c.icons.get(&universe) {
        Some(t) => {
            ui.add(egui::Image::new((t.id(), egui::vec2(ICON, ICON))));
        }
        None => {
            let (r, _) = ui.allocate_exact_size(egui::vec2(ICON, ICON), egui::Sense::hover());
            ui.painter().rect_filled(r, 2.0, theme::PANEL_ALT);
        }
    }
}

fn date(iso: &str) -> &str {
    iso.get(..10).unwrap_or(iso)
}

fn experience_list(ui: &mut egui::Ui, c: &CloudUi, src: ExperienceSource, empty: &str, todo: &mut Vec<PickerDo>) {
    let paged = c.lists.get(&src);
    let more = paged.is_some_and(|p| p.more);
    let exps = match paged.and_then(|p| p.list.as_ref()) {
        None => {
            ui.label(RichText::new("Press Refresh to list them.").color(theme::TEXT_DIM));
            return;
        }
        Some(Err(e)) => {
            ui.label(RichText::new(e).color(theme::WARN));
            return;
        }
        Some(Ok(v)) => v,
    };
    if exps.is_empty() {
        let msg = if c.busy() { "Loading…" } else { empty };
        ui.label(RichText::new(msg).color(theme::TEXT_DIM));
        return;
    }
    let h_id = ui.id().with("exp_row_h");
    let row_h = ui.data(|d| d.get_temp::<f32>(h_id).unwrap_or(48.0));
    let mut visible: Vec<u64> = Vec::new();
    for e in exps {
        let expanded = c.expanded.contains(&e.id);
        let probe = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), row_h));

        if !expanded && !ui.is_rect_visible(probe) {
            ui.add_space(row_h);
            continue;
        }

        visible.push(e.id);

        let top = ui.cursor().top();

        ui.horizontal(|ui| {
            icon(ui, c, e.id);
            ui.vertical(|ui| {
                ui.label(RichText::new(&e.name).strong());
                ui.label(
                    RichText::new(format!(
                        "by {}{}  ·  universe {}{}{}",
                        if e.creator_type == rustudio_account::CreatorKind::User { "@" } else { "" },
                        e.creator_name,
                        e.id,
                        if e.updated.is_empty() { String::new() } else { format!("  ·  updated {}", date(&e.updated)) },
                        if e.team_create == Some(true) { "  ·  Team Create" } else { "" }
                    ))
                    .small()
                    .color(theme::TEXT_DIM),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let open = c.expanded.contains(&e.id);
                if ui.small_button(if open { "Hide places" } else { "Places" }).clicked() {
                    todo.push(PickerDo::Expand(e.id));
                }
                if let Some(root) = e.root_place_id {
                    if ui.small_button("Open").on_hover_text("Open the start place").clicked() {
                        todo.push(PickerDo::Open(root, None));
                    }
                }
            });
        });
        if c.expanded.contains(&e.id) {
            ui.indent(("places", e.id), |ui| match c.places.get(&e.id) {
                Some(Ok(ps)) if ps.is_empty() => {
                    ui.label(RichText::new("Loading places…").small().color(theme::TEXT_DIM));
                }
                Some(Ok(ps)) => {
                    for p in ps {
                        ui.horizontal(|ui| {
                            ui.label(format!(
                                "{}{}  ({})",
                                if Some(p.id) == e.root_place_id { "★ " } else { "" },
                                p.name,
                                p.id
                            ));
                            if ui.small_button("Open").clicked() {
                                todo.push(PickerDo::Open(p.id, None));
                            }
                            if ui.small_button("Versions…").clicked() {
                                todo.push(PickerDo::Versions(p.id, p.name.clone()));
                            }
                        });
                    }
                }
                Some(Err(err)) => {
                    ui.label(RichText::new(err).small().color(theme::WARN));
                }
                None => {}
            });
        }
        ui.add_space(2.0);

        if !expanded {
            let h = ui.cursor().top() - top - ui.spacing().item_spacing.y;
            ui.data_mut(|d| d.insert_temp(h_id, h));
        }
    }

    if more {
        let probe = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), row_h));
        ui.label(RichText::new("Loading more…").small().color(theme::TEXT_DIM));
        if ui.is_rect_visible(probe) {
            todo.push(PickerDo::More(src));
        }
    }

    visible.retain(|u| !c.icons_asked.contains(u));

    if !visible.is_empty() {
        todo.push(PickerDo::Icons(visible));
    }
}

fn recent_list(ui: &mut egui::Ui, c: &CloudUi, todo: &mut Vec<PickerDo>) {
    let recent: Vec<RecentPlace> = c.cache.recent();
    if recent.is_empty() {
        ui.label(
            RichText::new("No places opened from Roblox yet. Open one from another tab, or by ID or URL below.")
                .color(theme::TEXT_DIM),
        );
        return;
    }
    let missing: Vec<u64> = recent
        .iter()
        .map(|r| r.origin.universe_id)
        .filter(|u| !c.icons_asked.contains(u))
        .collect();
    if !missing.is_empty() {
        todo.push(PickerDo::Icons(missing));
    }
    for r in &recent {
        let o = &r.origin;
        let cached = c.cache.cached_versions(o.place_id);
        ui.horizontal(|ui| {
            icon(ui, c, o.universe_id);
            ui.vertical(|ui| {
                ui.label(RichText::new(&o.name).strong());
                ui.label(
                    RichText::new(format!(
                        "{} by {}  ·  place {}  ·  {}",
                        o.experience,
                        o.creator_label(),
                        o.place_id,
                        match cached.first() {
                            Some(v) => format!("version {v} cached"),
                            None => "not cached".into(),
                        }
                    ))
                    .small()
                    .color(theme::TEXT_DIM),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Remove").on_hover_text("Remove from Recent (the cached copy stays)").clicked() {
                    todo.push(PickerDo::Forget(o.place_id));
                }
                if ui.small_button("Versions…").clicked() {
                    todo.push(PickerDo::Versions(o.place_id, o.name.clone()));
                }
                if ui
                    .small_button("Open")
                    .on_hover_text("Download the latest version (the cached copy when Roblox can't be reached)")
                    .clicked()
                {
                    todo.push(PickerDo::Open(o.place_id, None));
                }
            });
        });
        ui.add_space(2.0);
    }
}
